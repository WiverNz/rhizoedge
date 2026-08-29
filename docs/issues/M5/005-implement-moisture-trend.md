# Issue M5-005 — Implement moisture trend computation

**Milestone:** M5 · **PRD:** [PRD 050](../../prd/050-plant-model-and-recommendations.md) · **Depends on:** M1-012

## Context

PRD 050 F-050-11: a trend must be `None` rather than a fabricated slope from
two noisy points. A confident wrong trend is worse than an absent one.

## Goal

Compute a robust moisture slope, or nothing.

## Scope

- Least-squares slope in %VWC/hour over a configurable window (default 6 h)
- **`None` with fewer than 5 valid samples**
- `None` when samples are too sparse across the window
- Invalid samples excluded before fitting
- A pure function taking samples and a window

## Non-goals

- Using the trend for a safety decision — it informs recommendations only.

## Dependencies

- M1-012

## Implementation notes

Sparsity matters as much as count: five samples clustered in the last two
minutes of a six-hour window describe two minutes, not six hours. Require
coverage across the window, not merely a count.

Noise is real (the simulator adds it by default), so the fit must be
least-squares rather than an endpoint difference.

## Acceptance criteria

- [x] A known falling series produces a negative slope of the expected magnitude.
- [x] Four samples return `None`.
- [x] Five clustered samples return `None` (sparsity rule).
- [x] Invalid samples are excluded.
- [x] Noise does not flip the sign of a clear trend.
- [x] The function is pure.

## Verification

```bash
cargo test -p rhizo-domain trend::
```

## Tests required

- Known series.
- Insufficient count.
- Sparsity rejection.
- Invalid exclusion.
- Property: sign stability under noise.

## Documentation impact

- None.

## Files likely affected

```text
crates/domain/src/trend.rs
```
