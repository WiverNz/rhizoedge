# Issue M5-001 — Add plant and profile repositories

**Milestone:** M5 · **PRD:** [PRD 050](../../prd/050-plant-model-and-recommendations.md) · **Depends on:** M4-013

## Context

The `plants` and `plant_profiles` tables were created in M3. M5 gives them
repository methods and the invariants that go with them.

## Goal

Provide CRUD access to plants and profiles with their integrity rules.

## Scope

- Repository methods for both entities
- `auto_watering_enabled` defaults to **false** on insert
- Profiles reusable across plants; deleting an in-use profile is refused
- Deleting a plant **preserves** its watering events
- Foreign keys enforced

## Non-goals

- The HTTP layer (M5-002, M5-004).
- Profile value validation (M5-003).

## Dependencies

- M4-013

## Implementation notes

`auto_watering_enabled = false` at the storage layer, not only in the API —
a plant created by any path must be inert until a human opts in (SAFETY-012).

Plant deletion must not cascade to `watering_events`; that history is the record
of what the machine did and outlives the row that pointed at it. Nullify the
reference or use a soft delete, and assert it in a test.

## Acceptance criteria

- [ ] Plants and profiles can be created, read, updated, and deleted.
- [ ] A new plant has `auto_watering_enabled = false`.
- [ ] Deleting a profile in use is refused.
- [ ] Deleting a plant leaves its `watering_events` rows intact.
- [ ] Foreign key violations are rejected.

## Verification

```bash
cargo test -p rhizo-storage repo::plant repo::profile
```

## Tests required

- CRUD paths.
- Default-off assertion.
- In-use profile deletion refused.
- **History preserved on plant delete.**

## Documentation impact

- None.

## Files likely affected

```text
crates/storage/src/repo/plant.rs
crates/storage/src/repo/profile.rs
```
