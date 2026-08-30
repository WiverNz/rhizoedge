# Issue M6-006 — Implement the irrigation state machine

**Milestone:** M6 · **PRD:** [PRD 060](../../prd/060-irrigation-control-and-safety.md) · **Depends on:** M6-003, M6-005

## Context

ADR-006 and PRD 060's normative transition table. The function is pure and
**total**: every (state, input) pair yields a defined decision, including inputs
that are absent — which resolve to a lockout via the gate.

## Goal

Implement `evaluate` as a pure, total transition function.

## Scope

- `evaluate(inputs) -> IrrigationDecision`
- Every row of PRD 060's transition table
- The gate called first, unconditionally
- `Locked` reachable from every state on every tick
- Multi-dose cycle with `max_doses_per_cycle` and `absorption_wait_minutes`
- Recovery judged by `recovery_delta_vwc` above the pre-dose reading
- Cooldown between completed cycles

## Non-goals

- Persistence (M6-008).
- Publication (M6-009).

## Dependencies

- M6-003
- M6-005

## Implementation notes

Totality is the property to test: `prop_state_machine_total` generates every
state crossed with random inputs and asserts a defined outcome. A partial
function would panic in production on an input nobody anticipated.

No `self`, no mutation, no I/O, no clock access. The caller loads state, calls
`evaluate`, and persists the result — which is what makes ten thousand property
cases cost milliseconds.

`Locked` from any state matters: a leak does not wait for a convenient moment.

## Acceptance criteria

- [x] Every row of the transition table has a passing test.
- [x] The gate is called before any irrigation logic.
- [x] `Locked` is reachable from every state.
- [x] The function is pure — no clock, no I/O.
- [x] `prop_state_machine_total` passes: every state x random inputs yields a defined decision.
- [x] A cycle stops at `max_doses_per_cycle` with `MaxDosesReached`.
- [x] Cooldown is enforced between cycles.

## Verification

```bash
cargo test -p rhizo-domain irrigation::machine
PROPTEST_CASES=10000 cargo test -p rhizo-domain prop_state_machine_total
```

## Tests required

- One test per transition-table row, including illegal transitions.
- `prop_state_machine_total`.
- Multi-dose cycle.
- Cooldown.

## Documentation impact

- PRD 060's transition table verified against the implementation.

## Files likely affected

```text
crates/domain/src/irrigation/machine.rs
```
