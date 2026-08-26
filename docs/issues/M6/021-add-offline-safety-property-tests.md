# Issue M6-021 — Add offline autonomy safety property tests

**Milestone:** M6 · **PRD:** [PRD 060](../../prd/060-irrigation-control-and-safety.md) · **Depends on:** M6-019, M6-020, M6-018

## Context

SAFETY-013…020 need the same treatment SAFETY-001…012 received: property tests
against a pure function, cheap enough that they actually get run.

## Goal

Prove the offline invariants with property tests.

## Scope

- `safety_013_no_policy_never_actuates`, `safety_013_corrupt_policy_never_actuates`
- `safety_014_combined_budget_never_exceeded` — interleaved commanded and autonomous doses across 72 h with reconnections
- `safety_015_reboot_does_not_replenish_budget`, `safety_015_reboot_does_not_shorten_cooldown`
- `safety_016_replay_is_idempotent`
- `safety_017_missing_required_blocks`, `safety_017_missing_advisory_does_not_block`
- `safety_019_interrupted_activation_leaves_one_valid_policy`
- `safety_020_telemetry_never_evicts_audit`
- Regression corpus committed

## Non-goals

- Hardware invariants — the firmware halves land in M9.

## Dependencies

- M6-019
- M6-020
- M6-018

## Implementation notes

`safety_014_combined_budget_never_exceeded` is the flagship here, the offline
counterpart of `safety_006`. Generate adversarial histories: reboots mid-dose,
reconnections mid-replay, clock steps on the edge, autonomous and commanded doses
interleaved. Assert that at every instant the rolling 24 h sum across **both**
control paths is within the cap.

`safety_017_missing_advisory_does_not_block` is the converse test and is easy to
forget. An implementation that refuses on any missing measurement would pass every
other test here and would make advisory bindings useless.

Commit the shrunk counterexamples. A found bug that stops being tested is a bug
that comes back.

## Acceptance criteria

- [ ] All listed tests exist and pass.
- [ ] `cargo test safety_` runs the full suite including SAFETY-013…020.
- [ ] `PROPTEST_CASES=10000 cargo test safety_014` passes.
- [ ] The advisory-does-not-block converse is tested.
- [ ] `proptest-regressions/` is committed.
- [ ] Each test names the invariant it proves.

## Verification

```bash
cargo test safety_
PROPTEST_CASES=10000 cargo test safety_014
ls proptest-regressions/
```

## Tests required

- The listed property tests.

## Documentation impact

- safety-invariants.md statuses for the covered invariants.

## Files likely affected

```text
crates/policy/tests/safety.rs
crates/domain/tests/safety.rs
proptest-regressions/
```
