# PRD 060 — Irrigation Control and Safety

**Milestone:** M6 · **Status:** DELIVERED · **Depends on:** M5

> **Revised 2026-08-26.** M6 now also delivers the **offline evaluator**
> (`rhizo_policy::evaluate_offline`) and **reconciliation** of events buffered
> while a device was isolated ([ADR-015](../adr/015-device-offline-autonomy.md)).
> Issues M6-019…M6-021 were added. Everything about the connected state machine,
> the safety gate, and the command lifecycle is unchanged.
> M2 supplies policy persistence, isolation state, monotonic runtime state, and
> replay mechanics only. M6-019 both implements the single evaluator in
> `rhizo-policy` and extends the existing simulator to call it from exactly one
> place; no simulator-specific decision implementation is permitted.
>
> The reconnection seam is the new safety-critical surface. Two rules carry it:
> replay applies exactly once on `event_id`, and **the edge issues no dose to a
> plant whose reconciliation is incomplete** — a device that autonomously watered
> ninety seconds ago has that dose in its buffer, not yet in the budget
> (SAFETY-016).
>
> There is **one budget per plant**, not one per control path: autonomous doses
> land in the same rolling window as commanded ones (SAFETY-014).
>
> **Additional acceptance criteria:** `cargo test safety_` covers SAFETY-013…020;
> `PROPTEST_CASES=10000 cargo test safety_014` passes; an MQTT spy confirms no
> command is published while a plant is reconciling.

> **Extended 2026-08-28 — commands for sleeping devices.**
> [ADR-018](../adr/018-battery-and-deep-sleep-device-mode.md) §3 adds M6-022 and
> M6-023: a durable **command intent**, held while a battery device sleeps and
> turned into an ordinary command at the next wake.
>
> The safety argument rests on one distinction. **An intent is not a command.** No
> `command_id` exists until delivery, so there is still exactly one
> persist-before-publish moment per command and a delivery retry still reuses the
> `command_id` allocated at that moment — SAFETY-001 and SAFETY-010 are untouched,
> and `commands` gains no column. The full safety gate then **re-runs at
> delivery** against current inputs, which makes this path stricter than the
> immediate one: a leak, an empty tank, an exhausted rolling window, or a stale
> required measurement that appeared while the device slept all refuse the dose
> (SAFETY-003, -004, -005, -006, -012).
>
> Command TTL is **unchanged at 120 s** and needed no change, because the command
> is minted at the wake rather than at the request. The latency lives in
> `intent_expires_at`, an edge-side, operator-visible bound that never reaches a
> device (SAFETY-002 intact). Nothing is retained on MQTT, no topic is added, and
> there is no override, force, expedite, or wake parameter.
>
> **At most one open water intent per plant.** A second request returns 409. The
> rolling cap would bound the total anyway, but arriving at the cap by accident is
> not a design.
>
> **Additional acceptance criteria:** a `POST /water` to a sleeping device
> publishes nothing, verified by a spy subscriber, and returns 202 with
> `pending_for_device_wake` and **no `command_id`**; the delivered command's
> `issued_at` is the wake instant, not the request instant; `edge.time` precedes
> the command on every wake delivery; an edge restart between request and wake
> still delivers exactly once; an M8 mutation that publishes immediately to a
> sleeping device turns the suite red.
>
> **Open question.** Cancelling a pending intent from the API is deliberately not
> specified: a cancel that races a wake is a consensus problem for a feature
> nobody has asked for, and `intent_expires_at` already bounds the exposure.

## Summary

The milestone where the system gains the ability to move water. Implements the
irrigation state machine, the safety gate, the command lifecycle, and every
SAFETY invariant that does not require hardware. This is the most
safety-critical PRD in the project.

## Problem

Everything before M6 is observation. From M6 the software can operate a pump,
and the consequences of a defect change from "a wrong number on a screen" to
"water on the floor" or "a drowned plant". The design must make the dangerous
outcomes structurally difficult rather than merely tested against.

## Goals

1. The irrigation state machine as a pure, total function.
2. A safety gate that runs first, always, with no catch-all arm.
3. Command lifecycle: persist → publish → result → settle, crash-safe at every
   step.
