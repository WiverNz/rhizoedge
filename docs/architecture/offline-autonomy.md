# Offline Autonomy

How a plant-side device keeps a plant alive when it cannot reach the Edge
Controller — without improvising, and without weakening any safety rule.

Decision record: [ADR-015](../adr/015-device-offline-autonomy.md).
Connectivity definitions: [connectivity-modes.md](connectivity-modes.md).

---

## 1. The rule that makes this safe

> **A device may act autonomously only from a policy the Edge Controller
> previously validated, versioned, and the device persisted and activated
> atomically.**

The device never invents a rule, never falls back to a default threshold, and
never "does something sensible" because it lacks configuration. An unprovisioned
device in mode C is a data logger, and that is the correct behaviour.

The corollary matters as much: **absence of a policy is not permission**. No
policy, an unparseable policy, or a policy whose required sensors are missing all
resolve to *do not actuate* (SAFETY-013).

## 2. Ownership: what moved, what did not

Earlier versions of this plan said the device contains no irrigation
intelligence. That is now too strong. The accurate statement is:

| Concern | Owner |
|---|---|
| Recommendations, trends, explanations | **Edge only** |
| Manual-watering detection, EC correlation | **Edge only** |
| Plant profiles, policy authoring and validation | **Edge only** |
| Cross-plant and reservoir-wide reasoning | **Edge only** |
| Connected-mode watering decisions | **Edge** |
| **Offline bounded watering from a persisted policy** | **Device** |
| Final hardware safety veto | **Device, always** |

The device gained a deliberately restricted evaluator. It did not gain the Edge
Controller.

### What the offline evaluator may do

```text
threshold comparison  +  confirmation duration  +  hysteresis
+ cooldown  +  bounded dose  +  absorption wait  +  bounded dose count
+ rolling volume cap  +  required-sensor and staleness checks
+ the full safety gate
```

### What it may never do

```text
trend fitting          recommendation generation      confidence scoring
manual-watering detection                             profile editing
cross-plant reasoning  reservoir arbitration          policy authoring
unbounded dosing       dose sizing from a computation rather than the policy
```

If a rule needs history longer than the device's buffer, or data from another
device, it belongs to the Edge and is simply unavailable in mode C.

## 3. The offline policy

One policy per plant, delivered to the device that carries that plant's
actuator. A device serving several plants holds several policies.

```text
OfflinePolicy
├── policy_version        u32, edge-owned, monotonic
├── plant_id
├── enabled               bool — offline automation opt-in, default false
│
├── actuator
│   ├── actuator_id       must match a declared device capability
│   ├── dose_ml           the ONLY dose the device may deliver
│   ├── max_doses_per_cycle
│   └── absorption_wait   duration
│
├── control_measurement
│   ├── kind              e.g. soil_moisture
│   ├── point
│   ├── trigger_below     start a cycle below this
│   ├── resume_above      hysteresis: cycle ends at/above this
│   ├── confirm_duration  continuous time below trigger before acting
│   └── max_age           staleness limit
│
├── required_measurements []   absent or stale ⇒ no actuation
├── advisory_measurements[]    recorded, never gate actuation
│
├── limits
│   ├── cooldown              between completed cycles
│   ├── max_volume_per_window
│   └── window                rolling, default 24 h
│
└── safety
    ├── require_leak_clear    default true
    ├── require_tank_above    percent, or "required but unknown ⇒ refuse"
    └── require_pump_healthy  default true
```

Design points that are not arbitrary:

- **`dose_ml` is a value, not a formula.** The device delivers exactly the dose
  the Edge authored, up to `max_doses_per_cycle` times. Computing a dose on the
  device would be the beginning of reimplementing the recommendation engine.
- **`enabled` defaults to false.** Offline autonomy is opted into per plant, by a
  human, exactly like `auto_watering_enabled` (SAFETY-012 applied to
  provisioning).
- **`required_measurements` is explicit.** The device does not guess which
  sensors matter. A plant whose policy requires tank level will not water when
  the tank sensor is silent, and a plant whose policy does not require pot
  weight is unaffected by a broken scale (SAFETY-017).
- **`resume_above` is separate from `trigger_below`.** Without hysteresis a
  sensor sitting on the threshold produces a dose per evaluation tick.

## 4. Evaluation

