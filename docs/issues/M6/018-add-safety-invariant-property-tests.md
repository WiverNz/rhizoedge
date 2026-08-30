# Issue M6-018 — Add the full safety invariant property test suite

**Milestone:** M6 · **PRD:** [PRD 060](../../prd/060-irrigation-control-and-safety.md) · **Depends on:** M6-017, M6-012, M6-015

## Context

The heaviest test investment in the project. ADR-006's purity is what makes
this affordable: ten thousand adversarial cases against the gate cost
milliseconds because there is no database or broker involved.

## Goal

Prove every non-hardware invariant with property tests.

## Scope

- All ten property tests from testing/strategy.md section 4
- `safety_NNN_*` naming so `cargo test safety_` runs the suite
- `proptest-regressions/` committed — a shrunk counterexample is permanent evidence
- Each test names the invariant it proves

## Non-goals

- Hardware invariants SAFETY-011 (M9) and the physical parts of SAFETY-007 (M11).

## Dependencies

- M6-017
- M6-012
- M6-015

## Implementation notes

The flagship is `safety_006_rolling_24h_cap_never_exceeded`. It must generate
genuinely adversarial histories: restarts between publish and result, forward and
backward clock steps, interrupted doses, duplicate results — and assert that at
every instant the rolling sum is within the cap. If one property test survives,
it is that one.

Commit the regression corpus. A shrunk counterexample found once must keep
passing forever.

## Acceptance criteria

- [x] All ten property tests exist and pass.
- [x] `cargo test safety_` runs the complete suite.
- [x] Each test names its invariant.
- [x] `PROPTEST_CASES=10000 cargo test safety_` passes.
- [x] `proptest-regressions/` is committed.
- [x] SAFETY-001 through 007, 010, and 012 each have at least one passing property or integration test.

## Verification

```bash
cargo test safety_
PROPTEST_CASES=10000 cargo test safety_006
ls proptest-regressions/
```

## Tests required

- The ten tests in testing/strategy.md section 4.

## Documentation impact

- safety-invariants.md status column updated to ENFORCED for the covered invariants.

## Files likely affected

```text
crates/domain/tests/safety.rs
proptest-regressions/
```
