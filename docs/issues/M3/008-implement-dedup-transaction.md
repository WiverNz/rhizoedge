# Issue M3-008 — Implement the deduplicate-and-persist transaction

**Milestone:** M3 · **PRD:** [PRD 030](../../prd/030-edge-ingestion-and-storage.md) · **Depends on:** M3-002, M3-003

## Context

**The mechanism behind SAFETY-001 and SAFETY-010.** The dedup marker and the
message's effects must be durable together or not at all. Deduplicating in
memory would fail on restart, which is precisely the case that matters.

## Goal

Guarantee exactly-once logical processing across crashes.

## Scope

- `mark_processed` doing `INSERT ... ON CONFLICT DO NOTHING`
- 0 rows affected means duplicate: roll back, count, apply nothing
- Dedup key is `message_id` alone
- The marker and all effects share one transaction
- `SQLITE_BUSY` retried 3x with 50/100/200 ms jitter, then a clean failure leaving the message unprocessed

## Non-goals

- Deciding what effects to apply (M3-009).

## Dependencies

- M3-002
- M3-003

## Implementation notes

`(device_id, boot_id, sequence)` must **not** be used for dedup — a device
rebooting mid-second could legitimately reuse a sequence value while
`message_id` remains unique.

On a `SQLITE_BUSY` failure the message must be left unprocessed, so QoS 1
redelivery reprocesses it correctly. Marking it processed and then failing to
apply effects would lose data silently.

Structure the API so it is hard to apply an effect outside the transaction: the
effect methods take `&mut Transaction` and there is no pool-based variant.

## Acceptance criteria

- [x] A new `message_id` returns `New` and effects apply.
- [x] A repeated `message_id` returns `Duplicate` and **nothing** is written.
- [x] A crash simulated between marker and effects leaves neither durable.
- [x] `SQLITE_BUSY` retries then fails cleanly with the message unprocessed.
- [x] `mqtt_duplicate_messages_total` increments on a duplicate.
- [x] There is no API to apply an effect without a transaction.

## Verification

```bash
cargo test -p rhizo-storage dedup::
cargo test --test integration duplicate_telemetry
cargo test safety_001
```

## Tests required

- New/duplicate paths.
- Rollback leaves nothing.
- SCEN-010 duplicate QoS 1 telemetry produces one row.
- `prop_dedup_idempotent`: random duplication of a stream yields distinct-id count.

## Documentation impact

- None; ADR-004 documents the mechanism.

## Files likely affected

```text
crates/storage/src/repo/ingest.rs
crates/edge-controller/src/pipeline/mod.rs
```