The offline evaluator is a **pure function**, in the shared `rhizo-policy`
crate, called identically by the firmware, the simulator, and the Edge:

```rust
pub fn evaluate_offline(
    policy:  &OfflinePolicy,
    state:   &OfflineState,     // persisted: cycle, doses, budget, deadlines
    inputs:  &OfflineInputs,    // latest samples + ages, leak, tank, pump health
    elapsed: MonotonicMillis,   // since boot; never a wall clock
) -> OfflineDecision;

pub enum OfflineDecision {
    Idle,
    Confirming { remaining: Duration },
    Dose { ml: f32 },
    WaitAbsorption { remaining: Duration },
    Cooldown { remaining: Duration },
    Refuse(RefuseReason),
}
```

Purity buys the same thing it buys in `rhizo-domain`: the whole offline safety
surface is property-testable in microseconds, with no board, no broker, and no
soil.

The Edge links the same crate, which is what lets it **predict** what an
isolated device will do and **verify** a policy before publishing it. A policy
the Edge cannot evaluate is a policy it must not send.

### The gate runs first, always

```text
1. enabled == false                       → Refuse(AutomationDisabled)
2. policy invalid / version unknown       → Refuse(NoValidPolicy)
3. leak detected or unknown               → Refuse(Leak | LeakUnknown)
4. tank below minimum or unknown          → Refuse(TankLow | TankUnknown)
5. pump faulted                           → Refuse(PumpFaulted)
6. a required measurement missing/stale   → Refuse(RequiredMeasurementUnavailable)
7. control measurement invalid            → Refuse(SensorFault)
8. rolling budget exhausted               → Refuse(BudgetExhausted)
9. cooldown active                        → Cooldown
10. clock unusable for durations          → Refuse(TimebaseUnusable)
      ── only then is irrigation logic evaluated ──
```

Exhaustive matches, no catch-all arm, `Option`/tri-state for every absent-able
input — the same discipline as the Edge's gate
([ADR-006](../adr/006-irrigation-state-machine-ownership.md)).

## 5. Persisted offline state

NVS on the device, mirrored in the simulator's state file so restart behaviour is
comparable:

```text
policy_blob        + policy_version + CRC     the active policy
policy_staging     + CRC                      candidate during update (§7)
cycle_state                                   Idle|Confirming|Dosing|Absorbing|Cooldown
confirm_elapsed_ms                            monotonic accumulation
doses_this_cycle
budget_used_ml     + window_started_monotonic
cooldown_remaining_ms                         persisted as REMAINING, not as a deadline
in_flight_dose                                written before actuation
event_buffer                                  §6
last_reconciled_seq
```

`cooldown_remaining_ms` is stored as a remaining duration rather than an absolute
deadline precisely because the device may have no absolute time. On boot the
remaining duration is restored intact, so a reboot cannot shorten a cooldown.

Likewise `budget_used_ml` is **not** cleared on boot. It is reduced only when the
device can demonstrate that the window elapsed — either from monotonic time it
actually observed, or from a wall clock it trusts. A device that reboots
repeatedly does not thereby earn more water (SAFETY-014, SAFETY-015).

## 6. Event buffering while isolated

An ESP32 cannot retain unbounded history, and pretending otherwise would be a
design that fails quietly in the field.

**Bounded ring buffer in NVS, with tiered retention:**

| Tier | Kinds | Capacity | Overflow |
|---|---|---|---|
| **audit** | autonomous dose, refuse-with-reason, lockout set/cleared, policy activation, pump fault, leak | 64 events, never evicted by telemetry | oldest audit event evicted, `gap` recorded |
| **telemetry** | measurement samples | remaining space, target ~256 samples | oldest evicted silently |

Every event carries a device-generated `event_id` (UUID) and a monotonically
increasing `device_seq` scoped by `boot_id`. When eviction occurs the device
records a **gap marker**: the `device_seq` range lost and how many events. On
reconnect the gap is reported explicitly.

> A gap is data. It is reported, stored, and visible in the plant's history —
> never silently absorbed (SAFETY-020).

Audit events outrank telemetry because the record of what the machine did to a
living plant is not optional, while a missing moisture sample is a missing pixel
in a chart. This mirrors the edge outbox's value tiers
([ADR-014](../adr/014-failure-and-retry-policy.md)).

## 7. Policy delivery and activation