4. Manual, recommended, and automatic modes with explicitly different privileges.
5. Every non-hardware SAFETY invariant enforced and tested.
6. Lockout management with explicit and automatic clearing rules.
7. Activate offline autonomy by implementing the shared evaluator and wiring
   the M2 simulator to that implementation without a second decision path.

## Non-goals

- Real hardware (M11). M6 is validated entirely against the simulator, which
  refuses exactly as firmware does
  ([ADR-008](../adr/008-shared-code-simulator-and-firmware.md)).
- Cloud (M7). The cloud is not an input to any decision here.
- UI (M12). The API is the interface; safety must hold regardless of client.

## User/system flows

**Automatic cycle:**

```text
tick → load irrigation_state from SQLite → gather inputs
     → evaluate() → IssueDose(40 ml)
     → [TX: insert command 'issued', state→DoseIssued, outbox] → COMMIT
     → publish MQTT (QoS 1)
     → device validates → pump → command.result(completed, 40 ml)
     → [TX: command→completed, watering_event, state→WaitForAbsorption] → COMMIT
     → 15 min later → Recheck
        recovered → Normal   |   still dry & doses<max → DryConfirmed
                             |   still dry & doses=max → Lock(MaxDosesReached)
```

**Manual watering, refused:**

```text
POST /plants/monstera-01/water {"ml":30,"mode":"manual"}
   → safety gate → leak detected
   → 409 Conflict { "error": { "code":"safety_lockout",
                               "details": { "reason":"leak" } } }
   → no command persisted, nothing published
```

## Functional requirements

### The safety gate

| ID | Requirement |
|---|---|
| F-060-01 | The gate runs **before** any irrigation logic, in `evaluate`, at the single entry point |
| F-060-02 | Resolve the plant's `SensorBinding[]`, optional `ActuatorBinding`, and `MeasurementPolicy[]`; then run ordered veto checks for required leak/tank/control inputs, validity, freshness, and durable rolling budget |
| F-060-03 | Every safety match is **exhaustive with no `_ =>` arm**; a new enum variant fails to compile until classified |
| F-060-04 | `None` and `Unknown` map to a lockout, never to permission |
| F-060-05 | `manual` mode skips **only** `SensorFault` and `StaleData`; every other check applies |
| F-060-06 | There is no override parameter, force flag, or debug bypass on any endpoint |

### State machine

| ID | Requirement |
|---|---|
| F-060-10 | `evaluate` is pure: no I/O, no clock access, no mutation |
| F-060-11 | The function is **total** — every (state, input) pair yields a defined decision |
| F-060-12 | `Locked` is reachable from every state on every tick |
| F-060-13 | State is loaded from SQLite each tick, never from memory alone |
| F-060-14 | Transitions persist in one transaction with their side effects |

### Command lifecycle

| ID | Requirement |
|---|---|
| F-060-20 | The command row is committed with status `issued` **before** the MQTT publish |
| F-060-21 | `command_id` is the primary key; a duplicate insert fails at the storage layer |
| F-060-22 | `expires_at = issued_at + profile.command_ttl` (default 120 s) |
| F-060-23 | Publish retried at most 3× **with the same `command_id`**; never a new one |
| F-060-24 | After 3 failures the command is `failed`, state → `Recheck`, and **no watering event is created** |
| F-060-25 | A result for a command already in a terminal state is ignored |
| F-060-26 | `interrupted` and `failed` results credit `requested_ml` conservatively to the daily total |
| F-060-27 | On boot, `issued`/`in_flight` commands are reconciled: expired → `expired` + `Recheck`; live → await until `expires_at` |
| F-060-28 | A command is **never re-published** after a restart |

### Multi-dose cycle

| ID | Requirement |
|---|---|
| F-060-30 | Up to `max_doses_per_cycle` doses of `dose_ml` per cycle |
| F-060-31 | `absorption_wait_minutes` between doses, enforced by `wait_until` persisted in SQLite |
| F-060-32 | Recovery judged by moisture rising `recovery_delta_vwc` above the pre-dose reading |
| F-060-33 | No moisture and no weight response after two consecutive doses → `Lock(NoDeliveryDetected)` |
| F-060-34 | `cooldown_hours` enforced between completed cycles |

### Lockouts

