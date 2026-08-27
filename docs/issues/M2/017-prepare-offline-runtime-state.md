# Issue M2-017 — Prepare offline runtime state and isolation mechanics

**Milestone:** M2 · **PRD:** [PRD 020](../../prd/020-device-simulator.md) · **Depends on:** M2-016, M2-007, M2-013, M2-014

## Context

The simulator must faithfully model what an isolated device can persist and
observe before autonomous decisions are activated. The one shared evaluator is
implemented later by M6-019; implementing rules here would create the forbidden
simulator-specific evaluator described by ADR-008.

## Goal

Prepare the simulator's isolation lifecycle and persisted monotonic runtime state
without evaluating policy or autonomously actuating.

## Scope

- Detect MQTT loss/reconnection and expose connected versus isolated mode
- Continue sampling, typed telemetry production for the local buffer, physical
  model evolution, and monotonic time while isolated
- Persist the `OfflineState` fields required by M6-019: budget accumulator,
  cooldown remaining, confirmation elapsed, dose count, and window state
- On reboot with no trustworthy elapsed-time evidence, assume no time passed:
  cooldown is not shortened and budget is not replenished
- Load and expose the active policy/version from M2-016 as input for later use
- Provide a single integration seam that M6-019 will connect to
  `rhizo_policy::evaluate_offline`
- Assert that M2 has no offline decision implementation and schedules no
  autonomous dose

## Non-goals

- Offline policy evaluation (`M6-019`).
- Autonomous offline watering decisions or dose scheduling (`M6-019`).
- A simulator-specific copy of the offline rules — permanently forbidden.
- Firmware integration (`M9-016`).

## Dependencies

- M2-016
- M2-007
- M2-013
- M2-014

## Implementation notes

`cooldown_remaining_ms` is stored as a remaining duration, never a wall-clock
deadline. The runtime seam may gather typed inputs, but it must not classify
them into `Dose`, `Refuse`, or any other policy decision in M2.

The simulator may exercise commanded watering through the existing
`validate_water_command` call site. There is no autonomous caller until M6-019,
when the shared evaluator and its integration arrive together.

## Acceptance criteria

- [ ] Network isolation leaves the process, sampling, physical model, and local buffering running.
- [ ] Reconnection is detected and reported without resetting persisted policy state.
- [ ] Offline runtime state round-trips through the simulator state file.
- [ ] Reboot never shortens a stored cooldown or replenishes stored budget.
- [ ] The active policy and version are available through the M6 integration seam.
- [ ] No `evaluate_offline` implementation or simulator-specific equivalent exists.
- [ ] M2 schedules no autonomous dose, including with an enabled valid policy.
- [ ] Commanded actuation still has exactly one `validate_water_command` call site.

## Verification

```bash
cargo test -p device-simulator isolation::
cargo test -p device-simulator offline_state::
grep -rn 'evaluate_offline' crates/device-simulator/src && exit 1 || true
grep -rn 'validate_water_command' crates/device-simulator/src
```

## Tests required

- Isolation/reconnection lifecycle.
- Runtime-state persistence and conservative reboot handling.
- Structural absence of an offline evaluator and autonomous scheduler.
- Single validator call-site assertion.

## Documentation impact

- M6-019 owns evaluator implementation and simulator activation.

## Files likely affected

```text
crates/device-simulator/src/isolation.rs
crates/device-simulator/src/offline_state.rs
crates/device-simulator/src/state.rs
```
