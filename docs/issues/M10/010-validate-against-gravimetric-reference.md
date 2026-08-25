# Issue M10-010 — Validate readings against a gravimetric reference

**Milestone:** M10 · **PRD:** [PRD 100](../../prd/100-real-soil-sensor.md) · **Depends on:** M10-009

## Context

PRD 100 F-100-30 through F-100-32. **The test that decides whether the numbers
mean anything.** Without it, the system reports confident values of unknown
validity.

## Goal

Establish documented error bounds for the sensor.

## Scope

- Gravimetric reference at three moisture levels (oven-dry, field capacity, saturated)
- Compare sensor readings against computed VWC
- Document error bounds
- Observe drift over at least four weeks
- Feed the bounds into recommendation `confidence`

## Non-goals

- Laboratory certification.

## Dependencies

- M10-009

## Implementation notes

Procedure: weigh a soil sample wet, oven-dry it, weigh again, compute
volumetric water content from the mass difference and the sample volume. Compare
against what the probe reported at the wet weighing.

Four weeks of drift observation is the minimum useful window for a capacitive
probe, where junction corrosion is the expected slow failure.

If the error bounds turn out to be wide, that is a finding to document, not a
reason to withhold the numbers — but it must change how much the recommendation
engine trusts them.

## Acceptance criteria

- [ ] Readings compared against gravimetric reference at three levels.
- [ ] Error bounds documented with the raw measurements.
- [ ] Four weeks of drift observation recorded.
- [ ] Bounds inform recommendation confidence.
- [ ] The procedure is documented for repetition.
- [ ] Results are recorded in `docs/testing/hil-runs/`.

## Verification

```bash
# manual laboratory procedure; results recorded in docs/testing/hil-runs/
```

## Tests required

- Manual validation; the record is the artefact.

## Documentation impact

- Validation procedure and results.

## Files likely affected

```text
docs/testing/hil-runs/sensor-validation.md
```
