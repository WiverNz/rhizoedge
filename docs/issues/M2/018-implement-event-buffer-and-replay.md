# Issue M2-018 — Implement the offline event buffer and replay

**Milestone:** M2 · **PRD:** [PRD 020](../../prd/020-device-simulator.md) · **Depends on:** M2-017

## Context

[ADR-014](../../adr/014-failure-and-retry-policy.md) specifies a bounded ring with
tiered retention and explicit gap reporting. The simulator models the
device-side mechanics before autonomous event production is activated by
M6-019. Tests may inject typed events into the ring; M6 later supplies real
autonomous outcomes through the same buffer API.

## Goal

Buffer history while isolated and replay it idempotently on reconnection.

## Scope

- Bounded ring in the state file with `audit` and `telemetry` tiers
- Audit events never evicted to make room for telemetry
- `event_id` generated once at buffering time; **never** regenerated on replay
- `device_seq` monotonic within `boot_id`
- Eviction emits a `history.gap` event with range, count, and lost tier
- Replay in `device_seq` order, in batches, `complete: true` on the last
- Events retained until the edge acknowledges them

## Non-goals

- Edge-side reconciliation (M3-016, M6-020).
- Offline evaluation or autonomous event decisions (M6-019).

## Dependencies

- M2-017

## Implementation notes

Generating `event_id` at buffering time rather than at publish time is the whole
mechanism. A device that regenerates on replay defeats deduplication and creates
duplicate watering history — SAFETY-016's central failure.

Size the ring so a realistic isolation (hours) overflows telemetry but not audit.
SCEN-104 needs overflow to be reachable in a test without waiting for days of
virtual time.

## Acceptance criteria

- [ ] Events buffer while disconnected and replay on reconnect.
- [ ] `event_id` is byte-identical across repeated replays.
- [ ] Audit events survive a telemetry flood.
- [ ] Overflow emits a `history.gap` with correct range and count.
- [ ] The final batch sets `complete: true`.
- [ ] Unacknowledged events are retained and replayed again.
- [ ] Replaying three times produces one logical event per `event_id`.

## Verification

```bash
cargo test -p device-simulator buffer::
cargo test safety_016
cargo test safety_020
```

## Tests required

- SCEN-100, SCEN-101, SCEN-104.
- Tier retention property test.

## Documentation impact

- None.

## Files likely affected

```text
crates/device-simulator/src/buffer.rs
```