| ID | Requirement |
|---|---|
| F-060-40 | Auto-clearing lockouts: `StaleData`, `SensorFault`, `TankLow`, `DailyLimit`, `Uncertain` — when the condition demonstrably resolves |
| F-060-41 | Explicit-clear lockouts: `Leak`, `PumpFault`, `NoDeliveryDetected`, `MaxDosesReached` |
| F-060-42 | `POST /plants/{id}/lockout/clear` returns 409 if the condition is still active |
| F-060-43 | Lockout reason, since-timestamp, and clearability exposed in the API |

**As implemented (2026-10-22, [ADR-021](../adr/021-independent-plant-lockouts.md)).**
The four requirements above hold, on a model the original text did not spell
out:

- **F-060-40.** These reasons are *derived*: re-evaluated from the inputs on
  every pass and **never stored**, so "clears when the condition resolves" means
  the next evaluation no longer derives it. Two `Uncertain`/`ClockUnsynced`
  sources are not derived but **holds** — the forward clock step (F-060-51) and a
  device's `clock_unsynced` refusal — stored with a deadline and ending only when
  it passes, never by a request.
- **F-060-41.** Each explicit-clear reason is a **latch**: one
  `plant_lockouts` row per `(plant, reason)`, written beside any other record and
  never replaced. Several can be active at once; each is cleared separately. An
  unrecognised stored reason (`unknown`, or a name carried over by migration
  `0002`) is also a latch, cleared by its exact name. There is no automatic
  clear.
- **F-060-42.** `lockout/clear` names one latch and removes only it, inside a
  `BEGIN IMMEDIATE` transaction that, for `leak`, re-reads the bound leak stream
  and requires a fresh, positively dry reading (409 `safety_lockout` with
  `leak_signal` otherwise). A reason that is not a recorded latch is 409
  `not_locked_out` with the current `lockouts`; a hold is 409
  `hold_not_clearable`. Success answers `cleared` and `remaining[]`.
- **F-060-43.** `GET /plants/{id}`, `GET /plants`, and `GET /overview` list
  every latch and active hold as `lockouts[]` (`reason`, `kind`, `since`,
  `hold_until`, `operator_clearable`, `recognised`) in a fixed display order;
  `lockout` remains the one-reason summary. The existing `lockout.clearable`
  field means *clears by itself* there and *needs an operator* in `POST /water`'s
  409 body — a known inconsistency, left for a decision (M9 report §15).
- Additionally, `command_repo::issue` refuses a command for a plant with any
  record, and `authorize_publish` re-checks before **every** publish attempt
  (`failed` / `blocked_by_lockout`). A lockout recorded after that last check
  does not recall a message already handed to MQTT (ADR-021 §Risks).

### Clock handling

| ID | Requirement |
|---|---|
| F-060-50 | Rolling 24-hour window computed by summing `watering_events`, never a counter |
| F-060-51 | Forward clock step > 10 min → all plants `Lock(Uncertain)` for one cooldown |

**F-060-51 as implemented.** The step records a `hold("uncertain", now +
cooldown_hours)` for every plant *beside* its existing records: a latched leak is
neither replaced nor lifted by the hold, the hold's expiry takes nothing else
with it, and an operator clearing the leak inside the cooldown leaves the hold
refusing until its deadline (`safety_003_a_clock_step_hold_expiring_leaves_the_leak`,
`safety_003_clearing_the_leak_inside_a_clock_hold_leaves_the_hold`).
| F-060-52 | Backward step logged; the window naturally becomes more conservative |

### Offline evaluator activation

| ID | Requirement |
|---|---|
| F-060-60 | `rhizo_policy::evaluate_offline` is the single implementation of restricted offline decisions |
| F-060-61 | M6-019 installs exactly one simulator call site using the persistence/isolation seam prepared by M2-017 |
| F-060-62 | The simulator contains no duplicated policy rules; firmware later calls the same evaluator in M9-016 |
| F-060-63 | Autonomous `Dose` decisions route through the simulator's existing single actuation/validator path |
| F-060-64 | Missing, stale, unknown, invalid, or disabled policy inputs fail closed before autonomous scheduling |

## Interfaces

