# ADR-006 — Irrigation state machine and safety ownership

## Status

Accepted — 2026-08-25. Implemented in M6.

## Context

The project plan is explicit that automatic watering must never be:

```python
if moisture < threshold:
    pump_on()
```

The reason is not aesthetic. A threshold check has no memory, so it cannot
enforce a cooldown, cannot count doses, cannot wait for absorption, and cannot
survive a restart with its intent intact. It also produces the worst failure
mode in this domain: a sensor reading low because it is *out of the soil* causes
continuous pumping.

We need to decide where the state machine lives, who evaluates safety, how state
survives a restart, and how a decision becomes explainable.

## Decision

### The Edge owns decisions. The device owns the veto.

```text
Edge Controller                          Device
───────────────                          ──────
knows history, trends, profiles,         knows nothing about plants
daily totals, cooldowns                  knows its own hard limits
                                         knows its own sensors right now
decides WHETHER and HOW MUCH        →    decides whether to OBEY
```

Neither side can cause watering alone. The edge cannot actuate; the device will
not act without a command. This is defence in depth, and it is why SAFETY-007
can be trusted even if the edge is completely wrong.

### The state machine is a pure function

```rust
pub fn evaluate(inputs: IrrigationInputs<'_>) -> IrrigationDecision;
```

No `self`, no mutation, no I/O, no clock access. The caller loads state from
SQLite, calls `evaluate`, and persists the resulting transition. This makes the
entire safety surface property-testable in microseconds, which is the difference
between safety tests that exist and safety tests that were intended.

### States

```text
        ┌──────────────────────────────────────────────┐
        │                   Normal                     │
        └───────────────────┬──────────────────────────┘
                            │ moisture < target_min
                            ▼
                    ┌───────────────┐
                    │    Drying     │  observing; not yet acting
                    └───────┬───────┘
                            │ continuously dry for dry_confirm_minutes
                            ▼
                    ┌───────────────┐
                    │ DryConfirmed  │──── auto disabled ──► WaterRecommended
                    └───────┬───────┘                        (advisory only)
                            │ SAFETY GATE PASSES
                            ▼
                    ┌───────────────┐
                    │  DoseIssued   │  command written + published
                    └───────┬───────┘
                            │ command result received
              ┌─────────────┼─────────────┐
              │             │             │
        completed      rejected/failed  interrupted
              │             │             │
              ▼             ▼             ▼
   ┌───────────────────┐  ┌──────────────────────┐
   │ WaitForAbsorption │  │ Recheck (no credit)  │
   └─────────┬─────────┘  └──────────┬───────────┘
             │ absorption_wait elapsed │
             ▼                         │
        ┌─────────────┐◄───────────────┘
        │   Recheck   │
        └──────┬──────┘
               │
    ┌──────────┼─────────────────────┬──────────────────┐
    │ recovered│ still dry,          │ still dry,       │
    │          │ doses < max         │ doses == max     │
    ▼          ▼                     ▼                  │
 Normal   DryConfirmed          Locked(MaxDoses)        │
 (cycle    (next dose)                                  │
  done)                                                 │
                                                        │
        ┌───────────────────────────────────────────────┘
        ▼
  ┌──────────────────────────────────────────────────┐
  │ Locked(reason)  — reachable from ANY state       │
  │ Leak | TankLow | StaleData | SensorFault |       │
  │ DailyLimit | MaxDoses | NoDelivery | Uncertain   │
  └──────────────────────────────────────────────────┘
```

`Locked` is reachable from every state on every tick, because a leak does not
wait for a convenient moment.

### The safety gate runs first, always

```rust
fn safety_gate(i: &IrrigationInputs) -> Option<LockoutReason> {
    match i.leak {
        LeakState::Detected => return Some(LockoutReason::Leak),
        LeakState::Unknown  => return Some(LockoutReason::Uncertain),
        LeakState::Clear    => {}
    }
    match i.tank {
        None => return Some(LockoutReason::Uncertain),
        Some(t) if t.percent <= i.profile.tank_min_percent
                => return Some(LockoutReason::TankLow),
        Some(t) if t.is_stale(i.now) => return Some(LockoutReason::StaleData),
        Some(_) => {}
    }
    match i.latest_soil {
        None => return Some(LockoutReason::SensorFault),
        Some(s) if !s.is_valid()      => return Some(LockoutReason::SensorFault),
        Some(s) if s.is_stale(i.now)  => return Some(LockoutReason::StaleData),
        Some(_) => {}
    }
    if i.delivered_today_ml >= i.profile.max_daily_ml {
        return Some(LockoutReason::DailyLimit);
    }
    None
}
```

Properties that matter:

- **Exhaustive matches, no `_ =>` catch-all** on safety inputs. Adding a new
  variant to `LeakState` fails to compile until it is classified. This is the
  compile-time half of SAFETY-012.
- **`None` and `Unknown` map to a lockout, never to permission.** Absence of
  evidence is not evidence of safety.
- The gate is called before any irrigation logic in `evaluate`, and there is no
  second call site that could be forgotten.

### Restart recovery

