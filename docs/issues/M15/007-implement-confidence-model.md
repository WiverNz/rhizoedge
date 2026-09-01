# Issue M15-007 — Implement the confidence model

**Milestone:** M15 · **PRD:** [PRD 150](../../prd/150-per-plant-adaptive-water-model.md) · **Depends on:** M15-006

## Context

`Recommendation::confidence` is an `f32` that is advisory by design and gates
nothing, with a test that fails if anyone gates on it. `ModelConfidence` is the
opposite: a four-valued enum whose entire job is to gate. Confusing the two
would either make the recommendation engine's confidence load-bearing — which
PRD 050 §Open questions 1 explicitly avoided — or make the model's confidence
decorative, which would remove the cold-start safety property.

## Goal

`hydration::confidence`, and the rule that below `Medium` the model proposes
nothing.

## Scope

- `ColdStart` when either estimate is absent or the epoch has no observations.
- `Low`, `Medium`, `High` from: accepted observation counts against
  `MIN_SEGMENTS` / `MIN_RESPONSES`, residual spread against its ceiling, age of
  the newest observation, age of the epoch, and whether the plant's control
  sensor is currently flagged by `sensor_stuck_state` or unhealthy.
- Exhaustive matching, no catch-all arm, `Option` inputs throughout.
- `ModelConfidence::proposes()` — the single predicate the rest of the system
  asks, true only for `Medium` and `High`.

## Non-goals

- A continuous score. Four values are what the explanation can defend and what
  an operator can act on; a percentage invites a threshold argument nobody can
  settle.
- Merging with `Recommendation::confidence`.

## Dependencies

- M15-006

## Implementation notes

Confidence must **fall**, not only rise. The failure this guards is a plant that
learned well in spring, was left alone, and is still being trusted in August: the
newest-observation-age term is what makes that plant drift back to `Low` on its
own, with no event to trigger it.

An unhealthy or stuck control sensor caps confidence at `Low` regardless of the
observation history. The samples are already suppressed elsewhere, so the
estimates go stale rather than wrong — but "stale and trusted" is the exact
shape of the sleeping-device problem SAFETY-021 exists to prevent, and the same
answer applies.

`proposes()` is a method rather than a comparison at each call site, for the
reason `LeakLockout::blocks` is: a threshold spelled out in four places is a
threshold that will eventually differ in one of them.

## Acceptance criteria

- [ ] An absent estimate is `ColdStart`, never `Low`.
- [ ] Confidence rises with clean observations and falls with age.
- [ ] An unhealthy or stuck control sensor caps confidence at `Low`.
- [ ] An epoch change returns confidence to `ColdStart`.
- [ ] `proposes()` is true only for `Medium` and `High`.
- [ ] No catch-all arm anywhere in the module.
- [ ] `confidence_is_reported_and_never_decides` in `recommend` still passes.

## Verification

```bash
cargo test -p rhizo-domain hydration::confidence
cargo test -p rhizo-domain confidence_is_reported_and_never_decides
```

## Tests required

- Each transition, in both directions.
- A table-driven case per input dimension, holding the others fixed.
- A source scan asserting `Recommendation::confidence` is not read by any
  hydration module — the two quantities must not converge by accident.

## Documentation impact

- PRD 050 §Open questions 1: note that the advisory confidence stays advisory
  and that a separate gating confidence now exists.

## Files likely affected

```text
crates/domain/src/hydration/confidence.rs
crates/domain/src/recommend.rs
docs/prd/050-plant-model-and-recommendations.md
```
