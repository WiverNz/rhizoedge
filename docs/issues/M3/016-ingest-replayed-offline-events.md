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
- Publish `event.ack` (mqtt-v1.md §5.13) **after** the persisting transaction
  commits, covering the highest contiguous `device_seq`, non-retained, QoS 1

## Non-goals

- Releasing the plant from `Uncertain` (M6-020).
- Gap recording (M3-017).
- Any watering-safety meaning of a gap or of an incomplete replay. M3 owns
  **durable ingest and the transport acknowledgement**; M6 owns what the
  reconciled history permits. The split matters: an edge that acknowledged
  without persisting would satisfy this issue's shape and destroy M6's
  guarantee, because the device would have deleted the history M6 reasons
  about.

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

**The acknowledgement is the last step, and its order is the whole point.**
Receive → persist → commit → publish `event.ack`. A device's buffer is bounded
and an acknowledgement authorises it to delete; acknowledging on receipt, from
a write buffer, or optimistically before a commit that may still fail tells the
device to discard history the edge does not have. QoS 1 is not a substitute —
it reports the *broker's* acknowledgement, not the edge's.

`through_device_seq` is cumulative: the highest sequence such that everything at
or below it is committed. If batches commit out of order, acknowledge only up to
the last contiguous sequence and let the device replay the rest; a prefix that
skips a hole is a lie about what the edge holds. Set `boot_id` from the replay
being acknowledged — a device ignores an acknowledgement addressed to another
boot, and correctly so.

### Correction, post-M3

`device_seq` is zero-based, so the original `u64` acknowledgement could not tell
"sequence 0 is committed" from "nothing is committed" — both were 0. The commit
result is now `Option<u64>`: `None` means the edge publishes **no** `event.ack`
at all, and `Some(0)` is a real acknowledgement of sequence 0. A suffix-only
replay, where the device's buffer starts above anything the edge holds,
therefore commits its events and stays silent rather than telling the device to
discard sequence 0. Migration `0004_replay_progress_nullable.sql` makes
`replay_progress.through_device_seq` nullable to carry the same distinction.

**Consequence, recorded honestly.** An edge that has lost its `replay_progress`
while a device is still in the same boot will never acknowledge that device's
remaining buffer, because it has no prefix and protocol section 5.13 forbids
claiming one. The device replays indefinitely and eventually opens a
`history.gap`. That is the fail-safe direction — repeated replay rather than
deleted history — but it is a real operational edge, and closing it properly
needs a protocol-level "nothing acknowledged" or a resynchronisation exchange.

## Acceptance criteria

- [x] `through_device_seq` is `Option<u64>`; `None` publishes no acknowledgement and `Some(0)` acknowledges sequence 0.

- [x] A replayed batch is ingested and its events stored.
- [x] Replaying the same batch three times creates one row per `event_id`.
- [x] A batch split differently across messages still deduplicates correctly.
- [x] Autonomous doses are stored with `origin = 'offline_autonomous'`.
- [x] Replayed samples are stored with `origin = 'offline_replay'`.
- [x] `complete` is recorded so M6-020 can act on it.
- [x] An unacknowledged batch is safe to reprocess after an edge restart.
- [x] `event.ack` is published only after the transaction commits, and a
      simulated commit failure publishes none.
- [x] `through_device_seq` never exceeds the highest contiguous committed
      sequence, including when batches commit out of order.
- [x] `event.ack` is published non-retained; a retained one is a test failure.
- [x] The acknowledged `boot_id` is the one the replay carried.

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

- None. The wire mechanism is already normative in
  [mqtt-v1.md](../../protocol/mqtt-v1.md) §5.13, defined during the post-M2
  protocol seam cleanup rather than deferred to the implementing milestone.

## Files likely affected

```text
crates/edge-controller/src/pipeline/replay.rs
crates/storage/src/repo/replay.rs
```
