# Issue M15-004 — Implement the deterministic drying-rate estimator

**Milestone:** M15 · **PRD:** [PRD 150](../../prd/150-per-plant-adaptive-water-model.md) · **Depends on:** M15-003

## Context

With clean segments available, the first estimate becomes possible: how fast
does *this* pot lose moisture. `trend::fit` already establishes the house style
for this kind of function — least squares rather than endpoint difference,
`None` rather than a fabricated slope, purity, and an anchor supplied by the
caller — and this estimator is its longer-horizon sibling.

## Goal

`hydration::estimate_drying_rate` — pure, deterministic, recency-weighted,
outlier-resistant, and answering `None` whenever it cannot answer honestly.

## Scope

- Weighted least squares over accepted segments of the current epoch, weighting
  each by an exponential recency term with `OBSERVATION_HALF_LIFE_DAYS` and by
  its own sample count.
- Median-absolute-deviation outlier rejection at `MAD_OUTLIER_K`, returning the
  rejected observations to the caller so they can be marked in storage.
- `None` when fewer than `MIN_SEGMENTS` accepted observations remain, when the
  residual spread exceeds its ceiling, or when any intermediate is non-finite.
- `DryingRate { vwc_per_day, spread, n, newest_age }`.
- `hydration::expected_dry_at(now, vwc, target_min, &DryingRate)`.

## Non-goals

- Environmental normalisation. Recorded, unused; PRD 150 §Open questions 1.
- Any confidence value. M15-007.
- Any consumption of the estimate. M15-008 onward.

## Dependencies

- M15-003

## Implementation notes

`expected_dry_at` must answer `None` for a non-negative rate. A pot that is
getting wetter has no dry-threshold crossing, and projecting one produces either
an instant in the past or an arithmetic overflow — both worse than "unknown".

Weight by observation **age**, computed from the `now` the caller passes, and
never from a clock read inside the function. Two runs at the same `now` over the
same observations must produce identical `f64` output, bit for bit; that is what
makes M15-014's replay criterion testable rather than approximate.

Reject outliers by MAD rather than by standard deviation: one repotting-shaped
segment that escaped the epoch machinery would drag a mean-based rule with it,
which is exactly the failure this rule exists to survive.

Return rejections rather than swallowing them. An estimator that silently
discards half its input and reports high confidence is the specific failure
SAFETY-022's confidence gate cannot catch.

## Acceptance criteria

- [ ] A clean synthetic history recovers the injected slope within tolerance.
- [ ] Recency weighting moves the estimate toward recent behaviour when the
      history contains a genuine change.
- [ ] An injected outlier is rejected and named in the return value.
- [ ] Fewer than `MIN_SEGMENTS` observations answers `None`.
- [ ] Excess residual spread answers `None`.
- [ ] No input produces `NaN` or an infinity.
- [ ] `expected_dry_at` answers `None` for a non-negative rate and for a
      non-finite reading.
- [ ] Two runs over the same input at the same `now` are bit-identical.

## Verification

```bash
cargo test -p rhizo-domain hydration::drying
cargo test -p rhizo-domain hydration::prop
```

## Tests required

- Property: output is finite or `None`, for arbitrary generated histories
  including empty, single-observation, zero-span, and extreme values.
- Property: adding an observation identical to the weighted centre does not move
  the estimate beyond tolerance.
- Determinism across repeated evaluation.
- The documented worked example from PRD 150 reproduces its stated numbers.

## Documentation impact

- PRD 150 §First deterministic model, if the accepted rule deviates.

## Files likely affected

```text
crates/domain/src/hydration/drying.rs
crates/domain/src/hydration/mod.rs
```
