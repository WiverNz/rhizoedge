# Issue M2-013 — Implement the fault injection catalogue

**Milestone:** M2 · **PRD:** [PRD 020](../../prd/020-device-simulator.md) · **Depends on:** M2-008, M2-009

## Context

Simulator-strategy section 6 lists every fault the simulator must reproduce.
They are the mechanism by which failure-model.md's device-originated failures
become testable rather than merely documented.

## Goal

Implement every documented fault, settable at startup and at runtime.

## Scope

- disconnect, duplicate, reorder, invalid-soil, stuck-sensor, clock-unsync, clock-skew, leak, tank-empty, pump-no-delivery, pump-stuck-on, restart-mid-dose, restart
- Settable via CLI and the control API
- Rate-based faults take a probability
- Faults compose (leak plus tank-empty simultaneously)

## Non-goals

- Edge-side faults such as SQLITE_BUSY (M3).

## Dependencies

- M2-008
- M2-009

## Implementation notes

`pump-stuck-on` must exercise the run-guard path: the pump model fails to
de-energise and something else must stop it. In the simulator that is a
software timer; in firmware (M11-002) it is a separate task. The fault exists to
prove the guard is independent.

`restart-mid-dose` must terminate the process during actuation, after the state
write, so the interrupted-dose path is genuinely exercised.

`duplicate` republishes with the **same** `message_id` — a new id would be a
different message and would not test deduplication at all.

## Acceptance criteria

- [x] Every fault in the catalogue is implemented.
- [x] Each is settable at startup and at runtime.
- [x] `duplicate` republishes with an identical `message_id`.
- [x] `restart-mid-dose` terminates during actuation after the state write.
- [x] `clock-unsync` causes every water command to be refused.
- [x] `pump-no-delivery` runs the pump without changing moisture or weight.
- [x] Faults compose.

## Verification

```bash
cargo test -p device-simulator --test faults
cargo test -p device-simulator --lib fault::
cargo run -p device-simulator -- --device-id plant-node-01 --fault leak
```

`tests/faults.rs` has one test per fault asserting its **observable effect**, a
composition test, and two structural ones: `no_fault_can_cause_a_dose` (no
injected fault can start the pump or move water) and
`every_fault_in_the_catalogue_has_a_test_here` (the catalogue and the test file
cannot drift apart). The two transport faults, `duplicate` and `reorder`, are
tested as a pure pipeline in `fault::pipeline_tests`; `policy-interrupt` lands
with policy activation in M2-016.

## Tests required

- One test per fault asserting its observable effect.
- Composition of two faults.
- `duplicate` produces identical message ids.

## Documentation impact

- docs/testing/simulator-strategy.md §6 verified accurate: all thirteen faults
  behave as described, and `policy-interrupt` is the one addition (M2-016).

## Files likely affected

```text
crates/device-simulator/src/fault.rs
```
