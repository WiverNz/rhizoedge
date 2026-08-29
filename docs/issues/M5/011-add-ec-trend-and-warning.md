# Issue M5-011 — Add EC recording, trend, and high warning

**Milestone:** M5 · **PRD:** [PRD 050](../../prd/050-plant-model-and-recommendations.md) · **Depends on:** M5-005

## Context

PRD 050 F-050-18. EC is an indicator of dissolved salts. It is recorded and
trended — and **no nutrient value is ever derived from it**.

## Goal

Track EC and warn on abnormal rises.

## Scope

- EC history exposed through the measurements API
- EC trend computed like the moisture trend
- A warning event above `ec.warning_high_us_cm`
- Correlation with watering events for display

## Non-goals

- **Deriving N/P/K from EC — permanently out of scope** (PRD 100, PRD 140).
- Any lockout based on EC.

## Dependencies

- M5-005

## Implementation notes

The non-goal is the important line here. Cheap NPK probes compute their
outputs from EC by an undisclosed formula; presenting them as nutrient
measurements would be a false claim. Record the trend, warn on anomalies, claim
nothing further.

EC is a warning, never a lockout. High salinity is a horticultural problem for a
human to solve, not a reason to refuse water.

## Acceptance criteria

- [x] EC history is available via the API.
- [x] A trend is computed with the same robustness rules as moisture.
- [x] Exceeding the threshold raises a warning event.
- [x] EC never triggers a lockout.
- [x] No code derives a nutrient value from EC.

## Verification

```bash
cargo test -p rhizo-domain ec::
```

## Tests required

- Trend computation.
- Warning threshold.
- An explicit test that EC does not affect the gate.

## Documentation impact

- None.

## Files likely affected

```text
crates/domain/src/ec.rs
```
