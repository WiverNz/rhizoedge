# Issue M10-004 — Implement the Modbus soil sensor adapter

**Milestone:** M10 · **PRD:** [PRD 100](../../prd/100-real-soil-sensor.md) · **Depends on:** M10-003

## Context

The strategic sensor path: moisture, temperature, and EC from an RS485
probe.

## Goal

Read a real soil probe over Modbus.

## Scope

- `ModbusSoilSensor` implementing `SoilSensor`
- Moisture, temperature, and EC via the register map
- Calibration applied (M10-006)
- Read failure publishes `null`, **never a stale or default value**

## Non-goals

- Analogue sensors (M10-005).

## Dependencies

- M10-003

## Implementation notes

Publishing `null` rather than the last good value on a read failure is what
makes upstream staleness and stuck detection work. Repeating a cached value
would make a dead sensor look alive — the exact condition SAFETY-005 exists to
catch.

## Acceptance criteria

- [ ] Readings flow from a mock responder through to telemetry.
- [ ] A read failure publishes `null`.
- [ ] **No stale or default value is ever published.**
- [ ] Absent optional fields (no EC register) are omitted cleanly.
- [ ] Host tests cover it with a fake UART.

## Verification

```bash
cd firmware/esp32-node && cargo test sensors::modbus_soil
```

## Tests required

- Successful read.
- **Failure publishes null.**
- Optional field absence.

## Documentation impact

- None.

## Files likely affected

```text
firmware/esp32-node/src/sensors/soil/modbus.rs
```
