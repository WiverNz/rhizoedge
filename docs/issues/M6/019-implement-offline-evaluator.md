# Issue M6-019 — Implement the offline evaluator in rhizo-policy

**Milestone:** M6 · **PRD:** [PRD 060](../../prd/060-irrigation-control-and-safety.md) · **Depends on:** M6-002, M6-006, M2-017

## Context

The restricted decision function an isolated device runs
([offline-autonomy.md](../../architecture/offline-autonomy.md) §4). It is pure,
`no_std`, takes elapsed time as a parameter, and is the **only** implementation of
the offline rules.

## Goal

Implement `evaluate_offline` with its gate, exhaustively and conservatively,
then activate it in the existing simulator through one shared call site.

## Scope

- `evaluate_offline(policy, state, inputs, elapsed) -> OfflineDecision`
- Gate in the documented order: enabled, policy validity, leak, tank, pump, required measurements, control validity, budget, cooldown, timebase
- **Exhaustive matches, no catch-all arm** on any safety input
- Cycle logic: confirm → dose → absorption → recheck → cooldown, with hysteresis and `max_doses_per_cycle`
- Rolling budget from the persisted accumulator and window start
- Pure: no clock, no I/O, no allocation beyond `alloc`
- Extend the M2 simulator integration seam to call this evaluator from exactly one place while isolated
- Route `Dose` decisions into the simulator's existing single actuation path

## Non-goals

- Persistence (M2-016, M9-015).
- Firmware actuation/integration (M9-016).
- A simulator-local evaluator or duplicate rule implementation.
- Any rule outside the restricted subset — trends, recommendations, dose computation.

## Dependencies

- M6-002
- M6-006
- M2-017

## Implementation notes

Mirror the connected gate's discipline exactly: `Option`/tri-state inputs,
exhaustive matches, no `_ =>` arm. Adding a variant to `LeakState` must fail to
compile until it is classified here too.

Resist scope creep hard. Every rule the offline evaluator gains is a rule that
must be re-verified on constrained hardware and that widens the gap between the
two evaluators. If a rule needs history the device does not have, it belongs to
the Edge and is simply unavailable when isolated.

`elapsed` is a parameter. The crate must not be able to read a clock, which is
what makes SAFETY-015 structural.

M2-017 deliberately prepares persistence and isolation mechanics without making
decisions. This issue adds the evaluator and simulator call site together, so
there is never an interim simulator-specific implementation.

## Acceptance criteria

- [ ] Every gate step returns its documented `RefuseReason`.
- [ ] `None` or `Unknown` on any safety input refuses; never permits.
- [ ] No `_ =>` arm exists on any safety match.
- [ ] A full cycle runs: confirm, dose, absorb, recheck, second dose, cooldown.
- [ ] Hysteresis prevents dosing while between `trigger_below` and `resume_above`.
- [ ] The budget is respected and replenishes only as the window advances.
- [ ] The function is pure — no clock access anywhere in the crate.
- [ ] `cargo build -p rhizo-policy --no-default-features` still succeeds.
- [ ] The simulator calls `rhizo_policy::evaluate_offline` from exactly one place.
- [ ] An isolated simulator with a valid enabled policy can now schedule bounded autonomous doses.
- [ ] No simulator-local evaluator exists.

## Verification

```bash
cargo test -p rhizo-policy
cargo test safety_013 safety_017
PROPTEST_CASES=10000 cargo test -p rhizo-policy prop_
cargo test -p device-simulator offline::
grep -rn 'evaluate_offline' crates/device-simulator/src
```

## Tests required

- One test per gate step.
- Full-cycle test.
- Hysteresis boundary.
- `prop_offline_evaluator_total` — every state x random inputs yields a defined decision.
- A compile-fail test for an unclassified new enum variant.

## Documentation impact

- offline-autonomy.md §4 verified against the implementation.

## Files likely affected

```text
crates/policy/src/evaluate.rs
crates/policy/src/gate.rs
crates/device-simulator/src/offline.rs
```
