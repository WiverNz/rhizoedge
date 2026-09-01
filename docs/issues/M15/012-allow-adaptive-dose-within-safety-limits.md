# Issue M15-012 — Allow an adaptive dose inside the safety limits

**Milestone:** M15 · **PRD:** [PRD 150](../../prd/150-per-plant-adaptive-water-model.md) · **Depends on:** M15-011

## Context

This is the only issue in M15 that can change a volume that reaches a pump, and
it should be reviewed as such. Everything before it is inference; this is
actuation.

`IrrigationDecision::IssueDose` currently carries `inputs.automation.dose_ml`
verbatim, and its doc comment says "never a computed volume", citing F-060-23 —
which is the publish-retry requirement and has nothing to do with volumes. The
requirement it means is **F-050-23**: "derived from profile `dose_ml`, never
from an unbounded computation". A computation whose ceiling *is* `dose_ml`
satisfies that as written, and
[ADR-019](../../adr/019-per-plant-adaptive-water-model.md) §Status records the
reading so it is not re-litigated.

## Goal

An adaptive dose that is clamped in exactly one place, can only ever be smaller
than the static dose, and is offered to an **unchanged** gate and machine.
This is **SAFETY-022**.

## Scope

- `hydration::clamp_proposal(proposal, min_effective_ml, static_dose_ml) ->
  ClampedDose`, in `rhizo-domain`, the single clamp.
- `control::irrigation` selecting the dose for a plant in `adaptive` mode with
  `confidence.proposes()`: clamped proposal, else the static dose.
- Writing `applied = 'adaptive'` on the decision row, and the clamp reason.
- Correcting the "never a computed volume" comments and the F-060-23 citation on
  `AutomationPolicy::dose_ml` and `IrrigationDecision::IssueDose`.
- `safety_022_*` tests.

## Non-goals

- **Changing `safety_gate`, `machine::evaluate`, `budget::dose_fits`,
  `credited_ml`, any lockout rule, any TTL, or any firmware limit.** If this
  issue needs to touch one of them, the design is wrong and the answer is not to
  touch it.
- Any override, force, or bypass parameter.
- Any adaptive influence on `OfflinePolicy`, which is the device's, or on a
  cooldown, budget, staleness threshold, or lockout.

## Dependencies

- M15-011

## Implementation notes

Ordering is the whole safety argument:

```text
proposal → clamp → safety_gate → machine::evaluate → command → device gate → pump
```

The clamp runs **before** `evaluate`, so every existing check sees an ordinary
`f32` and cannot tell where it came from. That is deliberate: a gate that had to
know about adaptive doses would be a gate with two paths through it.

`clamp_proposal` returns a `ClampedDose` carrying the original, the clamped
value, and which bound was hit, rather than a bare `f32`. A bare return loses the
explanation, and this is the one number in the system an operator will most want
explained.

One behaviour change to state rather than discover: `budget::dose_fits` **refuses**
rather than clamping, and M15 does not change that — but a 12 ml adaptive dose
can now fit under a cap that a 40 ml static dose would have crossed. SAFETY-006
bounds the 24-hour **total**, and any dose fitting under it is permitted by the
invariant's own definition. Test it explicitly, both directions.

Where the proposal *exceeds* `dose_ml`, the clamp binds and the explanation says
so. Raising `dose_ml` stays an operator decision — F-150-16 and ADR-019 §5, and
the reason there is no "the model asked for more, so allow more" path anywhere.

## Acceptance criteria

- [ ] `clamp_proposal` is the only clamp, asserted by source scan.
- [ ] A proposal above `automation.dose_ml` is clamped to it.
- [ ] A proposal below `min_effective_ml` is clamped up, and `min_effective_ml`
      is itself never above `dose_ml`.
- [ ] Confidence below `Medium` uses the static dose, always.
- [ ] `adaptive_mode` other than `adaptive` uses the static dose, always.
- [ ] `safety_gate`, `machine::evaluate`, `budget`, and every lockout rule are
      byte-identical apart from doc comments.
- [ ] A proposal that would cross the rolling cap is refused with today's reason.
- [ ] A smaller adaptive dose that fits under the cap is issued, and the 24-hour
      total still never exceeds `max_daily_ml`.
- [ ] The cooldown, the cycle dose limit, the TTL, and the device gate are
      unaffected.
- [ ] The corrected comments cite F-050-23.
- [ ] No override, force, or bypass parameter exists on any path touched.

## Verification

```bash
cargo test safety_
cargo test -p rhizo-domain hydration::clamp
cargo test -p edge-controller hydration::adaptive
cargo test -p rhizo-domain irrigation::
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

## Tests required

- `safety_022_adaptive_dose_never_exceeds_the_static_dose`.
- `safety_022_adaptive_dose_cannot_cross_the_rolling_cap`.
- `safety_022_adaptive_dose_cannot_shorten_a_cooldown`.
- `safety_022_a_model_cannot_clear_a_lockout`.
- `safety_022_a_missing_or_corrupt_model_falls_back_to_the_static_policy`.
- Property: for arbitrary observation histories and arbitrary policies, the
  issued volume is always finite and always `<= automation.dose_ml`.
- Property: an adaptive decision never issues a volume a static decision with the
  same inputs would have been refused for.

## Documentation impact

- `docs/architecture/safety-invariants.md`: SAFETY-022 moves from planned to
  enforced, with its test names.
- PRD 060: a note that the dose may be a bounded computation from M15, with the
  F-050-23 reading and the corrected citation.
- PRD 050 F-050-23: the bounded-computation reading recorded.

## Files likely affected

```text
crates/domain/src/hydration/clamp.rs
crates/domain/src/plant.rs
crates/domain/src/irrigation/types.rs
crates/edge-controller/src/control/irrigation.rs
docs/architecture/safety-invariants.md
docs/prd/050-plant-model-and-recommendations.md
docs/prd/060-irrigation-control-and-safety.md
```
