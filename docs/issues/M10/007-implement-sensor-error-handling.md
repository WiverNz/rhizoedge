# Issue M10-007 — Implement sensor error handling and health

**Milestone:** M10 · **PRD:** [PRD 100](../../prd/100-real-soil-sensor.md) · **Depends on:** M10-006

## Context

PRD 100 F-100-20 through F-100-25. Distinct error kinds, distinct counters,
and a health state that reaches the edge as a lockout.

## Goal

Handle sensor failures without stalling telemetry.

## Scope

- Timeout, CRC, exception, range, and stuck errors counted separately
- 3 consecutive failures mark the sensor unhealthy in status
- Stuck detection on **raw** readings
- A bus error does not stall the telemetry loop
- Recovery clears the unhealthy state

## Non-goals

- Deciding what unhealthy means — that is the edge's gate (M6-004).

## Dependencies

- M10-006

## Implementation notes

Detecting stuck on raw rather than calibrated readings matters: calibration
can map a range of raw values to the same displayed value, masking a genuinely
frozen sensor.

The telemetry loop must survive a bus that never responds. A blocking read
without a timeout would take the whole device down.

## Acceptance criteria

- [ ] Each error kind increments its own counter.
- [ ] 3 consecutive failures mark the sensor unhealthy.
- [ ] Stuck detection operates on raw readings.
- [ ] A persistently failing bus does not stall telemetry.
- [ ] Recovery clears the unhealthy state.
- [ ] Unhealthy state reaches the edge and produces `SensorFault`.

## Verification

```bash
cd firmware/esp32-node && cargo test sensors::errors
```

## Tests required

- Each error path.
- Health transitions.
- Stuck on raw.
- Telemetry survives a dead bus.

## Documentation impact

- None.

## Files likely affected

```text
firmware/esp32-node/src/sensors/health.rs
```
