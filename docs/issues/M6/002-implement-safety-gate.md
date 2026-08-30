# Issue M6-002 — Implement the safety gate

**Milestone:** M6 · **PRD:** [PRD 060](../../prd/060-irrigation-control-and-safety.md) · **Depends on:** M6-001

## Context

**The most safety-critical function in the project.** ADR-006: the gate runs
first, always, from the single entry point, with exhaustive matches and no
catch-all arm — so a new enum variant fails to compile until someone classifies
it.

## Goal

Implement the ordered safety gate with compile-time exhaustiveness.

## Scope

- `safety_gate(&IrrigationInputs) -> Option<LockoutReason>`
- Ordered: leak, tank, sample validity, sample freshness, daily cap
- **Exhaustive matches with no `_ =>` arm on any safety input**
- `None` and `Unknown` map to a lockout, never to permission
- Called at the top of `evaluate`, from nowhere else

## Non-goals

- Mode-specific relaxation (M6-005 handles the manual exception).

## Dependencies

- M6-001

## Implementation notes

The no-catch-all rule is the compile-time half of SAFETY-012. Write the
matches so that adding a `LeakState::Degraded` variant later breaks the build
until someone decides what it means.

`evaluate` must be the only public entry point, and the gate must be its first
statement. A second decision function that skipped the gate would be
undetectable in review.

Order: leak → leak-unknown → tank-unknown → tank-low → tank-stale → sample-absent
→ sample-invalid → sample-stale → daily-cap.

## Acceptance criteria

- [x] Each lockout reason is returned under its documented condition.
- [x] `leak = Unknown` returns a lockout, not `None`.
- [x] `tank = None` returns a lockout.
- [x] `latest_soil = None` returns a lockout.
- [x] **No `_ =>` arm exists on any safety match.**
- [x] Adding an enum variant fails to compile until classified — held by the
      compiler, since every safety match is exhaustive. Asserted by
      `safety_012_no_catch_all_arm_on_a_safety_match` reading the gate's own
      source rather than by a `trybuild` case, because what a compile-fail test
      would *not* catch is a `_ =>` arm added to make the build pass, which is
      the real risk (see [docs/reports/M6.md](../../reports/M6.md) §Deviations).
- [x] The gate is the first statement in `evaluate`.
- [x] There is no other public decision function.

## Verification

```bash
cargo test -p rhizo-domain safety_gate::
cargo test safety_012
grep -n '_ =>' crates/domain/src/irrigation/gate.rs   # expect no safety matches
```

## Tests required

- One test per lockout reason in isolation.
- **`safety_012_missing_input_never_waters`** as a property test over inputs with random fields None.
- A compile-fail test (trybuild) for an unclassified new variant. **Delivered as
  a source scan instead**: `safety_012_no_catch_all_arm_on_a_safety_match`. The
  exhaustiveness itself is already a compiler property; the scan covers the way
  it would actually be lost.

## Documentation impact

- Doc comment citing SAFETY-003, -004, -005, -006, -012.

## Files likely affected

```text
crates/domain/src/irrigation/gate.rs
```
