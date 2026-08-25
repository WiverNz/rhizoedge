# Issue M13-005 — Add plant grouping and filtering

**Milestone:** M13 · **PRD:** [PRD 130](../../prd/130-multi-plant-home.md) · **Depends on:** M13-001

## Context

Twenty plants in one flat list is unusable. Grouping by room or reservoir
makes the overview legible.

## Goal

Make a larger deployment navigable.

## Scope

- `plants.group_name` column
- Filtering by group, state, and lockout on the plants endpoint
- Grouped rendering in the UI overview
- Migration adding the column

## Non-goals

- Nested groups.

## Dependencies

- M13-001

## Implementation notes

A flat optional group name rather than a hierarchy: a household has rooms,
not a tree, and adding hierarchy now would be a guess about a requirement.

## Acceptance criteria

- [ ] Plants can be assigned a group.
- [ ] Filtering by group, state, and lockout works.
- [ ] The UI groups plants in the overview.
- [ ] Ungrouped plants render sensibly.
- [ ] The migration is additive.

## Verification

```bash
curl -s 'localhost:8080/api/v1/plants?group=bedroom&locked=true' | jq
```

## Tests required

- Filtering combinations.
- Ungrouped handling.

## Documentation impact

- http-api-boundaries.md extended.

## Files likely affected

```text
migrations/edge/0003_plant_groups.sql
crates/edge-controller/src/api/plants.rs
```
