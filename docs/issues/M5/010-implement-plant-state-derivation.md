# Issue M5-010 — Implement plant state derivation

**Milestone:** M5 · **PRD:** [PRD 050](../../prd/050-plant-model-and-recommendations.md) · **Depends on:** M5-009

## Context

PRD 050's state model is descriptive and operator-facing, deliberately
separate from the irrigation state machine that acts (M6). Keeping them distinct
lets the UI show 'needs water' without implying 'is about to water'.

## Goal

Derive and persist the operator-facing plant state.

## Scope

- Derive `Healthy | Drying | WaterRecommended | SensorFault | WateringLocked`
- Persist transitions as events
- Expose in the plant API
- Kept distinct from `IrrigationState`

## Non-goals

- Irrigation transitions (M6-006).

## Dependencies

- M5-009

## Implementation notes

Do not merge these two state concepts, however tempting. `WaterRecommended`
with automation disabled is a normal, stable, indefinite condition; the
irrigation machine would have nothing useful to say about it.

Persist transitions, not every evaluation — a 30-second tick would otherwise
write thousands of rows recording that nothing changed.

## Acceptance criteria

- [ ] Each state is derived under its documented conditions.
- [ ] Transitions are persisted as events; steady state is not.
- [ ] State appears in the plant API.
- [ ] `PlantState` and `IrrigationState` are separate types.
- [ ] A plant with automation off can sit in `WaterRecommended` indefinitely.

## Verification

```bash
cargo test -p rhizo-domain plant_state::
```

## Tests required

- Each state's conditions.
- Transition-only persistence.
- Separation from IrrigationState.

## Documentation impact

- None.

## Files likely affected

```text
crates/domain/src/plant_state.rs
crates/edge-controller/src/plant/state.rs
```
