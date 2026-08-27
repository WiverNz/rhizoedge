# Issue M3-007 — Implement bounded, rate-limited message quarantine

**Milestone:** M3 · **PRD:** [PRD 030](../../prd/030-edge-ingestion-and-storage.md) · **Depends on:** M3-006, M3-003

## Context

Protocol section 10 requires malformed messages to be quarantined for
inspection. Failure-model 1.5 adds the constraint that a device flooding
malformed payloads must not fill the disk.

## Goal

Store malformed messages for inspection without unbounded growth.

## Scope

- Write to `quarantined_messages` with topic, first 1 KiB of payload, error, timestamp
- Cap at 1000 rows, evicting oldest
- Rate limit: 10 quarantine writes per minute per device
- Rate-limited messages counted but not stored
- Pipeline continues after a quarantine

## Non-goals

- The quarantine API endpoint (M4-008).

## Dependencies

- M3-006
- M3-003

## Implementation notes

The payload truncation and the rate limit are both required — a 10 MB
malformed payload published once, or a 200-byte one published 1000 times a
second, would each fill the disk without them.

The most important behaviour: after quarantining, the **next valid message is
processed normally**. A quarantine that wedges the pipeline turns one bad
message into an outage.

## Acceptance criteria

- [x] Invalid JSON is quarantined with its topic and error.
- [x] Payloads are truncated to 1 KiB.
- [x] The table never exceeds 1000 rows.
- [x] More than 10 malformed messages per minute from one device are counted but not stored.
- [x] A valid message following a quarantined one is processed normally.
- [x] `mqtt_decode_errors_total` increments regardless of rate limiting.

## Verification

```bash
cargo test -p edge-controller quarantine::
cargo test --test integration malformed_payload
```

## Tests required

- Truncation.
- Row cap eviction.
- Rate limiting.
- SCEN-014: pipeline continues after a quarantine.

## Documentation impact

- None.

## Files likely affected

```text
crates/edge-controller/src/pipeline/quarantine.rs
crates/storage/src/repo/quarantine.rs
```
