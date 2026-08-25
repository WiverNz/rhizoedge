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
- `DeviceConfig` with `config_version` and range validation
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

## Acceptance criteria

- [ ] Status, LWT, and config payloads round-trip.
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

## Documentation impact

- None.

## Files likely affected

```text
crates/mqtt-contract/src/payload/status.rs
crates/mqtt-contract/src/payload/config.rs
```
