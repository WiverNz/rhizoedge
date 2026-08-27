# Issue M3-009 — Persist measurements and device events with per-field validation

**Milestone:** M3 · **PRD:** [PRD 030](../../prd/030-edge-ingestion-and-storage.md) · **Depends on:** M3-008, M1-006

## Context

Protocol section 10's asymmetry: a telemetry batch with one invalid typed
sample keeps its good sibling samples and stores the invalid sample with null
value columns. Discarding the whole batch would throw away useful readings.

## Goal

Write measurements and diagnostic events inside the dedup transaction.

## Scope

- Validate every `MeasurementSample` through the shared kind specification;
  invalid value columns become NULL plus a `sensor_invalid` event
- Insert one narrow row per sample, preserving `batch_id`, point, kind, unit,
  quality, calibration reference, `received_at`, and advisory `device_time_ms`
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

`leak_state` with a null value must be stored as NULL and must **not** become 0. M6 reads
it as a tri-state, and a null-to-false conversion here would silently defeat
SAFETY-012 later.

All of this happens inside the M3-008 transaction.

## Acceptance criteria

- [x] A valid batch produces one row per typed sample sharing its `batch_id`.
- [x] An out-of-range soil-moisture sample stores NULL value columns, keeps sibling rows, and raises `sensor_invalid`.
- [x] `NaN` is handled identically.
- [x] A null `leak_state` sample stores NULL, not 0.
- [x] A sequence regression within a boot raises an event without rejecting.
- [x] A `boot_id` change raises `boot` and the sequence restart is **not** flagged.
- [x] An outbox row is written in the same transaction.

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
