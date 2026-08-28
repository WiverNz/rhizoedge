# Issue M4-004 — Implement sample staleness and the liveness timer

**Milestone:** M4 · **PRD:** [PRD 040](../../prd/040-device-registry-and-health.md) · **Depends on:** M4-001

## Context

ADR-013: `max_sample_age = max(15 min, 3 x telemetry_interval)`, computed from
`received_at`. PRD 040 F-040-09 adds that liveness must be evaluated by a timer,
because a device that stops publishing produces no message to react to.

## Goal

Compute staleness correctly and detect silent devices.

## Scope

- `sample_age_seconds` derived at read time from `last_seen_at`
- The threshold formula with its 15-minute floor
- A periodic timer emitting stale events and updating gauges
- The same timer republishes `edge.time` to every online device at least every `TIME_SYNC_INTERVAL_SECONDS` (F-040-18)
- Staleness **derived, never stored**

## Non-goals

- Lockouts (M6-005).

## Dependencies

- M4-001

## Implementation notes

Derived rather than stored is deliberate: a stored flag needs a writer, and a
writer that fails leaves a wrong flag. Computing it at read time means it cannot
be stale about staleness.

The timer exists only to emit events and update metrics; the authoritative
answer is always computed.

Use `received_at`, never `device_time_ms` — this is the requirement SAFETY-005
depends on downstream.

## Acceptance criteria

- [x] `sample_age_seconds` is computed from `received_at`.
- [x] The threshold is `max(15 min, 3 x interval)`.
- [x] A device configured with a 10-second interval uses the 15-minute floor.
- [x] A device that stops publishing while connected is detected **by the timer**.
- [x] No `stale` column exists in the schema.
- [x] Every online device receives an `edge.time` at least every 300 s.
- [x] A device with a badly wrong clock still reports correct staleness.

## Verification

```bash
cargo test -p edge-controller staleness::
cargo test --test integration stale_without_inbound_message
```

## Tests required

- Threshold arithmetic including the floor.
- Timer detection with no inbound message.
- Wrong device clock does not affect the result.
- SCEN-075 aged-out time sync refuses commands.
- SCEN-078 periodic refresh keeps a long-connected device synced.

## Documentation impact

- None.

## Files likely affected

```text
crates/edge-controller/src/device/health.rs
```
