# Issue M13-003 — Add the reservoir entity

**Milestone:** M13 · **PRD:** [PRD 130](../../prd/130-multi-plant-home.md) · **Depends on:** M13-001

## Context

Several devices may draw from one tank. Modelling the reservoir explicitly is
what lets a low level lock out every plant that depends on it.

## Goal

Model shared reservoirs.

## Scope

- `reservoirs` table with capacity and minimum
- `devices.reservoir_id` foreign key
- REST endpoints for reservoirs
- Level derived from the devices reporting on that reservoir
- Migration adding the table and column

## Non-goals

- The lockout logic (M13-004).

## Dependencies

- M13-001

## Implementation notes

Additive migration: existing devices get a null `reservoir_id` and behave
exactly as before, using their own tank reading. Only devices explicitly grouped
into a reservoir change behaviour.

## Acceptance criteria

- [ ] Reservoirs can be created and devices assigned.
- [ ] The migration is additive and existing behaviour is unchanged.
- [ ] The reservoir level derives from its devices' readings.
- [ ] REST endpoints work.
- [ ] A device with no reservoir behaves as before.

## Verification

```bash
cargo test -p rhizo-storage repo::reservoir
curl -s localhost:8080/api/v1/reservoirs | jq
```

## Tests required

- CRUD.
- Migration additivity.
- Level derivation.

## Documentation impact

- ADR-004 schema section extended.

## Files likely affected

```text
migrations/edge/0002_reservoirs.sql
crates/edge-controller/src/api/reservoirs.rs
```
