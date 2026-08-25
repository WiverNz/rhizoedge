# Issue M4-002 — Implement Last Will handling

**Milestone:** M4 · **PRD:** [PRD 040](../../prd/040-device-registry-and-health.md) · **Depends on:** M4-001

## Context

Protocol section 5.6. The LWT payload is fixed at connect time, so its
`message_id` is generated then — and may be seen twice if a device reconnects
and disconnects on the same will.

## Goal

Mark devices offline correctly when the broker publishes their will.

## Scope

- Recognise `status: offline` with `reason: connection_lost`
- Mark the device offline and freeze `last_seen_at`
- Record an `offline` device event
- Tolerate a repeated LWT `message_id` via the existing dedup path
- Distinguish `connection_lost` from `shutdown`

## Non-goals

- Plant lockouts (M6).

## Dependencies

- M4-001

## Implementation notes

The repeated-`message_id` case is handled correctly by M3-008's dedup: the
second delivery is a duplicate and is ignored, which is the right outcome. Add a
test making that explicit rather than leaving it as an inference.

`shutdown` is a clean disconnect and is informational; `connection_lost` is
worth a warning.

## Acceptance criteria

- [ ] Killing the simulator marks the device offline within the keepalive window.
- [ ] An `offline` device event is recorded.
- [ ] `last_seen_at` stops advancing.
- [ ] A repeated LWT `message_id` is deduplicated with no second event.
- [ ] `shutdown` and `connection_lost` are distinguished in the event detail.

## Verification

```bash
cargo test --test integration lwt_offline
```

## Tests required

- SCEN-020.
- Repeated LWT message_id deduplicated.
- Clean shutdown versus connection loss.

## Documentation impact

- None.

## Files likely affected

```text
crates/edge-controller/src/pipeline/status.rs
```
