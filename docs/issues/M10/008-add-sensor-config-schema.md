# Issue M10-008 — Add the sensor configuration schema

**Milestone:** M10 · **PRD:** [PRD 100](../../prd/100-real-soil-sensor.md) · **Depends on:** M10-007, M6-013

## Context

Sensor kind, register map, calibration, and bus parameters are device config
(ADR-011 layer L3), versioned and delivered retained.

## Goal

Extend the device config with sensor settings.

## Scope

- `soil_sensor` block: kind, slave address, register map name, calibration
- Bus parameters: baud, parity, turnaround
- Validation with rejection of impossible values
- Config changes applied without a reboot where possible
- **Still no safety limit fields**

## Non-goals

- Cloud-pushed config — not in V1.

## Dependencies

- M10-007
- M6-013

## Implementation notes

Additive to the config payload, so it is a non-breaking v1 change
(versioning-policy section 1): older firmware ignores the new block.

Applying without a reboot is a convenience, not a requirement — a calibration
change that needs a restart is acceptable if it is reported honestly.

## Acceptance criteria

- [ ] The sensor block is accepted and applied.
- [ ] Invalid values are rejected and the previous config retained.
- [ ] Older firmware ignoring the block still works.
- [ ] Calibration changes take effect as documented.
- [ ] The config still contains no safety limit field.

## Verification

```bash
cd firmware/esp32-node && cargo test config::sensor
curl -X PUT localhost:8080/api/v1/devices/plant-node-01/config -d @sensor-config.json
```

## Tests required

- Application.
- Validation.
- Backward compatibility.

## Documentation impact

- protocol/mqtt-v1.md config section extended additively.

## Files likely affected

```text
firmware/esp32-node/src/app/config.rs
crates/mqtt-contract/src/payload/config.rs
```
