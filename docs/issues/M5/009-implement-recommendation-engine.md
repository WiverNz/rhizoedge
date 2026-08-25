# Issue M5-009 — Implement the rule-based recommendation engine

**Milestone:** M5 · **PRD:** [PRD 050](../../prd/050-plant-model-and-recommendations.md) · **Depends on:** M5-005, M5-006

## Context

PRD 050's rule. Explainability is built in from the start: reasons are typed
enum values, so the UI can render them and tests can assert on them.

## Goal

Produce an explainable recommendation with structured reasons.

## Scope

- The conjunctive rule from PRD 050
- **Every failing conjunct contributes a reason** — a `no_water` answer is as explainable as a `water` one
- `recommendation ∈ water | no_water | blocked`
- `recommended_ml` from `profile.dose_ml`, never an unbounded computation
- `confidence` reduced for sparse, noisy, or partially missing inputs
- `blocked_by` naming the lockout when the gate would refuse

## Non-goals

- Issuing commands (M6).
- Machine learning — permanently out of scope.

## Dependencies

- M5-005
- M5-006

## Implementation notes

`confidence` is reported for operator intuition and is **not** an input to any
safety decision. State that in the doc comment; a future contributor might
otherwise gate a dose on it.

Reasons as typed enums rather than strings costs boilerplate and buys assertable
tests and a renderable UI. Render to prose in exactly one place (the API layer).

## Acceptance criteria

- [ ] A dry, fresh, past-cooldown plant recommends water with reasons.
- [ ] Each failing conjunct produces its specific reason.
- [ ] `no_water` carries reasons explaining why.
- [ ] `recommended_ml` equals `profile.dose_ml`.
- [ ] Confidence drops with sparse data.
- [ ] The engine is pure — no I/O, no clock access.
- [ ] Confidence is used in no decision.

## Verification

```bash
cargo test -p rhizo-domain recommend::
```

## Tests required

- Each conjunct failing in isolation, asserting the exact reason set.
- The all-pass case.
- Confidence reduction.
- Purity (no clock calls).

## Documentation impact

- Doc comment stating confidence is advisory only.

## Files likely affected

```text
crates/domain/src/recommend.rs
```
