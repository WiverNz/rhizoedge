# Issue M10-006 — Implement sensor calibration

> **SEN0601 implementation:** Configure its documented units/scaling and
> calibration provenance first, checking repeatability in intended soil. Do not
> impose an analogue raw-dry/raw-wet procedure on factory-scaled registers or
> claim laboratory VWC accuracy. Missing calibration/conversion information
> stays invalid. The analogue two-point and optional compensation work below
> is deferred, superseded for this slice by PRD 100 prototype scope.


> **Execution scope, 2026-11-22:** **Status: PLANNED — prototype slice.** Apply the current milestone PRD prototype scope and [testing policy](../../testing/strategy.md). Only relevant tests and a basic smoke are required; extended campaigns below are backlog reference, not mandatory local reruns. See [ROADMAP](../../../ROADMAP.md).


**Milestone:** M10 · **PRD:** [PRD 100](../../prd/100-real-soil-sensor.md) · **Depends on:** M10-004

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

## Implementation notes

The refuse-when-uncalibrated rule is the safety-relevant one. A device
shipping raw ADC counts as `moisture_vwc` would produce numbers that pass every
range check while meaning nothing, and automatic watering would act on them.

Clamp-and-flag rather than extrapolate: outside the calibrated range the linear
model has no support, and extrapolating produces confident nonsense.

## Acceptance criteria

Apply the prototype subset specified in the milestone PRD. Leave any broader
criterion unchecked and explicitly deferred; completion of this slice does not
claim the deferred full scope passed.

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
