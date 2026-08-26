# Issue M3-016 — Ingest replayed offline events idempotently

**Milestone:** M3 · **PRD:** [PRD 030](../../prd/030-edge-ingestion-and-storage.md) · **Depends on:** M3-008, M3-009

## Context

A reconnecting device replays what happened while it was isolated
([mqtt-v1.md](../../protocol/mqtt-v1.md) §5.4). The ingestion path already
deduplicates on `message_id`; replayed events additionally carry a device-generated
`event_id` that must be the deduplication key (SAFETY-016).

## Goal

Ingest replayed history exactly once, however many times it is replayed.

## Scope

- Handle `device.events` batches through the existing dedup transaction
- Deduplicate on `event_id`, not on `message_id`, for replayed events
- Persist measurement samples with `origin = 'offline_replay'`
- Persist autonomous doses as `watering_events` with `origin = 'offline_autonomous'`
- Track `complete` and record when a device's replay has finished
- Acknowledge processed batches so the device may release them

## Non-goals

- Releasing the plant from `Uncertain` (M6-020).
- Gap recording (M3-017).

## Dependencies

- M3-008
- M3-009

## Implementation notes

Use the same `processed_messages` transaction as live telemetry. The key
difference is the identifier: a replayed batch may arrive in several MQTT
messages with different `message_id`s while carrying the same `event_id`s, so
deduplicating on `message_id` alone would let a re-replayed batch through.

`origin` is what keeps history honest: an operator looking at a watering event
must be able to tell whether the edge asked for it or the device decided alone.

## Acceptance criteria

- [ ] A replayed batch is ingested and its events stored.
- [ ] Replaying the same batch three times creates one row per `event_id`.
- [ ] A batch split differently across messages still deduplicates correctly.
- [ ] Autonomous doses are stored with `origin = 'offline_autonomous'`.
- [ ] Replayed samples are stored with `origin = 'offline_replay'`.
- [ ] `complete` is recorded so M6-020 can act on it.
- [ ] An unacknowledged batch is safe to reprocess after an edge restart.

## Verification

```bash
cargo test -p edge-controller replay::
cargo test --test integration offline_replay
cargo test safety_016
```

## Tests required

- SCEN-100, SCEN-101, SCEN-102.
- Idempotency property test.

## Documentation impact

- None.

## Files likely affected

```text
crates/edge-controller/src/pipeline/replay.rs
crates/storage/src/repo/replay.rs
```
