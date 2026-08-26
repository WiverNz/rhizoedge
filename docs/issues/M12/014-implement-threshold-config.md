# Issue M12-014 — Implement threshold and alert configuration

**Milestone:** M12 · **PRD:** [PRD 120](../../prd/120-rust-ui.md) · **Depends on:** M12-013

## Context

Warnings and control conditions are different things, and the UI is where that
distinction either becomes clear to the operator or is lost.

## Goal

Make per-plant thresholds and their alerts configurable and legible.

## Scope

- Per-kind warning and critical band editing with a visual band preview
- Alert severity configuration per kind
- **Explicitly show that a threshold alerts but does not water**
- Current value shown against its bands
- Threshold history with crossing events

## Non-goals

- Notification delivery configuration (M13-007).

## Dependencies

- M12-013

## Implementation notes

The band preview earns its place: nested warning-inside-critical bands are hard
to reason about as four numbers and obvious as a diagram, and an operator who can
see the bands is far less likely to invert them.

State plainly in the UI that a critical temperature raises an alert and does not
cause watering. An operator who assumes the system will "do something" about a
cold room is an operator who will not act themselves.

## Acceptance criteria

- [ ] Bands are editable and rendered as a visual preview.
- [ ] The current value is shown against its bands.
- [ ] The UI states that thresholds alert and do not actuate.
- [ ] Crossing history is visible.
- [ ] Inverted or overlapping bands are rejected with a clear message.
- [ ] A plant with no policy for a bound kind is shown as unconfigured, not as healthy.

## Verification

```bash
cd ui/rhizo-ui && cargo test thresholds::
```

## Tests required

- Band rendering.
- Validation messages.
- Unconfigured-vs-healthy distinction.

## Documentation impact

- None.

## Files likely affected

```text
ui/rhizo-ui/src/views/thresholds.rs
```