The dangerous failure is a half-written policy taking effect. The sequence is
therefore **validate → stage → activate → acknowledge**, never
*receive → use → discover it was wrong*:

```text
edge authors policy, validates it with rhizo-policy, bumps policy_version
        ↓  retained publish on rhizo/v1/devices/{id}/policy
device receives
        ↓
1. parse; on failure → keep active policy, report rejection, STOP
2. validate against declared capabilities and firmware hard limits
        ↓  on failure → keep active policy, report rejection, STOP
3. write to policy_staging with CRC
4. verify staging read-back
5. atomic activate: flip the active pointer
6. persist; report applied_policy_version in status
```

Guarantees:

- **A bad policy never replaces a good one.** Steps 1–4 are non-destructive.
- **Power loss at any step leaves exactly one valid policy active** — the old one
  before step 5, the new one after (SAFETY-019).
- **`policy_version` ≤ the applied version is ignored**, so a retained
  republication after a rollback cannot silently regress the device.
- **Hard limits win.** A policy asking for a dose above
  `FIRMWARE_MAX_ML_PER_RUN` is *rejected at step 2*, not clamped at actuation
  time — the operator learns the real limit while editing
  ([ADR-011](../adr/011-configuration-and-secrets-model.md)).

The Edge surfaces `desired_policy_version` vs `applied_policy_version` as drift,
exactly as it already does for device config.

## 8. Reconnection and reconciliation

The transition out of mode C is where duplicate watering would be created if the
design were careless.

```text
device reconnects
   ↓ publishes retained status incl. applied_policy_version, boot_id, device_seq
   ↓ replays buffered events in device_seq order, QoS 1, in batches
edge ingests each event
   ↓ dedup on event_id  (the same processed_messages mechanism as telemetry)
   ↓ autonomous doses become watering_events with origin=offline_autonomous
   ↓ the rolling budget is recomputed from rows — offline doses now count
   ↓ gap markers stored as device_events(kind='history_gap')
   ↓ plant leaves Uncertain only after the replay is acknowledged complete
edge resumes normal control
```

Three properties, each with an invariant:

1. **Exactly once.** Replay is idempotent on `event_id`; a device that
   reconnects, disconnects, and reconnects mid-replay creates no duplicates
   (SAFETY-016).
2. **No double dosing across the seam.** The edge does not issue a dose to a
   plant whose reconciliation is incomplete. An autonomous dose delivered ninety
   seconds ago is in the buffer, not yet in the budget — issuing on top of it is
   exactly the failure this rule prevents (SAFETY-016).
3. **Budget continuity.** Offline doses count toward the same rolling window as
   commanded doses. There is one budget per plant, not one per control path
   (SAFETY-014).

The device retains replayed events until the edge acknowledges them, so an edge
crash mid-reconciliation loses nothing — it simply replays again.

## 9. What this costs

Stated plainly, because it is real:

- **Two evaluators exist.** The Edge's rich engine and the device's restricted
  one can disagree about whether a plant needs water. Mitigated by sharing
  `rhizo-policy` so the *offline* rules have one implementation, and by the Edge
  linking that crate so it can predict device behaviour. Not eliminated.
- **Firmware complexity grows.** NVS state, an event ring, atomic activation, and
  a monotonic budget are genuinely more code in the place hardest to debug.
- **A window of divergence exists at reconnection.** Bounded by §8's rules, not
  removed.
- **Bounded history means gaps are possible.** Made visible rather than hidden.

The alternative — a plant that dies because the router rebooted while its owner
was away — is worse. But this is a considered trade, not a free feature.

## 10. Where the code lives

| Crate | Contents | `no_std` |
|---|---|---|
| `rhizo-mqtt-contract` | wire types, capabilities, policy payload, hard limits, `validate_water_command` | ✅ |
| **`rhizo-policy`** *(new)* | `OfflinePolicy`, `evaluate_offline`, offline state machine, budget accounting | ✅ |
| `rhizo-domain` | recommendations, connected irrigation machine, plant model; **links `rhizo-policy`** to validate and predict | ❌ std |
| `esp32-node` | adapters, NVS, event ring; calls `evaluate_offline` from exactly one place | — |
| `device-simulator` | same call site, same crate | — |

`rhizo-policy` sits between the contract and the domain:
`mqtt-contract ← policy ← domain`. See
[ADR-001](../adr/001-rust-workspace-and-crate-boundaries.md).