```rust
pub struct IrrigationInputs<'a> {
    pub now: DateTime<Utc>,
    pub state: &'a IrrigationState,
    pub mode: EvaluationMode,          // Automatic | ManualRequest { ml }
    pub latest_soil: Option<&'a SoilSample>,
    pub pre_dose_soil: Option<&'a SoilSample>,
    pub latest_weight: Option<&'a WeightSample>,
    pub tank: Option<TankState>,
    pub leak: LeakState,               // Clear | Detected | Unknown
    pub sensor_bindings: &'a [SensorBinding],
    pub actuator_binding: Option<&'a ActuatorBinding>,
    pub measurement_policies: &'a [MeasurementPolicy],
    pub automation: &'a AutomationPolicy,
    pub delivered_last_24h_ml: f32,
    pub doses_this_cycle: u8,
    pub last_cycle_completed_at: Option<DateTime<Utc>>,
    pub wait_until: Option<DateTime<Utc>>,
    pub auto_watering_enabled: bool,
    pub device_online: bool,
}

pub enum IrrigationDecision {
    Idle,
    Recommend { ml: f32, reasons: Vec<Reason> },
    IssueDose { ml: f32, reasons: Vec<Reason> },
    Wait       { until: DateTime<Utc> },
    Lock       { reason: LockoutReason },
    CycleComplete,
}

pub fn evaluate(inputs: IrrigationInputs<'_>) -> IrrigationDecision;
```

REST additions:

```text
POST /api/v1/plants/{id}/water                 { "ml": 30, "mode": "manual" }
POST /api/v1/plants/{id}/auto-watering/enable
POST /api/v1/plants/{id}/auto-watering/disable
POST /api/v1/plants/{id}/lockout/clear         { "reason": "leak" }
GET  /api/v1/commands/{command_id}
PUT  /api/v1/devices/{id}/config
POST /api/v1/devices/{id}/commands/tare
POST /api/v1/devices/{id}/commands/calibrate
```

## Data model

Uses `commands`, `watering_events`, and `irrigation_state` from
[ADR-004](../adr/004-sqlite-edge-persistence-model.md).

**M6's schema change adds no column to `commands`.** What it adds is the durable
intent a sleeping device's dose waits in, the pre-dose baselines recovery and
no-delivery detection compare against, and the lockout audit fields SAFETY-003
needs:

```text
command_intents   intent_id, plant_id, device_id, kind, requested_ml, mode,
                  created_at, intent_expires_at, expected_delivery_after,
                  state, command_id (NULL until delivery), refusal_reason,
                  settled_at, updated_at
                  UNIQUE(plant_id) WHERE state='pending_for_device_wake'
                                     AND kind='water'

plants            + lockout_cleared_by, lockout_cleared_at, lockout_until
irrigation_state  + pre_dose_vwc, pre_dose_grams
```

Since migration `0002` ([ADR-021](../adr/021-independent-plant-lockouts.md)) the
lockout state itself is `plant_lockouts (plant_id, reason, kind, since,
hold_until)`, one row per independent latch or hold; the `plants.lockout_*`
columns are a display summary that no decision reads.

`command_intents.command_id` being **nullable** is the reviewer's check that an
intent is not a command: no `command_id` exists until the wake mints one, so
there is still exactly one persist-before-publish moment per command and a
delivery retry still reuses the id allocated at that moment.

### Intent lifecycle

```text
pending_for_device_wake ──delivery──▶ sent            (command_id assigned)
                        ──gate──────▶ refused         (refusal_reason recorded)
                        ──sweep─────▶ expired_before_wake
```

Terminal states are terminal. The full safety gate re-runs **at delivery**
against current inputs, which makes this path stricter than the immediate one;
`edge.time` is published before the command on every wake delivery, and a
`clock_unsynced` refusal inside one awake window is retryable rather than
terminal.

The rolling-window query, which is the SAFETY-006 mechanism:

```sql
SELECT COALESCE(SUM(delivered_ml), 0)
FROM watering_events
WHERE plant_id = ?1
  AND completed_at > ?2                     -- now_ms - 86400000
  AND mode IN ('automatic', 'recommended');
```

Derived from rows rather than a counter, so a restart cannot reset it.

## State model — normative transition table

