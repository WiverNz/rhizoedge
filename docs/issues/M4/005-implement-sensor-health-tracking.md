# Issue M4-005 — Implement sensor health tracking

**Milestone:** M4 · **PRD:** [PRD 040](../../prd/040-device-registry-and-health.md) · **Depends on:** M4-001

## Context

PRD 040 F-040-06 requires distinguishing 'the sensor reported an invalid
value' from 'there is no such sensor' from 'the sensor is fine'. All three are
lockout-relevant in M6 and must not be conflated.

## Goal

Track per-sensor presence and health from status messages.

## Scope

- Store the sensors block as JSON on `devices`
- Expose presence, health, and error count per sensor
- Correlate with `sensor_invalid` events from ingestion
- Expose in the device API

## Non-goals

- Per-sensor history (M10).

## Dependencies

- M4-001

## Implementation notes

A JSON column rather than a table: it is a small whole-value snapshot from
the latest status with no independent lifecycle. Revisit in M10 when real
sensors produce real fault histories.

The three-way distinction is the point. `present: false` means the sensor was
never fitted; `healthy: false` means it is fitted and broken. M6 treats both as
lockouts but the operator needs to tell them apart.

## Acceptance criteria

- [ ] Sensor health is recorded from status messages.
- [ ] Presence, health, and error count are exposed per sensor.
- [ ] An absent sensor is distinguishable from an unhealthy one.
- [ ] Ingestion-side `sensor_invalid` events are visible alongside.

## Verification

```bash
cargo test -p edge-controller device::sensors
curl -s localhost:8080/api/v1/devices/plant-node-01 | jq .sensors
```

## Tests required

- Health recording.
- Absent versus unhealthy distinction.

## Documentation impact

- None.

## Files likely affected

```text
crates/edge-controller/src/device/sensors.rs
```
