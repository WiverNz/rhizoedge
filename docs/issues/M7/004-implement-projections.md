# Issue M7-004 — Implement order-insensitive projections

**Milestone:** M7 · **PRD:** [PRD 070](../../prd/070-cloud-sync-and-storage.md) · **Depends on:** M7-003

## Context

ADR-005: events may arrive out of order because retries interleave, so
projections must be order-insensitive. A late-arriving old status must not
overwrite a newer one.

## Goal

Maintain the projection tables from the ledger, safely under reordering.

## Scope

- Project each known event kind into its table
- Projection in the **same transaction** as the ledger insert
- `devices.status` guarded by `WHERE excluded.last_seen_at > devices.last_seen_at`
- `measurements` keyed by a natural composite PK so re-projection is an upsert
- `watering_events` keyed by id; `completed_at` fills in whenever it arrives

## Non-goals

- Reprojection tooling (M7-012).

## Dependencies

- M7-003

## Implementation notes

The status guard is the concrete instance of order-insensitivity, and it is
easy to omit: a plain upsert would let a delayed `offline` event mark a device
that has since reconnected as dead.

Same-transaction projection means the ledger and the tables cannot diverge
through a partial failure.

## Acceptance criteria

- [ ] Each known kind projects into its table.
- [ ] Projection shares the ledger's transaction.
- [ ] A late older status does **not** overwrite a newer one.
- [ ] Re-projecting the same event is an upsert with no duplicate.
- [ ] `watering.completed` fills in `completed_at` regardless of arrival order.
- [ ] Shuffling a batch produces identical final projections.

## Verification

```bash
cargo test -p cloud-api project::
```

## Tests required

- Per-kind projection.
- **Out-of-order status guard.**
- Shuffled-batch equivalence.
- Upsert idempotency.

## Documentation impact

- None.

## Files likely affected

```text
crates/cloud-api/src/project.rs
```
