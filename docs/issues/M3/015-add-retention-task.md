# Issue M3-015 — Implement the retention task

**Milestone:** M3 · **PRD:** [PRD 030](../../prd/030-edge-ingestion-and-storage.md) · **Depends on:** M3-009

## Context

ADR-004 sets retention: eligible `processed_messages` 7 days, synced outbox rows
24 hours, quarantine 1000 rows, measurements 90 days. A marker is eligible only
when its durable effect has independent stable uniqueness; status markers are
retained. Watering events, commands, and device events are **never** pruned —
they are the record of what the machine did to a living plant.

## Goal

Prune bounded tables on a schedule without touching the ledger.

## Scope

- A periodic task (hourly) applying the retention rules
- Deletes in batches so a large backlog does not lock the database
- `watering_events`, `commands`, and `device_events` explicitly excluded
- A metric for rows pruned per table

## Non-goals

- Downsampling measurements (M13-010).

## Dependencies

- M3-009

## Implementation notes

The 7-day `processed_messages` horizon is an operational bound, not the
correctness mechanism. Tests remove eligible markers and replay the original
messages to prove stable effect keys still prevent duplicate logical rows.
Status is excluded because retained messages and fixed LWT payloads have no
bounded application-level redelivery horizon.

Batch the deletes. A single `DELETE` over months of rows holds a write lock long
enough to stall ingestion.

Make the never-pruned exclusion explicit in code and assert it in a test — a
future 'tidy up old data' change is exactly how a ledger gets destroyed.

## Acceptance criteria

- [x] `processed_messages` older than 7 days are pruned only for message kinds
      whose durable effects have independent stable uniqueness; status markers
      remain because retained/LWT redelivery is unbounded.
- [x] Synced outbox rows older than 24 hours are pruned.
- [x] Quarantine is capped at 1000 rows.
- [x] Measurements older than 90 days are pruned.
- [x] `watering_events`, `commands`, and `device_events` row counts are **unchanged** after a run over old data.
- [x] Deletes are batched and do not stall ingestion.

## Verification

```bash
cargo test -p edge-controller retention::
```

## Tests required

- Each retention rule.
- An explicit test that ledger tables are untouched over 5-year-old data.
- Batching under a large backlog.

## Documentation impact

- None.

## Files likely affected

```text
crates/edge-controller/src/retention.rs
```
