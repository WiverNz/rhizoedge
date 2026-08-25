# Issue M7-007 — Implement adaptive batch sizing

**Milestone:** M7 · **PRD:** [PRD 070](../../prd/070-cloud-sync-and-storage.md) · **Depends on:** M7-006

## Context

ADR-005 risk: 500 events may not complete on a slow uplink within the request
timeout, and a batch that always times out never drains.

## Goal

Adapt batch size to the observed link.

## Scope

- Halve the batch size on timeout, floor 10
- Grow back toward the configured maximum after consecutive successes
- The current size exposed as a metric

## Non-goals

- Compression.

## Dependencies

- M7-006

## Implementation notes

The floor of 10 matters: shrinking to 1 would make a large backlog take
effectively forever. If batches of 10 still time out, the problem is not batch
size and the metrics should make that visible.

Grow back gradually — an immediate return to 500 after one success would
oscillate on a marginal link.

## Acceptance criteria

- [ ] A timeout halves the batch size.
- [ ] The size never goes below 10.
- [ ] It grows back after consecutive successes.
- [ ] The current size is exported as a metric.
- [ ] A slow link eventually drains rather than stalling.

## Verification

```bash
cargo test -p edge-controller outbox::batch_size
```

## Tests required

- Halving on timeout.
- Floor enforcement.
- Growth after success.
- A simulated slow link drains.

## Documentation impact

- None.

## Files likely affected

```text
crates/edge-controller/src/cloud/drain.rs
```
