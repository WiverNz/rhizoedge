# Issue M11-005 — Implement the tank level sensor adapter

**Milestone:** M11 · **PRD:** [PRD 110](../../prd/110-real-pump-and-safety-hardware.md) · **Depends on:** M10-011

## Context

SAFETY-004. **`null` is Unknown, and Unknown is a refusal** — absence of a
reading is never permission to pump.

## Goal

Read a real reservoir level sensor.

## Scope

- `RealTankSensor` implementing the existing trait
- Float switch, ultrasonic, or resistive selectable by config
- Level as a percentage; **absent or failed reads publish `null`**
- The device refuses commands at or below `tank.min_percent`
- A float switch reports 0 or 100 only, documented as coarse

## Non-goals

- Flow measurement (M14).

## Dependencies

- M10-011

## Implementation notes

Do not interpolate a float switch. It is a binary sensor and presenting it
as a percentage between its two states would be invented data.

The stuck-float failure (failure-model 5.3) is undetectable in V1. The
mitigations are architectural: the daily cap bounds delivery, and choosing a
reservoir no larger than a day's safe delivery bounds the physical worst case.
Document that as a deployment recommendation.

## Acceptance criteria

- [ ] Level is read and published as a percentage.
- [ ] A failed read publishes **`null`**.
- [ ] The device refuses commands at or below the minimum, independently of the edge.
- [ ] A disconnected sensor produces `null` and a refusal.
- [ ] A float switch reports only 0 or 100.
- [ ] **`null` is treated as Unknown, never as full.**

## Verification

```bash
cd firmware/esp32-node && cargo test sensors::tank
cargo test safety_004
```

## Tests required

- Each sensor kind.
- **Null on failure.**
- Independent device refusal.

## Documentation impact

- Deployment note on reservoir sizing.

## Files likely affected

```text
firmware/esp32-node/src/sensors/tank/real.rs
```
