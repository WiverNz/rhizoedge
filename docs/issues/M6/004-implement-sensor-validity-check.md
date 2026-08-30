# Issue M6-004 — Implement the sensor validity gate check

**Milestone:** M6 · **PRD:** [PRD 060](../../prd/060-irrigation-control-and-safety.md) · **Depends on:** M6-002

## Context

SAFETY-005 in part: a sample that is absent, out of range, NaN, or from a
sensor marked stuck or unhealthy cannot drive automatic watering.

## Goal

Reject invalid sensor state before any irrigation logic runs.

## Scope

- Absent sample: `SensorFault`
- Invalid sample (range, NaN): `SensorFault`
- Sensor marked unhealthy or stuck: `SensorFault`
- Sensor absent entirely (`--sensors` omission): `SensorFault`
- Auto-clears when a valid sample arrives

## Non-goals

- Staleness, which is a separate check (M6-005).

## Dependencies

- M6-002

## Implementation notes

Keep validity and staleness as **separate** reasons. 'The sensor is broken'
and 'the sensor has not reported recently' need different operator responses,
and collapsing them into one lockout reason makes the UI unhelpful.

A missing sensor and a broken sensor both lock out, but the device API
distinguishes them (M4-005) so the operator can tell.

## Acceptance criteria

- [x] An absent sample yields `SensorFault`.
- [x] An out-of-range or NaN sample yields `SensorFault`.
- [x] A stuck sensor yields `SensorFault`.
- [x] `SensorFault` and `StaleData` are distinct reasons.
- [x] A valid sample clears it automatically.

## Verification

```bash
cargo test -p rhizo-domain gate::validity
cargo test safety_005
```

## Tests required

- Each invalid condition.
- Distinctness from StaleData.
- Auto-clear.

## Documentation impact

- None.

## Files likely affected

```text
crates/domain/src/irrigation/gate.rs
```
