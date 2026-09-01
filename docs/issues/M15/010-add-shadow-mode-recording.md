# Issue M15-010 — Add shadow-mode proposal recording

**Milestone:** M15 · **PRD:** [PRD 150](../../prd/150-per-plant-adaptive-water-model.md) · **Depends on:** M15-009

## Context

No estimator in this milestone has been validated against a real plant, and no
amount of synthetic history can do it: the failure modes that matter are a probe
that drifts, a pot that changes, and a human who waters without telling anyone.
Shadow mode is how those get found before an estimate can move a millilitre.

It also answers the harder review question — "how would you know if this model
were wrong?" — with a query rather than an opinion.

## Goal

Compute and persist what the model *would* have recommended, alongside what the
static policy *did* recommend, without any watering path reading it.

## Scope

- `adaptive_mode` values `disabled` / `shadow` / `advisory` / `adaptive`, with
  `PUT /api/v1/plants/{id}/adaptive-mode` and validation.
- On every tick that evaluates a plant in `shadow` or `advisory` mode: compute
  the proposal, apply `clamp_proposal`, and write a `plant_adaptive_decisions`
  row carrying `static_ml`, `proposed_ml`, `clamped_ml`, `applied = 'static'`,
  confidence, and typed reasons.
- Record the predicted rise for each issued dose, so M15-013 can compare it with
  the observed rise.
- `GET /api/v1/plants/{id}/hydration-model/observations` for inspecting the
  accumulated comparison.

## Non-goals

- Letting any recorded value affect a decision. That is M15-012, and it is a
  separate reviewable change on purpose.
- Showing the adaptive block to the operator in `shadow`. That is what
  `advisory` is for; M15-011 draws the line.

## Dependencies

- M15-009

## Implementation notes

`applied` is written as `'static'` in this issue, by every path, unconditionally.
M15-012 is the only change permitted to write `'adaptive'`, and having the column
already exist means that change is a one-line diff a reviewer can see whole.

A `no_commands_in_shadow` test belongs here, in the spirit of M5's
`no_commands_in_m5`: assert by source scan and by behaviour that no shadow-mode
path reaches `control::command`. The M5 precedent is the reason that test was
believable, and the same reasoning applies to a milestone whose entire claim is
"this changes nothing yet".

Writing a decision row on every tick for every enabled plant is more rows than
the recommendation table takes, because `plant_recommendations` is written only
on change. Follow that precedent: write on change of `(clamped_ml, confidence,
reasons)`, not on every tick, and let retention bound the rest.

## Acceptance criteria

- [ ] `adaptive_mode` accepts exactly the four values and rejects others with
      422.
- [ ] `shadow` and `advisory` produce decision rows and no commands.
- [ ] `applied` is `'static'` on every row this issue can produce.
- [ ] Rows are written on change, not on every tick.
- [ ] `disabled` produces no rows.
- [ ] The observations endpoint pages deterministically and filters by epoch.
- [ ] `no_commands_in_shadow` passes, by source scan and by behaviour.

## Verification

```bash
cargo test -p edge-controller hydration::shadow
cargo test -p edge-controller no_commands_in_shadow
curl -s localhost:8080/api/v1/plants/monstera-01/hydration-model/observations | jq
```

## Tests required

- A full simulated watering history in `shadow` issues no command and records
  every comparison.
- Mode transitions in both directions take effect on the next tick.
- Change-only persistence, asserted by row count over a stable history.

## Documentation impact

- `docs/protocol/http-api-boundaries.md`: the mode and observations endpoints.
- PRD 150 §State model, if the mode set deviates.

## Files likely affected

```text
crates/edge-controller/src/plant/hydration/shadow.rs
crates/edge-controller/src/api/hydration.rs
crates/edge-controller/src/control/tick.rs
docs/protocol/http-api-boundaries.md
```
