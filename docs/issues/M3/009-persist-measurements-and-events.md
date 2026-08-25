# Issue M3-009 — Persist measurements and device events with per-field validation

**Milestone:** M3 · **PRD:** [PRD 030](../../prd/030-edge-ingestion-and-storage.md) · **Depends on:** M3-008, M1-006

## Context

Protocol section 10's asymmetry: a message with one out-of-range field keeps
its good fields and nulls the bad one. Discarding the whole message would throw
away the reading the safety logic needs.

## Goal

Write measurements and diagnostic events inside the dedup transaction.

## Scope

- Range-validate each field; invalid becomes NULL plus a `sensor_invalid` event
- Insert the measurement row with `received_at` and advisory `device_time_ms`
- Update `devices.last_seen_at`, `boot_id`, `last_sequence`
- Record `sequence_regression` when sequence decreases within a `boot_id`
- Record `boot` when `boot_id` changes; **do not** flag the sequence restart
- Insert the outbox row (drained from M7)

## Non-goals

- Device online/offline (M4).
- Outbox draining (M7-006).

## Dependencies

- M3-008
- M1-006

## Implementation notes

The boot_id/sequence interaction is subtle and easy to get wrong: a restart
legitimately resets the sequence, so a naive regression check fires on every
reboot and buries the real signal.

`leak_detected: null` must be stored as NULL and must **not** become 0. M6 reads
it as a tri-state, and a null-to-false conversion here would silently defeat
SAFETY-012 later.

All of this happens inside the M3-008 transaction.

## Acceptance criteria

- [ ] A valid message produces one measurement row.
- [ ] `moisture_vwc: 150` stores NULL for that field, keeps the others, and raises `sensor_invalid`.
- [ ] `NaN` is handled identically.
- [ ] `leak_detected: null` stores NULL, not 0.
- [ ] A sequence regression within a boot raises an event without rejecting.
- [ ] A `boot_id` change raises `boot` and the sequence restart is **not** flagged.
- [ ] An outbox row is written in the same transaction.

## Verification

```bash
cargo test -p edge-controller persist::
cargo test --test integration partial_invalid_fields
```

## Tests required

- Per-field nulling.
- Boot vs regression discrimination.
- Null leak preserved as NULL.
- Outbox row written atomically.

## Documentation impact

- None.

## Files likely affected

```text
crates/edge-controller/src/pipeline/persist.rs
crates/storage/src/repo/measurement.rs
```
