# Issue M5-013 — Implement sensor and actuator bindings

**Milestone:** M5 · **PRD:** [PRD 050](../../prd/050-plant-model-and-recommendations.md) · **Depends on:** M5-001, M4-011

## Context

[ADR-016](../../adr/016-plant-binding-and-policy-model.md) replaces the flat
`PlantConfig` with bindings, so a rule names a **kind and a role** and a binding
maps that to hardware. Replacing a failed probe becomes a binding edit rather than
a data migration.

## Goal

Implement bindings with their roles and validation.

## Scope

- `SensorBinding` CRUD: device, sensor, point, kind, role (`control`/`required`/`advisory`)
- `ActuatorBinding` CRUD with cardinality **0..1** — zero is normal and fully supported
- Validation against declared capabilities (M4-011): reject a binding naming an undeclared sensor or actuator
- At most one `control` binding per plant; removing the last one is refused while automation is enabled
- Leak and tank bindings forced to `required` for any plant with an actuator
- REST endpoints for both binding kinds

## Non-goals

- Threshold policies (M5-014).
- Offline policy authoring (M5-016).

## Dependencies

- M5-001
- M4-011

## Implementation notes

The three roles are safety-relevant and must not be interchangeable. `control`
drives the decision; `required` must be healthy for actuation to be safe;
`advisory` is recorded and may alert but never gates the pump. Marking a leak
sensor `advisory` would silently remove its veto, which is why validation forces
it to `required`.

**Zero actuator bindings is the common case**, not a degraded one. Test it as a
first-class path: a monitoring-only plant must be creatable, viewable, and
alertable, and must simply have no actuation route (SAFETY-018).

## Acceptance criteria

- [ ] Bindings can be created, listed, updated, and deleted.
- [ ] A binding naming an undeclared capability is rejected with a specific error.
- [ ] A second `control` binding on one plant is rejected.
- [ ] Removing the last `control` binding while automation is on is refused.
- [ ] A plant with **no** actuator binding is fully functional for monitoring.
- [ ] Leak and tank bindings cannot be set to `advisory` when an actuator exists.
- [ ] Replacing a sensor is a binding edit that preserves history and policies.

## Verification

```bash
cargo test -p rhizo-domain binding::
cargo test -p edge-controller api::bindings
cargo test safety_018
```

## Tests required

- Each validation rule.
- Role semantics.
- SCEN-106 monitoring-only plant.
- Sensor replacement preserves policies.

## Documentation impact

- http-api-boundaries.md binding endpoints.

## Files likely affected

```text
crates/domain/src/binding.rs
crates/storage/src/repo/binding.rs
crates/edge-controller/src/api/bindings.rs
```
