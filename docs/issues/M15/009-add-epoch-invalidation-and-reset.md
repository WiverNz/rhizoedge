# Issue M15-009 — Add epoch invalidation and reset triggers

**Milestone:** M15 · **PRD:** [PRD 150](../../prd/150-per-plant-adaptive-water-model.md) · **Depends on:** M15-008

## Context

A learned model describes a physical setup. Repot the plant, change the
substrate, move the probe two centimetres, or swap the device, and the model now
describes something that no longer exists — while continuing to report high
confidence, because nothing about its observations changed.

[ADR-019](../../adr/019-per-plant-adaptive-water-model.md) §7 chose explicit
epochs over silent re-learning: a decay window wide enough to absorb a repot is
wide enough to blur a real seasonal change, and a system that cannot say *when*
it stopped believing something cannot explain itself.

## Goal

Every event that invalidates a model opens a new epoch — automatically where the
system can see it, and through one explicit endpoint where it cannot.

## Scope

- **Automatic triggers:** a `control`-role sensor binding created, changed, or
  removed; an actuator rebinding; a bound sensor's `device_id` changing; a
  `calibration_ref` change on the control sensor; and no control-sensor sample
  for longer than `EPOCH_STALE_DAYS`, detected on the next sample.
- **Explicit trigger:** `POST /api/v1/plants/{id}/hydration-model/reset` taking
  `{ reason, note? }` with `reason` one of `repotted`, `substrate_changed`,
  `pot_changed`, `plant_replaced`, `operator_reset`.
- Opening an epoch: increment, write `plant_hydration_epochs`, mark prior
  observations `superseded`, clear the model row, emit a `plant_events` row.
- Estimators read only the current epoch — enforced in the repository query, not
  by convention at the call site.

## Non-goals

- Guessing at a repot from the data. Deliberately not attempted: a
  false-positive reset silently discards weeks of learning, and the operator who
  repotted is standing there anyway.
- Any cloud event. The ADR-005 catalogue is closed; M15-013 records the decision
  and its consequence.
- Merging or migrating observations across epochs. That is the thing epochs
  exist to prevent.

## Dependencies

- M15-008

## Implementation notes

The reset endpoint is **not** a watering path and carries no override, force, or
bypass semantics: it discards inference, never a limit. Say so in the handler's
doc comment, because "reset" next to a plant is exactly the shape of thing a
future contributor would extend to clear a lockout.

Epoch scoping belongs in the query. `WHERE plant_id = ? AND epoch = ? AND status
= 'accepted'` in `repo::hydration`, once, is what makes F-150-25 a property of
the system rather than of every caller remembering. Test it by writing
superseded observations and asserting the estimator cannot see them.

An automatic trigger fires from the same transaction as the change that caused
it — the binding write — so a crash cannot leave a changed binding with an
unreset model. This is the same persist-together discipline SAFETY-001 imposes
on the dedup marker and its effects.

`EPOCH_STALE_DAYS` is a starting value. A plant whose device was replaced during
a long holiday should come back to a fresh model; a plant that missed four days
should not.

## Acceptance criteria

- [ ] Each automatic trigger opens exactly one epoch, in the same transaction as
      its cause.
- [ ] The reset endpoint opens an epoch and records reason, note, and actor.
- [ ] An unknown `reason` is rejected with 422, never coerced to
      `operator_reset`.
- [ ] After an epoch change the model reports `cold_start` and proposes nothing.
- [ ] No estimator can read a `superseded` observation, asserted through the
      repository rather than the call site.
- [ ] Superseded observations are retained, not deleted.
- [ ] An epoch change writes a `plant_events` row visible in the plant history.
- [ ] The endpoint has no override, force, or bypass parameter.

## Verification

```bash
cargo test -p edge-controller hydration::epoch
cargo test -p edge-controller api::hydration
curl -s -XPOST localhost:8080/api/v1/plants/monstera-01/hydration-model/reset \
  -H 'content-type: application/json' -d '{"reason":"repotted"}' | jq
```

## Tests required

- Each automatic trigger, separately.
- Crash between a binding change and its epoch change leaves neither applied.
- A superseded observation is invisible to both estimators.
- Epoch increment is checked, not saturating, at the type boundary.

## Documentation impact

- `docs/protocol/http-api-boundaries.md`: the reset and mode endpoints.
- PRD 150 §User/system flows, if the trigger set deviates.

## Files likely affected

```text
crates/edge-controller/src/plant/hydration/epoch.rs
crates/edge-controller/src/api/hydration.rs
crates/edge-controller/src/api/bindings.rs
crates/storage/src/repo/hydration.rs
docs/protocol/http-api-boundaries.md
```
