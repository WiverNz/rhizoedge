# Issue M15-001 — Add hydration domain types and the model epoch

**Milestone:** M15 · **PRD:** [PRD 150](../../prd/150-per-plant-adaptive-water-model.md) · **Depends on:** M13-017

## Context

[ADR-019](../../adr/019-per-plant-adaptive-water-model.md) introduces a per-plant
hydration model as **derived state**, in the same family as `plant_dry_state` and
`plant_threshold_state` rather than in the authored family with
`measurement_policies`. Before any estimator, storage, or endpoint exists, the
vocabulary has to exist, in the one crate that is pure and clock-free.

Naming was decided in ADR-019 §2 and should not be re-opened here.

## Goal

Land the types, the epoch, and the estimator configuration in
`rhizo_domain::hydration`, with no arithmetic that anyone could mistake for an
estimator.

## Scope

- New module `crates/domain/src/hydration/` with `mod.rs` and `types.rs`.
- `DryingSegment`, `DoseResponseObservation`, `DryingRate`, `DoseResponse`,
  `HydrationModel`, `ModelConfidence`, `EpochReason`, `ObservationStatus`,
  `EstimatorConfig`, `ClampedDose`.
- `ModelEpoch(u32)` with a checked `next()`; a saturating increment would let a
  wrapped epoch silently reuse a superseded epoch's observations.
- Wire names for every enum, as `as_str`, matching the `LockoutReason` and
  `MeasurementKind` precedent.
- Documented defaults: `MIN_SEGMENT_SAMPLES`, `MIN_SEGMENT_HOURS`,
  `MIN_SEGMENTS`, `MIN_RESPONSES`, `OBSERVATION_HALF_LIFE_DAYS`,
  `EPOCH_STALE_DAYS`, `MAD_OUTLIER_K`, `MIN_EFFECTIVE_ML`.

## Non-goals

- Any estimator. M15-004 and M15-006.
- Any storage. M15-002.
- Any change to `recommend`, `irrigation`, or `plant`.

## Dependencies

- M13-017

## Implementation notes

The crate is pure and the clock ban is lint-enforced, so every type here takes
time as data. `DryingSegment` carries `started_at`/`ended_at` and
`DoseResponseObservation` carries `dosed_at`/`peak_at`, and the estimators later
take a `now` parameter for weighting rather than reading one.

`ModelConfidence` is a **different quantity** from `Recommendation::confidence`,
which is advisory and gates nothing by deliberate design. Say so in the doc
comment on both, in this issue, so the two never get merged by someone tidying
up.

Every constant here is a starting value, not a measurement. Mark them as such —
PRD 150 §Open questions 3 is where they get revisited.

## Acceptance criteria

- [ ] `rhizo_domain::hydration` compiles with no I/O, no `Utc::now`, and no
      dependency beyond what `rhizo-domain` already has.
- [ ] Every enum has a documented stable `as_str` and no catch-all match arm.
- [ ] `ModelEpoch::next` is checked, not saturating, and is tested at the bound.
- [ ] The doc comment on `ModelConfidence` states its difference from
      `Recommendation::confidence`.
- [ ] Every default constant carries its provenance as "starting value".

## Verification

```bash
cargo test -p rhizo-domain hydration::
cargo clippy -p rhizo-domain --all-targets -- -D warnings
```

## Tests required

- `ModelEpoch::next` at `u32::MAX`.
- Round-trip of every enum through its `as_str`.
- A compile-fail-style source scan asserting no `_ =>` arm in the module, in the
  style of `no_catch_all_arm_on_a_safety_match`.

## Documentation impact

- ADR-019 referenced from the module doc comment.
- `component-model.md`: `rhizo-domain` gains the hydration module in its
  responsibilities list.

## Files likely affected

```text
crates/domain/src/lib.rs
crates/domain/src/hydration/mod.rs
crates/domain/src/hydration/types.rs
docs/architecture/component-model.md
```