| From | Condition | To | Side effect |
|---|---|---|---|
| any | gate returns a reason | `Locked(r)` | latch `r` if it is latch-class and not yet recorded; update the summary; event, metric |
| `Locked(r)` | r derived and no longer derived, no record remains | `Normal` | summary cleared, event |
| `Locked(r)` | a hold's deadline passes, no record remains | `Normal` | summary cleared, event |
| `Locked(r)` | operator clears latch `r`, no record remains | `Normal` | `lockout_cleared` event, outbox |
| `Normal` | moisture < `target_min` | `Drying` | — |
| `Drying` | moisture ≥ `target_min` | `Normal` | reset dry duration |
| `Drying` | dry ≥ `dry_confirm_minutes` | `DryConfirmed` | — |
| `DryConfirmed` | auto disabled | `DryConfirmed` | emit recommendation only |
| `DryConfirmed` | auto enabled, cooldown elapsed, gate passes | `DoseIssued` | persist + publish command |
| `DoseIssued` | result `completed` | `WaitForAbsorption` | watering_event, `wait_until` |
| `DoseIssued` | result `rejected` | `Recheck` | record reason, no event |
| `DoseIssued` | result `interrupted`/`failed` | `Recheck` | credit `requested_ml`, no event |
| `DoseIssued` | `expires_at` passed, no result | `Recheck` | command → `expired` |
| `WaitForAbsorption` | `now < wait_until` | `WaitForAbsorption` | — |
| `WaitForAbsorption` | `now ≥ wait_until` | `Recheck` | — |
| `Recheck` | moisture ≥ pre-dose + `recovery_delta` | `Normal` | `CycleComplete`, set `last_cycle_completed_at` |
| `Recheck` | still dry, `doses < max` | `DryConfirmed` | — |
| `Recheck` | still dry, `doses = max` | `Locked(MaxDosesReached)` | alert |
| `Recheck` | 2 doses, no moisture and no weight response | `Locked(NoDeliveryDetected)` | alert |

**Totality:** every state has a defined outcome for every input, including
absent inputs — which resolve to `Locked(Uncertain)` via the gate. This is
asserted by `prop_state_machine_total`.

## Failure modes

| Failure | Behaviour |
|---|---|
| Crash between commit and publish | recovery finds `issued` with no result; awaits or expires it. **Never re-publishes.** |
| Crash between publish and result | same; a late result matches the existing `command_id` |
| Crash during `WaitForAbsorption` | state and `wait_until` restored from SQLite; cycle continues |
| Result arrives for an unknown `command_id` | logged, ignored — no watering event invented |
| Duplicate result | dedup at M3's layer plus a terminal-status check |
| **Crash between receiving a result and committing it** | **Currently loses the result: `rumqttc` PUBACKs before the pipeline commits, and protocol section 5.10 stops the device retrying at the PUBACK. Must be closed before M6 enables watering ([M6-010](../issues/M6/010-implement-command-result-handling.md)).** |
| Device offline when a dose is due | no command issued; `device_online: false` is a gate input |
| MQTT publish fails 3× | command `failed`, state `Recheck`, no event |
| Leak asserted mid-dose | device stops; result reports partial delivery; edge locks out |
| Clock steps forward mid-cycle | `Lock(Uncertain)` for one cooldown |
| Profile edited mid-cycle | the next tick uses the new profile; an in-flight command keeps its own TTL |

## Safety implications

M6 is where nine invariants become enforced. Each maps to a requirement and a test:

| Invariant | Mechanism | Test |
|---|---|---|
| SAFETY-001 | `command_id` PK + device dedup ring | `safety_001_duplicate_command_single_actuation` |
| SAFETY-002 | device TTL check via shared validator | `safety_002_expired_command_rejected` |
| SAFETY-003 | gate check 1, applies to manual too | `safety_003_leak_blocks_manual_api` |
| SAFETY-004 | gate check 2, `None` → lockout | `safety_004_tank_unknown_or_low_blocks` |
| SAFETY-005 | gate checks 3–4 using `received_at` | `safety_005_stale_or_invalid_blocks_auto` |
| SAFETY-006 | rolling window from rows | `safety_006_rolling_24h_cap_never_exceeded` |
| SAFETY-007 | shared validator clamps | `safety_007_clamp_never_exceeds_hard_max` |
| SAFETY-010 | persist-before-publish + reconciliation | `safety_010_restart_mid_command_no_replay` |
| SAFETY-012 | exhaustive gate, `Option` inputs | `safety_012_missing_input_never_waters` |

Three requirements carry disproportionate weight:

- **F-060-20** (persist before publish). The reverse order permits a pump to run
  with no record it was asked to.
