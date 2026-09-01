# Issue M15-008 — Assemble the hydration model and its scheduled refresh

**Milestone:** M15 · **PRD:** [PRD 150](../../prd/150-per-plant-adaptive-water-model.md) · **Depends on:** M15-007

## Context

Four pieces now exist independently: two extractors and two estimators, plus a
confidence rule. This issue composes them into one `HydrationModel` per plant,
gives it a refresh that runs on the control tick, and establishes the property
everything downstream depends on — that the model is a **function of the
observation ledger**, so restarting the edge cannot change a recommendation.

## Goal

A persisted, rebuildable `HydrationModel` per plant, refreshed on the tick,
never consulted by any watering path yet.

## Scope

- `plant::hydration::refresh(plant_id)`: extract new segments and responses,
  re-estimate, compute confidence, and write `plant_hydration_model` — all in
  one transaction, so a crash mid-refresh leaves observations and model
  consistent.
- `plant::hydration::rebuild(plant_id, epoch)`: re-estimate from the persisted
  observation ledger alone, ignoring the stored model.
- Refresh scheduling inside `control::tick`, skipped for plants whose
  `adaptive_mode` is `disabled`, and bounded so a slow refresh cannot delay the
  irrigation pass.
- `model_version`: a stored estimator-semantics version; a mismatch forces a
  rebuild rather than reinterpreting numbers produced by different arithmetic.

## Non-goals

- Any consumption of the model by a recommendation or a command. M15-010,
  M15-011, M15-012.
- Epoch management. M15-009.

## Dependencies

- M15-007

## Implementation notes

**`refresh` must never be able to delay or fail a watering decision.** Run it
after the irrigation pass in the tick, treat every non-fatal error as a logged
warning that leaves the previous model in place, and give it its own duration
histogram so a slow refresh is visible before it is a problem. The precedent is
`sample_storage_bytes`: a gauge is not worth the process, and neither is a
model.

`rebuild` is not a debugging convenience — it is the executable statement of
F-150-26, and M15-014 asserts that `rebuild` and the incrementally maintained
model agree for every plant in the scenario suite. Write it as a first-class
function with its own tests, not as a test helper.

Persist the model with `updated_from_observation_id` so a stored model can be
traced to the last observation that moved it. Without it, "why did this change
overnight?" is unanswerable, and that question will be asked.

## Acceptance criteria

- [ ] `refresh` is transactional; a simulated crash mid-refresh leaves no
      partially updated model.
- [ ] A refresh with no new observations writes nothing and is cheap.
- [ ] `rebuild` reproduces the incrementally maintained model exactly, for every
      test history.
- [ ] Restarting the process produces the same recommendation before and after.
- [ ] A `model_version` mismatch triggers a rebuild.
- [ ] `disabled` plants are skipped entirely — no extraction, no estimation, no
      rows.
- [ ] A refresh failure logs, leaves the previous model in place, and never
      affects the irrigation pass.

## Verification

```bash
cargo test -p edge-controller hydration::refresh
cargo test -p edge-controller hydration::rebuild
cargo test -p edge-controller control::tick
```

## Tests required

- Incremental-versus-rebuild equality over generated histories.
- Restart equivalence, using the `TestClock` so the comparison is exact.
- A refresh error does not propagate into the control tick's result.
- `disabled` plants accumulate no rows.

## Documentation impact

- PRD 150 §Interfaces, if the composed signature deviates.
- `docs/architecture/data-flow.md`: the refresh appears in the control-tick flow.

## Files likely affected

```text
crates/edge-controller/src/plant/hydration/mod.rs
crates/edge-controller/src/control/tick.rs
crates/storage/src/repo/hydration.rs
docs/architecture/data-flow.md
```
