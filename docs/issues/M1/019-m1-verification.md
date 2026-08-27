# Issue M1-019 — M1 verification and exit criteria

**Milestone:** M1 · **PRD:** [PRD 010](../../prd/010-domain-and-mqtt-protocol.md) · **Depends on:** M1-001, M1-002, M1-003, M1-004, M1-005, M1-006, M1-007, M1-008, M1-009, M1-010, M1-011, M1-012, M1-013, M1-014, M1-015, M1-016, M1-017, M1-018

## Context

Final gate for M1. The protocol is the artefact that cannot be changed cheaply
once devices exist, so this verification is worth doing carefully.

## Goal

Verify every PRD 010 acceptance criterion.

## Scope

- Run the full gate plus the no_std and fixture checks
- Review the implementation against protocol/mqtt-v1.md clause by clause
- Update ROADMAP.md M1 status
- Record the milestone report

## Non-goals

- New behaviour.

## Dependencies

- M1-001
- M1-002
- M1-003
- M1-004
- M1-005
- M1-006
- M1-007
- M1-008
- M1-009
- M1-010
- M1-011
- M1-012
- M1-013
- M1-014
- M1-015
- M1-016
- M1-017
- M1-018

## Implementation notes

The clause-by-clause review against mqtt-v1.md is the substance of this
issue. Any divergence is resolved by changing the code, or — if the spec is
wrong — by changing the spec deliberately and noting it in the report. Silent
divergence between the normative document and the implementation is the failure
mode to avoid.

Pay particular attention to protocol section 5.8's ordering: it is the clause
most likely to be implemented approximately.

## Acceptance criteria

- [x] All gate commands pass.
- [x] The no_std build passes.
- [x] Every fixture behaves as documented.
- [x] Every clause of mqtt-v1.md sections 2-10 is implemented or explicitly noted.
- [x] `validate_water_command` order matches section 5.8 exactly.
- [x] `cargo test safety_` passes for the SAFETY-002 and SAFETY-007 tests that exist at this stage.
- [x] ROADMAP.md M1 status updated.
- [x] Milestone report recorded.

## Verification

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo build -p rhizo-mqtt-contract --no-default-features --target thumbv7em-none-eabi
cargo test safety_
cargo run --manifest-path tools/docscheck/Cargo.toml
```

## Tests required

- Full suite.

## Documentation impact

- ROADMAP.md.
- Milestone report.
- mqtt-v1.md corrected if divergence was found.

## Files likely affected

```text
ROADMAP.md
```
