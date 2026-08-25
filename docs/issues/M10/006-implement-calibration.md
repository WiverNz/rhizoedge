# Issue M10-006 — Implement sensor calibration

**Milestone:** M10 · **PRD:** [PRD 100](../../prd/100-real-soil-sensor.md) · **Depends on:** M10-004, M10-005

## Context

PRD 100 F-100-14: **an uncalibrated sensor publishes `null`**, not a raw value
scaled to look like a percentage. A plausible wrong number is far more dangerous
than an absent one, because the lockout never fires.

## Goal

Map raw readings to physical units, or refuse.

## Scope

- Two-point calibration (`raw_dry`, `raw_wet`) to a linear VWC mapping
- Calibration stored in device config, versioned
- **Uncalibrated publishes `null`** and raises `calibration_missing`
- Out-of-calibrated-range readings clamped **and flagged**, never extrapolated
- Optional linear temperature compensation, default zero

## Non-goals

- Automatic calibration.

## Dependencies

- M10-004
- M10-005

## Implementation notes

The refuse-when-uncalibrated rule is the safety-relevant one. A device
shipping raw ADC counts as `moisture_vwc` would produce numbers that pass every
range check while meaning nothing, and automatic watering would act on them.

Clamp-and-flag rather than extrapolate: outside the calibrated range the linear
model has no support, and extrapolating produces confident nonsense.

## Acceptance criteria

- [ ] Two-point calibration produces correct VWC for known raw values.
- [ ] **An uncalibrated sensor publishes `null`.**
- [ ] `calibration_missing` is raised.
- [ ] Out-of-range readings are clamped and flagged.
- [ ] Temperature compensation is optional and defaults to no effect.
- [ ] Calibration survives a reboot.

## Verification

```bash
cd firmware/esp32-node && cargo test calibration::
```

## Tests required

- Mapping accuracy.
- **Uncalibrated publishes null.**
- Clamp-and-flag at range edges.
- Persistence.

## Documentation impact

- Calibration procedure.

## Files likely affected

```text
firmware/esp32-node/src/sensors/calibration.rs
```
