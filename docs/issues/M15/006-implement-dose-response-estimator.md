# Issue M15-006 — Implement the dose-response estimator

**Milestone:** M15 · **PRD:** [PRD 150](../../prd/150-per-plant-adaptive-water-model.md) · **Depends on:** M15-005

## Context

The second of the two estimates: how much measured moisture one millilitre buys
in this pot. It is what turns "the plant is dry" into "28 ml", and it is the
half that touches actuation, so it is the half whose refusals matter most.

## Goal

`hydration::estimate_dose_response` — pure, deterministic, weighted,
outlier-resistant, fitted through the origin, and answering `None` rather than a
number it cannot support.

## Scope

- Weighted least squares of `rise_vwc` against millilitres, **through the
  origin**, over accepted observations of the current epoch.
- Recency weighting shared with M15-004, plus a `0.5` factor for
  `verified = false` observations.
- MAD outlier rejection, returning rejections to the caller.
- `None` below `MIN_RESPONSES`, above the residual-spread ceiling, on a
  non-positive fitted slope, or on any non-finite intermediate.
- `DoseResponse { vwc_per_ml, spread, n, unverified_n }`.
- `hydration::propose_dose(vwc, target_recovery_vwc, &DoseResponse)`.

## Non-goals

- Clamping. M15-012 owns the clamp, in one place, and this function must not
  pre-empt it — two clamps is how one of them stops being tested.
- Saturation curves. Real absorption is not linear all the way up, but the
  observations available in V1 span one plant's configured dose, over which
  linear-through-origin is both adequate and explainable.

## Dependencies

- M15-005

## Implementation notes

Through the origin, with no intercept, for a physical reason worth writing in
the doc comment: an intercept lets the fit claim a non-zero rise from a zero
dose, which is false, and its most likely cause is contamination the extractor
should have caught.

A fitted slope `<= 0` answers `None`, not a clamped small positive. A pot whose
measured response to water is zero or negative has something wrong with it —
disconnected tube, probe outside the wetting front, a sensor fault — and the
existing `NoDeliveryDetected` lockout is the mechanism that should be speaking,
not this estimator.

`propose_dose` returns a raw, unclamped `f32` and is documented as unsafe to use
directly. M15-012's `clamp_proposal` is the only permitted consumer, and
M15-014 asserts the call graph.

## Acceptance criteria

- [ ] A clean synthetic history recovers the injected response within tolerance.
- [ ] The fit passes through the origin; a synthetic offset does not shift it.
- [ ] Unverified observations are weighted lower, and `unverified_n` reports how
      many contributed.
- [ ] An injected outlier is rejected and named.
- [ ] Fewer than `MIN_RESPONSES` answers `None`.
- [ ] A non-positive fitted slope answers `None`.
- [ ] No input produces `NaN` or an infinity.
- [ ] `propose_dose` answers `None` for a non-finite reading or an absent
      estimate, and is never called outside `clamp_proposal`.

## Verification

```bash
cargo test -p rhizo-domain hydration::response
cargo test -p rhizo-domain hydration::prop
```

## Tests required

- Property: finite or `None`, over arbitrary generated observation sets.
- Property: `propose_dose` output is finite and positive whenever it is `Some`.
- Determinism across repeated evaluation.
- PRD 150's worked example reproduces its stated `proposed_ml`.

## Documentation impact

- PRD 150 §First deterministic model, if the accepted rule deviates.

## Files likely affected

```text
crates/domain/src/hydration/response.rs
crates/domain/src/hydration/mod.rs
```
