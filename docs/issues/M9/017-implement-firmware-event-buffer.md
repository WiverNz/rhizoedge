# Issue M9-017 — Implement the bounded event buffer with tiered retention

**Milestone:** M9 · **PRD:** [PRD 090](../../prd/090-esp32-rust-firmware.md) · **Depends on:** M9-016

## Context

[ADR-014](../../adr/014-failure-and-retry-policy.md): a bounded NVS ring where
audit events outrank telemetry and eviction is reported as an explicit gap
(SAFETY-020).

## Goal

Buffer history on the device and replay it idempotently.

## Scope

- Bounded NVS ring with `audit` and `telemetry` tiers
- Audit events never evicted to make room for telemetry
- `event_id` generated once at buffering time, never regenerated on replay
- `device_seq` monotonic within `boot_id`
- Eviction emits `history.gap` with range, count, and lost tier
- Replay in order, in batches, `complete: true` on the last, retained until acknowledged
- Buffer survives reboot

## Non-goals

- Edge-side ingestion (M3-016).

## Dependencies

- M9-016

## Implementation notes

Flash wear matters here more than for policies: telemetry buffering writes
often. Use a ring with in-place slot reuse rather than rewriting the whole region,
and consider buffering telemetry in RAM with periodic NVS flush while keeping
audit events written through immediately. Audit durability is worth the wear;
telemetry durability is not.

`event_id` stability across replay is the property SAFETY-016 depends on.
Generate at buffering time and store it — never derive it at publish time.

## Acceptance criteria

- [ ] Events buffer while isolated and replay on reconnection.
- [ ] `event_id` is identical across repeated replays.
- [ ] Audit events survive a telemetry flood.
- [ ] Overflow emits a gap with the correct range and count.
- [ ] The buffer survives a reboot.
- [ ] Unacknowledged events are replayed again.
- [ ] Audit events are durable across power loss; telemetry may be lost.

## Verification

```bash
cd firmware/esp32-node && cargo test buffer::
cargo test safety_016 safety_020
```

## Tests required

- Tier retention property test.
- `event_id` stability.
- Gap emission.
- Reboot survival.

## Documentation impact

- None.

## Files likely affected

```text
firmware/esp32-node/src/app/buffer.rs
```
