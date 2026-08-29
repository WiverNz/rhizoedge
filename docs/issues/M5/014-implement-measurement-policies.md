# Issue M5-014 — Implement per-measurement threshold policies

**Milestone:** M5 · **PRD:** [PRD 050](../../prd/050-plant-model-and-recommendations.md) · **Depends on:** M5-013

## Context

Different plants legitimately disagree about what a reading means: the same room
temperature is fine for one plant and critical for another. Thresholds therefore
belong to the plant, not to the sensor
([ADR-016](../../adr/016-plant-binding-and-policy-model.md)).

## Goal

Implement per-plant, per-kind threshold configuration with validation.

## Scope

- `MeasurementPolicy` per (plant, kind): target range, warning band, critical band, `stale_after`, hysteresis, confirm duration
- Not every kind needs every field — all optional except `stale_after`
- Validation: `target_min < target_max`; warning band inside critical band; positive durations
- Seeded from the plant's profile template at creation
- REST endpoints

## Non-goals

- Evaluating them (M5-015).

## Dependencies

- M5-013

## Implementation notes

Warning and critical bands must nest correctly: `critical_low <= warning_low <
warning_high <= critical_high`. A configuration where warning is outside critical
is incoherent and would produce alerts in an order nobody expects, so reject it
rather than trying to interpret it.

Profiles seed and then stop mattering. Editing `monstera_default` must **not**
rewrite twelve plants' thresholds — silently changing the irrigation rules of
existing plants is not a feature (ADR-016).

## Acceptance criteria

- [x] Policies can be set per plant and per kind.
- [x] Each validation rule rejects with its own error.
- [x] A missing optional field is genuinely optional and does not block evaluation.
- [x] `stale_after` is required and validated positive.
- [x] Creating a plant from a profile seeds policies.
- [x] Editing a profile afterwards does **not** modify existing plants.
- [x] Two plants can hold different thresholds for the same shared sensor.

## Verification

```bash
cargo test -p rhizo-domain measurement_policy::
cargo test -p edge-controller api::policies
```

## Tests required

- Each validation rule.
- Optionality.
- Profile seeding without retroactive edits.
- Shared-sensor divergence.

## Documentation impact

- http-api-boundaries.md policy endpoints.

## Files likely affected

```text
crates/domain/src/measurement_policy.rs
crates/edge-controller/src/api/measurement_policies.rs
```
