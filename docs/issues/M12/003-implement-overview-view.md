# Issue M12-003 — Implement the overview dashboard

**Milestone:** M12 · **PRD:** [PRD 120](../../prd/120-rust-ui.md) · **Depends on:** M12-002

## Context

The first screen: every plant, its state, and anything that needs attention.
`/api/v1/overview` exists specifically for this view.

## Goal

Show system state at a glance.

## Scope

- Plant summaries: name, state, moisture, lockout
- Device online count, plants locked out
- Cloud sync status: pending count and last success
- Control loop health
- **Lockouts most prominent**
- Polling every 5 seconds

## Non-goals

- Charts (M12-007).

## Dependencies

- M12-002

## Implementation notes

Prominence is a safety requirement, not a design preference (PRD 120
F-120-20). An operator scanning this screen must not be able to miss that a
plant is locked out.

Poll rather than push: telemetry arrives every 300 s and the control loop ticks
every 30 s, so sub-second latency has nothing to show.

## Acceptance criteria

- [ ] All plants appear with their state and latest moisture.
- [ ] **A lockout is the most visually prominent element.**
- [ ] Device and lockout counts are accurate.
- [ ] Cloud sync status is shown, including when the cloud is disabled.
- [ ] The view refreshes every 5 seconds.
- [ ] It remains legible with 20 plants.

## Verification

```bash
cd ui/rhizo-ui && cargo tauri dev   # manual inspection against a running edge
```

## Tests required

- Component test: lockout prominence.
- Data rendering.

## Documentation impact

- None.

## Files likely affected

```text
ui/rhizo-ui/src/views/overview.rs
```
