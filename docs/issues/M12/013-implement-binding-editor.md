# Issue M12-013 — Implement the sensor binding and measurement policy editor

**Milestone:** M12 · **PRD:** [PRD 120](../../prd/120-rust-ui.md) · **Depends on:** M12-008

## Context

[ADR-016](../../adr/016-plant-binding-and-policy-model.md) gives every plant its
own bindings and policies. The operator needs to see and edit them, or the model
is only theoretically per-plant.

## Goal

Let the operator manage a plant's sensors and what its measurements mean.

## Scope

- List a plant's sensor bindings with device, sensor, kind, point, and role
- Add and remove bindings, choosing only from **declared** device capabilities
- Show actuator binding, or clearly show that the plant has none
- Edit per-kind target, warning, and critical bands and `stale_after`
- Client-side validation mirroring the server's; 422 rendered inline
- Show which bindings currently have no measurement policy

## Non-goals

- Threshold alert configuration (M12-014).
- Offline policy editing (M12-015).

## Dependencies

- M12-008

## Implementation notes

Offer only capabilities the device actually declared. A free-text sensor field
would let an operator create a binding that can never produce data and that the
server will reject anyway — surfacing the real options is both friendlier and
safer.

A plant with no actuator must render as a **normal monitoring plant**, not as a
plant with a missing part. No empty watering panel, no disabled controls, no
warning icon (SAFETY-018).

## Acceptance criteria

- [ ] Bindings are listed, added, and removed.
- [ ] Only declared capabilities are offerable.
- [ ] A monitoring-only plant renders normally with no watering UI at all.
- [ ] Threshold bands are editable per kind.
- [ ] Server 422s render inline naming the violated rule.
- [ ] Bindings without a policy are visibly flagged.
- [ ] Role semantics are explained where the operator chooses one.

## Verification

```bash
cd ui/rhizo-ui && cargo test bindings::
```

## Tests required

- Capability-constrained selection.
- Monitoring-only rendering.
- 422 inline rendering.

## Documentation impact

- None.

## Files likely affected

```text
ui/rhizo-ui/src/views/bindings.rs
```
