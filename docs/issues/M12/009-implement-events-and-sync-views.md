# Issue M12-009 — Implement the events and sync views

**Milestone:** M12 · **PRD:** [PRD 120](../../prd/120-rust-ui.md) · **Depends on:** M12-002

## Context

Device events and cloud sync status are the operator's window into what the
system noticed and whether history is reaching the cloud.

## Goal

Expose diagnostics without a terminal.

## Scope

- Device events with severity, filterable by device and kind
- Sync status: pending count, last success, quarantined events
- Quarantined messages listing
- **Critical events visually distinguished**

## Non-goals

- Acting on quarantined items — inspection only in V1.

## Dependencies

- M12-002

## Implementation notes

A leak event is the highest-severity thing this system produces; it should
be unmistakable in a list. Severity styling is the mechanism.

Sync status matters because a silently broken cloud sync loses history without
any local symptom — the pending count and last-success time are the only
signals.

## Acceptance criteria

- [ ] Events render with severity and are filterable.
- [ ] **Critical events are visually distinct.**
- [ ] Sync status shows pending count and last success.
- [ ] Quarantined events and messages are listed.
- [ ] Cloud-disabled is shown as disabled, not as an error.

## Verification

```bash
cd ui/rhizo-ui && cargo tauri dev   # manual inspection
```

## Tests required

- Severity styling.
- Filtering.
- Cloud-disabled rendering.

## Documentation impact

- None.

## Files likely affected

```text
ui/rhizo-ui/src/views/events.rs
ui/rhizo-ui/src/views/sync.rs
```
