# Issue M12-005 — Implement the device view

**Milestone:** M12 · **PRD:** [PRD 120](../../prd/120-rust-ui.md) · **Depends on:** M12-002

## Context

Device health, sensor status, config drift, and the firmware hard limits —
the latter shown read-only, because they are reported one-way.

## Goal

Show device state and health.

## Scope

- Online state, firmware and protocol version, `clock_synced`
- Sample age, sensor presence and health
- **Config drift shown when desired and applied differ**
- Hard limits displayed read-only
- Device events with severity
- Rename (display name only)

## Non-goals

- Changing `device_id` — no such operation exists.

## Dependencies

- M12-002

## Implementation notes

Showing hard limits read-only is worth doing: it makes the safety boundary
visible to the operator and reinforces that nothing in the UI can change it.

Config drift needs to be visible rather than silent — that is the whole reason
M4-006 detects it.

## Acceptance criteria

- [ ] Device state and versions render.
- [ ] Sensor presence and health are distinguishable.
- [ ] **Config drift is shown when present.**
- [ ] Hard limits render read-only with no edit control.
- [ ] Events render with severity.
- [ ] Rename changes the display name only.

## Verification

```bash
cd ui/rhizo-ui && cargo tauri dev   # manual inspection
```

## Tests required

- Component tests: drift indicator, read-only limits.

## Documentation impact

- None.

## Files likely affected

```text
ui/rhizo-ui/src/views/device.rs
```
