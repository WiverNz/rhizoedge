# Issue M3-012 — Add a metric cardinality guard test

**Milestone:** M3 · **PRD:** [PRD 030](../../prd/030-edge-ingestion-and-storage.md) · **Depends on:** M3-011

## Context

ADR-010 identifies cardinality creep as a real risk: someone adds `device_id`
to a hot counter and the series count multiplies by the fleet size. A test makes
it visible at review time.

## Goal

Fail the build when exported series count grows unexpectedly.

## Scope

- A test running a fixed fixture workload
- Count exported series and assert it stays below a documented threshold
- The threshold documented with its reasoning
- A failure message explaining cardinality discipline

## Non-goals

- Runtime cardinality limiting.

## Dependencies

- M3-011

## Implementation notes

Use a fixed fixture (three devices, a handful of message kinds) so the count
is deterministic. Set the threshold with headroom for legitimate growth, and
update it deliberately when a metric is added.

The failure message is the useful part — it should say 'a new label was probably
added; check ADR-010's cardinality rules' rather than just showing two numbers.

## Acceptance criteria

- [x] The test passes at current cardinality.
- [x] Adding a `device_id` label to a hot counter fails it.
- [x] The threshold and its reasoning are documented.
- [x] The failure message explains what to check.

## Verification

```bash
cargo test -p edge-controller metrics::cardinality
```

## Tests required

- The guard test.
- Manual: add a label, confirm failure, revert.

## Documentation impact

- Comment in the test documenting the threshold.

## Files likely affected

```text
crates/edge-controller/tests/metrics_cardinality.rs
```
