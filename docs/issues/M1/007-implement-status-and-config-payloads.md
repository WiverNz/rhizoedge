# Issue M1-007 — Implement device status, LWT, and config payloads

**Milestone:** M1 · **PRD:** [PRD 010](../../prd/010-domain-and-mqtt-protocol.md) · **Depends on:** M1-004

## Context

Protocol sections 5.5-5.7 define the retained status message, the Last Will
payload, and the device config. The status message reports the firmware's hard
limits for observability — one-way reporting, never a channel to change them.

## Goal

Implement the status, LWT, and config payload types with their validation.

## Scope

- `DeviceStatus` with sensors map, limits, uptime, heap, RSSI, `applied_config_version`
- The LWT payload shape
- `DeviceConfig` with `config_version` and range validation — **no time-server field**; the device's clock comes from the Edge over MQTT
- `EdgeTime` payload: a single `edge_time_ms` integer, published on the live `time` topic ([mqtt-v1.md](../../protocol/mqtt-v1.md) §5.12)
- `TIME_SYNC_INTERVAL_SECONDS = 300` and `TIME_SYNC_MAX_AGE_SECONDS = 1800` as compile-time constants, **not configurable**
- `TimeSyncState` — a pure, `no_std` holder of `last_applied_edge_time_ms` and
  `synced_at_monotonic`, with `apply(edge_time_ms, monotonic_now) -> bool` and
  `is_synced(monotonic_now) -> bool`. **Strictly** increasing acceptance
- Config validation rejecting out-of-range values
- Unknown config fields ignored

## Non-goals

- Publishing config (M6-013).
- Applying config (M2-003, M9-012).

## Dependencies

- M1-004

## Implementation notes

`DeviceConfig` must **not** contain any safety limit field. Ignoring unknown
fields means an attempt to smuggle `max_ml_per_run` through the config topic has
no effect — which is the desired outcome, but assert it with a test so the
property is checked rather than assumed.

Config ranges: `telemetry_interval_seconds` 10-3600,
`pump.ml_per_second` 0.1-100.0, `tank.min_percent` 0.0-100.0.

The `limits` block in status is read-only reporting. Give it no setter and no
corresponding config field.

`TimeSyncState` lives here for the same reason `validate_water_command` does: the
simulator and the firmware must not each write the rule. The rule is
`edge_time_ms > last_applied_edge_time_ms` — **strictly** greater. Writing `>=`
would let a QoS 1 duplicate, redelivered indefinitely, refresh
`synced_at_monotonic` and hold `clock_synced` true forever without the device ever
receiving a newer Edge timestamp. A rejected value must update nothing at all.
`apply` takes monotonic time as a parameter and reads no clock.

## Acceptance criteria

- [ ] Status, LWT, config, and `edge.time` payloads round-trip.
- [ ] `EdgeTime` carries exactly one field; there is no round-trip or offset field to misuse.
- [ ] `TimeSyncState::apply` accepts a **strictly** newer `edge_time_ms` and returns true.
- [ ] It rejects an older value **and an equal value**, returning false and leaving
      `synced_at_monotonic` untouched.
- [ ] `is_synced` is false once `TIME_SYNC_MAX_AGE_SECONDS` has elapsed on the
      monotonic clock, regardless of how many messages were rejected meanwhile.
- [ ] The config type has **no** time-server field, and one in the JSON is ignored.
- [ ] Config values outside their ranges are rejected.
- [ ] A config containing `max_ml_per_run` decodes successfully and the field is **absent** from the resulting type.
- [ ] `status` accepts only `online` and `offline`.
- [ ] The sensors map handles absent sensors.

## Verification

```bash
cargo test -p rhizo-mqtt-contract payload::status payload::config
```

## Tests required

- Round trips.
- Config range rejections.
- The smuggled-limit test asserting the field cannot be represented.
- `safety_002_stale_time_sync_never_applied`.
- `safety_002_duplicate_time_sync_does_not_extend_validity` — replay one value
  many times, advance past the max age, assert `is_synced` is false.

## Documentation impact

- None.

## Files likely affected

```text
crates/mqtt-contract/src/payload/status.rs
crates/mqtt-contract/src/payload/config.rs
crates/mqtt-contract/src/payload/time.rs
```