- **F-060-23** (retry the same `command_id`, never a new one). Issuing a fresh
  command after an ambiguous publish failure is the most plausible route to
  duplicate watering in the entire design
  ([ADR-014](../adr/014-failure-and-retry-policy.md)).
- **F-060-03** (no catch-all arm). It converts "we thought of every case" from a
  claim into a compiler-checked property.

## Observability

Metrics:

```text
watering_commands_total{mode,outcome}    watering_delivered_ml_total{mode}
watering_failures_total{reason}          irrigation_state_transitions_total{from,to}
plants_locked_out                        lockouts_total{reason}
control_tick_duration_seconds
```

Every state transition is persisted, so "what did the system think, and when"
is reconstructable months later — the question actually asked when a plant dies.

Logging: INFO for every dose issued, every result, every lockout set or cleared.
These are world-changing events and should be visible in a log skimmed after a
two-week absence.

## Testing strategy

The heaviest test investment in the project.

- **Property tests** — the flagship layer. All ten listed in
  [strategy.md](../testing/strategy.md) §4, especially
  `safety_006_rolling_24h_cap_never_exceeded` with adversarial histories
  (restarts between publish and result, clock steps, interrupted doses).
- **Unit** — every row of the transition table, including illegal transitions;
  each gate reason in isolation; manual-mode privilege differences; TTL
  arithmetic; publish-retry semantics.
- **Integration** — SCEN-011, -030, -032, -033, -035, -040, -041, -042, -043,
  -051, -052, -054, -070, -071, -072.
- **Regression corpus** — `proptest-regressions/` committed; every shrunk
  counterexample is permanent evidence.

## Acceptance criteria

- [x] `cargo test safety_` passes with every SAFETY-001…007, 010, 012 test green.
- [x] The full cycle scenario (SCEN-002) produces the exact documented state
      sequence and never exceeds `max_daily_ml`.
- [x] Publishing the same command three times causes one actuation and one
      `watering_event`.
- [x] `POST /water` during a leak returns **409**, and no MQTT message is
      published.
- [x] Clearing a leak lockout while the leak is active returns 409.
- [x] Killing the edge after publish and restarting produces no second command
      and exactly one `watering_event`.
- [x] A plant with no tank sensor never receives an automatic dose.
- [x] A plant with a stale sample never receives an automatic dose, but **can**
      be watered manually.
- [x] `evaluate` has no `_ =>` arm on any safety match (reviewed and asserted by
      `safety_012_no_catch_all_arm_on_a_safety_match`, which reads the gate's own
      source; a `trybuild` compile-fail case was not added — see
      [docs/reports/M6.md](../reports/M6.md) §Deviations).
- [x] `PROPTEST_CASES=10000 cargo test safety_006` passes.
- [x] The shared offline evaluator, its sole simulator call site, reconciliation,
      and SAFETY-013…020 property coverage required by M6-019…M6-021 are green.
- [x] No MQTT water command is published while reconciliation is incomplete.
- [x] A `command.result` acknowledged to a device survives an edge crash between
      receipt and commit — the device's retry stops on the edge's durable
      commit, not on the broker PUBACK. Carried forward from the M3 audit; a
      lost delivered dose under-counts the SAFETY-006 budget. Closed by manual
      acknowledgement on the ingress: the PUBACK is sent by the pipeline after
      its transaction commits.

## Dependencies

- M5 (plants, profiles, recommendations, trends).
- M2 (a device that refuses like hardware — without it, SAFETY-007 and
  SAFETY-002 cannot be tested here at all).

## Open questions

1. **Whether `MaxDosesReached` should auto-clear after the cooldown** rather than
   requiring an explicit clear. Requiring explicit clearing is chosen because
   reaching the dose limit means the model of the plant is wrong, and repeating
   the cycle would repeat the mistake. Revisit after M10 with real-plant data.
2. **Crediting `requested_ml` for an interrupted dose** may over-count when the
   interruption happened early. Chosen deliberately: over-counting reduces the
   next dose, under-counting could permit an extra one. The conservative
   direction is the safe one.
3. **Default `command_ttl` of 120 s** — long enough for a healthy LAN, short
   enough that a reconnecting device sees only stale commands. Tunable per
   profile.

## Future work

- Weight-derived delivery verification once a scale is fitted (M9+).
- Per-plant learned absorption time (post-V1).
- Zone-based irrigation with valves ([PRD 140](140-field-readiness.md)).
