# Issue M3-006 — Implement envelope decoding and received_at stamping

**Milestone:** M3 · **PRD:** [PRD 030](../../prd/030-edge-ingestion-and-storage.md) · **Depends on:** M3-005, M1-004

## Context

ADR-013: the edge stamps `received_at` from its own clock and every safety
computation uses it. Using the device timestamp would let a device with a
backwards clock make stale data look fresh, silently defeating SAFETY-005 three
milestones later.

## Goal

Decode inbound messages and stamp authoritative arrival time.

## Scope

- Parse the topic before the payload; drop unknown topics with a metric
- Decode the envelope; reject `v != 1`
- Reject topic/payload `device_id` and `kind` mismatches
- Stamp `received_at` from the edge `Clock`
- Map each `DecodeError` variant to its `mqtt_decode_errors_total` reason label

## Non-goals

- Quarantine storage (M3-007).
- Range validation handling (M3-009).

## Dependencies

- M3-005
- M1-004

## Implementation notes

Topic first, payload second: an unknown topic should not cost a JSON parse,
and a malformed payload on a known topic is a different condition from traffic
on a topic we do not handle.

The edge `Clock` is injected, so accelerated-time test topologies (M8) work
without special-casing.

Do not use `device_time_ms` for anything except storage as advisory data.

## Acceptance criteria

- [x] Valid messages decode with `received_at` from the edge clock.
- [x] `device_time_ms` is stored but never used for freshness.
- [x] `v: 2` is rejected with reason `version`.
- [x] A device mismatch is rejected with reason `device_mismatch`.
- [x] A kind/topic mismatch is rejected with reason `kind_mismatch`.
- [x] Unknown topics are dropped and counted, not quarantined.
- [x] Every `DecodeError` variant maps to a distinct metric label.

## Verification

```bash
cargo test -p edge-controller decode::
```

## Tests required

- One test per rejection reason.
- `received_at` comes from the injected clock.
- An explicit test that a device with a wrong clock does not affect `received_at`.

## Documentation impact

- None.

## Files likely affected

```text
crates/edge-controller/src/pipeline/decode.rs
```