`irrigation_state` is a table, not a field on an in-memory struct
([ADR-004](004-sqlite-edge-persistence-model.md)). On boot:

1. Load every plant's persisted state — never construct a default for an
   existing plant.
2. Reconcile `commands` in `issued`/`in_flight` per SAFETY-010: expired ones
   become `expired` and move the plant to `Recheck`; live ones stay in flight.
3. Resume ticking.

A plant that was in `WaitForAbsorption` before a crash is still in
`WaitForAbsorption` after it, with the same `wait_until`. This satisfies
SAFETY-010 and is why the state machine can afford to be time-based.

### Modes and what each may bypass

| Mode | Trigger | Gate applied | May bypass |
|---|---|---|---|
| `automatic` | state machine | full gate | nothing |
| `recommended` | operator accepts a recommendation | full gate | nothing |
| `manual` | operator explicit dose | gate minus `SensorFault`/`StaleData` | sensor freshness only |

Manual watering is permitted with a broken sensor because a human has looked at
the plant and taken responsibility — that is exactly the situation where the
automation should step aside. It is **not** permitted during a leak, an empty
tank, or over the daily cap, because those are physical facts about the world
that a human's intent does not change. Manual doses count toward the device's
`FIRMWARE_MAX_DAILY_ML` (SAFETY-007) regardless.

This asymmetry is deliberate, is surfaced in the UI, and is asserted by
`safety_003_leak_blocks_manual_api`.

### Explainability

Every decision carries structured reasons:

```rust
pub enum Reason {
    MoistureBelowTarget  { vwc: f32, target_min: f32 },
    DryFor               { minutes: i64, required: i64 },
    LastWatering         { hours_ago: f64 },
    TrendFalling         { vwc_per_hour: f32 },
    CooldownActive       { remaining_minutes: i64 },
    DailyBudgetRemaining { ml: f32 },
    SafetyLockout        { reason: LockoutReason },
}
```

Reasons are typed, not strings, so the UI can render them and tests can assert
on them. They are persisted with the recommendation and the watering event. The
API renders them to human-readable text in one place.

### Multi-dose rather than one large dose

A cycle delivers up to `max_doses_per_cycle` doses of `dose_ml`, waiting
`absorption_wait_minutes` between them and re-checking moisture.

Rationale: soil moisture responds to water over minutes, not instantly, and a
capacitive probe near the surface over-reports the effect of a fresh pour. A
single 120 ml dose commits to an estimate; three 40 ml doses with feedback
between them converge on the actual need and cap the damage of a wrong estimate
at 40 ml.

## Alternatives considered

**Threshold-based control with a debounce.** Rejected: no memory, no cooldown,
no dose counting, no restart survival, and pathological behaviour when a probe
falls out of the pot.

**PID control.** Rejected: soil moisture has a long, nonlinear, and
poorly-characterised response to water; there is no meaningful actuator
resolution (the pump is on or off); and a PID's integral term is precisely the
mechanism by which a stuck-low sensor produces sustained pumping.

**State machine on the device.** Rejected: the device has no history, no
profiles, and no daily totals, and putting them there would make every rule
change a reflash. The device keeps the veto, which needs no history.

**Actor with internal mutable state.** Rejected: it hides state transitions
behind message handling and makes restart recovery a bespoke serialisation
problem. A pure function over persisted state has neither issue.

**Letting manual watering bypass everything.** Rejected — see the mode table.
The failure it enables (operator clicks "water" while the floor is wet) is both
plausible and expensive.

## Consequences

Positive:

- The full safety surface is property-testable without a database, a broker, or
  a plant.
- Restart behaviour is a consequence of the design rather than a feature that
  had to be added.
- Every decision is explainable to the operator, which is what makes automatic
  mode trustworthy enough to enable.

Negative, accepted:

- Loading state from SQLite every tick is more work than keeping it in memory.
  At one row per plant per 30 seconds this is irrelevant, and it removes an
  entire class of cache-coherence bug.
- The multi-dose cycle takes 30–60 minutes to complete, so a very dry plant is
  not rescued instantly. This is correct behaviour for soil and is documented in
  the UI as expected.
- Reasons as typed enums means adding a reason touches the enum, the renderer,
  and the tests. Accepted for the testability.

## Risks

- **A future contributor adds a decision path that skips `safety_gate`.**
  *Mitigation:* `evaluate` is the only public entry point; the gate is called at
  its top; the module exposes no other decision function. Reviewed as
  safety-critical, and `safety_012_missing_input_never_waters` would fail.
- **Absorption wait tuned wrong for a given soil** produces unnecessary extra
  doses. *Mitigation:* it is a per-profile value, and the no-delivery detector
  (failure-model §5.1) stops runaway cycles regardless.
- **Clock step during a cycle** confuses `wait_until`. *Mitigation:* the
  clock-step lockout in [time-model.md](../architecture/time-model.md) §7.

## Follow-up

- [PRD 060](../prd/060-irrigation-control-and-safety.md) — normative state and transition table.
- [safety-invariants.md](../architecture/safety-invariants.md) — SAFETY-001…012.
- M6-001…M6-016 implement the machine, the gate, and the invariant tests.
