# Issue M11-010 — Run HIL-3 pump calibration

**Milestone:** M11 · **PRD:** [PRD 110](../../prd/110-real-pump-and-safety-hardware.md) · **Depends on:** M11-009, M11-004

## Context

First water in the system, into a measuring cup. Establishes the
`ml_per_second` everything downstream depends on.

## Goal

Calibrate delivery and verify accuracy.

## Scope

- Prime the line; confirm no leaks at any joint
- Five 10-second runs with measured volumes
- Mean and standard deviation computed
- `ml_per_second` stored
- A 40 ml request verified by measurement
- All raw measurements recorded

## Non-goals

- Any plant.

## Dependencies

- M11-009
- M11-004

## Implementation notes

Recording all five raw measurements, not just the mean, is what makes drift
detectable in a year's time. The historical record is the artefact.

σ above 5% of the mean means investigate rather than average: air lock,
occlusion, or a failing pump head.

## Acceptance criteria

- [ ] Five runs recorded with raw volumes.
- [ ] σ is below 5% of the mean.
- [ ] `ml_per_second` is stored in device config.
- [ ] A 40 ml request delivers within ±10%, measured.
- [ ] No leaks at any joint.
- [ ] The calibration date is recorded.

## Verification

```bash
# manual: HIL-3 checklist with a measuring cup
```

## Tests required

- The HIL-3 checklist.

## Documentation impact

- hil-runs record with all raw measurements.

## Files likely affected

```text
docs/testing/hil-runs/
```
