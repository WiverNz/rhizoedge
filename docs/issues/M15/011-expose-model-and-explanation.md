# Issue M15-011 — Expose the model and its explanation over the API

**Milestone:** M15 · **PRD:** [PRD 150](../../prd/150-per-plant-adaptive-water-model.md) · **Depends on:** M15-010

## Context

The recommendation engine's founding argument was that an operator will not
enable automation they do not understand, which is why every failing conjunct
contributes a typed reason and why a `no_water` answer is as explainable as a
`water` one. A learned model raises the bar: the operator now has to understand
not only the rule but where its numbers came from.

"Why does Rhizo recommend 28 ml?" must be answerable in numbers, from the API,
with no access to the database.

## Goal

`GET /api/v1/plants/{id}/hydration-model`, an `adaptive` block on the
recommendation response in `advisory` and `adaptive` modes, and typed reasons
for every adaptive contribution.

## Scope

- `GET /api/v1/plants/{id}/hydration-model` returning epoch and its reason,
  learning state, confidence, both estimates with their spread and observation
  counts and ages, `expected_dry_at`, `updated_at`, and — when the model
  proposes nothing — `observations_needed` naming what is missing.
- An `adaptive` block on `GET /plants/{id}/recommendation`:
  `proposed_ml`, `clamped_ml`, `applied`, `confidence`, and the clamp reason.
- New `Reason` variants, typed like every existing one:
  `AdaptiveProposal { proposed_ml, response_per_ml }`,
  `AdaptiveClamped { proposed_ml, clamped_ml, ceiling_ml }`,
  `AdaptiveUnavailable { missing }`,
  `AdaptiveConfidenceTooLow { confidence }`,
  `ExpectedDry { at, vwc_per_day }`.
- Their `code()` strings and their prose rendering in `control::tick::reason_text`,
  which is the one place prose is produced.

## Non-goals

- A UI. [PRD 120](../../prd/120-rust-ui.md) builds the screen; this issue makes
  it possible.
- Letting the block change a dose. M15-012.

## Dependencies

- M15-010

## Implementation notes

The worked example in PRD 150 §First deterministic model is the acceptance
target: the endpoint must carry every number that example states, so the
question in the issue title is answerable from one response body. Assert that in
a test against a fixture history rather than trusting the field list.

`AdaptiveUnavailable { missing }` is the important variant, and the reason the
enum is not simply "no adaptive block". A cold-start plant and a plant whose
drying estimate went stale look identical from outside unless the response says
which. `missing` is a typed enum, not a string.

Reasons stay typed and prose stays in the API layer — the property M5 established
and the thing most likely to be eroded by an issue whose subject is explanation.
A `String` reason added here would be the first one in the project.

## Acceptance criteria

- [ ] The endpoint answers for a cold-start plant, naming what is missing.
- [ ] The endpoint answers for a confident plant with every documented field.
- [ ] The recommendation `adaptive` block appears in `advisory` and `adaptive`,
      and is absent in `disabled` and `shadow`.
- [ ] Every adaptive reason is a typed variant with a stable `code()`.
- [ ] No reason variant carries free prose.
- [ ] PRD 150's worked example is reproducible from one response body.
- [ ] A plant with no actuator can still hold and report a full model
      (SAFETY-018).

## Verification

```bash
cargo test -p edge-controller api::hydration
cargo test -p edge-controller hydration::explanation
curl -s localhost:8080/api/v1/plants/monstera-01/hydration-model | jq
curl -s localhost:8080/api/v1/plants/monstera-01/recommendation | jq .adaptive
```

## Tests required

- Fixture-driven reproduction of the worked example.
- Every `Reason` variant renders both as JSON and as prose.
- A monitoring-only plant reports a model and no proposal path.
- Response shape is stable across a restart.

## Documentation impact

- `docs/protocol/http-api-boundaries.md` §2.5: the recommendation response gains
  a block; the new endpoint is documented alongside it.
- PRD 150 §Interfaces.

## Files likely affected

```text
crates/edge-controller/src/api/hydration.rs
crates/edge-controller/src/api/recommendation.rs
crates/edge-controller/src/control/tick.rs
crates/domain/src/recommend.rs
docs/protocol/http-api-boundaries.md
```
