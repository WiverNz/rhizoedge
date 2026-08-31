# Issue M7-010 — Add the cloud independence differential test

**Milestone:** M7 · **PRD:** [PRD 070](../../prd/070-cloud-sync-and-storage.md) · **Depends on:** M7-006, M6-018

## Context

**SAFETY-009's test, and the strongest single statement the project can make
about edge-first correctness**: the same seeded scenario, run with the cloud up
and with it down, must produce identical command sequences.

## Goal

Prove that cloud availability cannot influence a watering decision.

## Scope

- Run a seeded scenario twice: cloud up, cloud down
- Capture every issued command and every lockout
- Assert the sequences are **identical** modulo ids and timestamps
- Assert every lockout occurs in both runs
- Also assert `rhizo-domain` has no cloud dependency

## Non-goals

- Testing the cloud itself.

## Dependencies

- M7-006
- M6-018

## Implementation notes

Determinism is a prerequisite: use `TestClock` and a fixed seed so the only
difference between runs is cloud availability. Without that, a flaky difference
would be indistinguishable from a real one.

The dependency assertion is the structural half — `IrrigationInputs` has no
cloud field and `rhizo-domain` cannot depend on `rhizo-cloud-client`, so the
differential test confirms what the type system already forbids.

## Acceptance criteria

- [x] The scenario runs identically with the cloud up and down.
- [x] Command sequences match modulo ids and timestamps.
- [x] Every lockout occurs in both runs.
- [x] `rhizo-domain`'s dependency list contains no cloud crate.
- [x] The test is deterministic across repeated runs.
- [x] **`safety_009_decisions_identical_with_cloud_down` passes.**

## Verification

```bash
cargo test safety_009
cargo test --test integration cloud_independence
```

## Tests required

- **`safety_009_decisions_identical_with_cloud_down` (SCEN-061).**
- The dependency assertion.

## Documentation impact

- safety-invariants.md SAFETY-009 status.

## Files likely affected

```text
crates/edge-controller/tests/cloud_independence.rs
```
