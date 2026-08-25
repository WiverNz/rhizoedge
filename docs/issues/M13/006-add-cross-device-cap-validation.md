# Issue M13-006 — Add cross-device daily cap validation

**Milestone:** M13 · **PRD:** [PRD 130](../../prd/130-multi-plant-home.md) · **Depends on:** M13-001

## Context

PRD 130: `FIRMWARE_MAX_DAILY_ML` is **per device**. With several plants on one
device, the device cap bounds their combined delivery — correct and
conservative, but surprising if per-plant caps sum above it.

## Goal

Prevent a configuration whose caps cannot all be honoured.

## Scope

- When several plants share a device, validate that the sum of their `max_daily_ml` does not exceed the device cap
- **Reject at configuration time** with an explanatory error
- Surface the device cap and current allocation in the API
- Show the allocation in the UI

## Non-goals

- Dynamically allocating the device budget between plants.

## Dependencies

- M13-001

## Implementation notes

Rejecting rather than clamping, consistent with ADR-011: the operator learns
the real constraint while editing rather than discovering at 3 a.m. that plant
three never gets watered because plants one and two consumed the device budget.

Surfacing the allocation is what makes the constraint understandable rather than
merely enforced.

## Acceptance criteria

- [ ] A configuration whose per-plant caps exceed the device cap is **rejected**.
- [ ] The error names the device cap and the attempted total.
- [ ] The API exposes the device cap and current allocation.
- [ ] The UI shows the allocation.
- [ ] Single-plant devices are unaffected.

## Verification

```bash
cargo test -p rhizo-domain cap_allocation::
```

## Tests required

- Rejection with a clear message.
- Allocation reporting.
- Single-plant unaffected.

## Documentation impact

- configuration-model.md notes the per-device cap interaction.

## Files likely affected

```text
crates/domain/src/profile.rs
crates/edge-controller/src/api/plants.rs
```
