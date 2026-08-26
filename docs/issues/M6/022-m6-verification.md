# Issue M6-022 — M6 verification and exit criteria

**Milestone:** M6 · **PRD:** [PRD 060](../../prd/060-irrigation-control-and-safety.md) · **Depends on:** M6-001, M6-002, M6-003, M6-004, M6-005, M6-006, M6-007, M6-008, M6-009, M6-010, M6-011, M6-012, M6-013, M6-014, M6-015, M6-016, M6-017, M6-018, M6-019, M6-020, M6-021

## Context

Final gate for M6, and the most consequential verification in the project:
from here the software can move water.

## Goal

Verify every PRD 060 acceptance criterion and every enforced invariant.

## Scope

- Full gate plus the whole safety suite
- Verify the no-catch-all property by inspection and by a compile-fail test
- Verify the single-gate property: every actuation path calls `evaluate`
- Update safety-invariants.md statuses and ROADMAP.md
- Record the report

## Non-goals

- New behaviour.

## Dependencies

- M6-001
- M6-002
- M6-003
- M6-004
- M6-005
- M6-006
- M6-007
- M6-008
- M6-009
- M6-010
- M6-011
- M6-012
- M6-013
- M6-014
- M6-015
- M6-016
- M6-017
- M6-018
- M6-019
- M6-020
- M6-021

## Implementation notes

Three verifications carry the weight:

1. `POST /water` during a leak returns 409 **and publishes no MQTT message** —
   confirmed by a spy subscriber, not by reading code.
2. Killing the edge after publish and restarting produces **no second command**
   and exactly one watering event.
3. `PROPTEST_CASES=10000 cargo test safety_006` passes.

Also confirm by inspection that `evaluate` is the only public decision function
and that no `_ =>` arm exists on a safety match.

## Acceptance criteria

- [ ] All gate commands pass.
- [ ] `cargo test safety_` is fully green.
- [ ] SCEN-002's full cycle produces the exact documented state sequence.
- [ ] Duplicate commands actuate once.
- [ ] `POST /water` during a leak returns 409 with **no MQTT published**.
- [ ] Restart after publish produces no second command and one watering event.
- [ ] A plant with no tank sensor never receives a dose.
- [ ] A stale plant is blocked automatically but can be watered manually.
- [ ] No `_ =>` arm on any safety match.
- [ ] `PROPTEST_CASES=10000 cargo test safety_006` passes.
- [ ] safety-invariants.md statuses and ROADMAP.md updated; report recorded.

## Verification

```bash
cargo test --workspace --all-features
cargo test safety_
PROPTEST_CASES=10000 cargo test safety_006
cargo test --test integration
cargo run --manifest-path tools/docscheck/Cargo.toml
```

## Tests required

- Full suite plus the safety suite at high case counts.

## Documentation impact

- safety-invariants.md.
- ROADMAP.md.
- Milestone report.

## Files likely affected

```text
docs/architecture/safety-invariants.md
ROADMAP.md
```
