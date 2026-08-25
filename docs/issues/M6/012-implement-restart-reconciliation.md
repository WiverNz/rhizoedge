# Issue M6-012 — Implement command reconciliation at startup

**Milestone:** M6 · **PRD:** [PRD 060](../../prd/060-irrigation-control-and-safety.md) · **Depends on:** M6-010

## Context

SAFETY-010's recovery procedure. Restarting the edge — at any point,
including mid-dose — must never re-issue a completed command or double-count a
watering event.

## Goal

Reconcile in-flight commands and restore irrigation state on boot.

## Scope

- Load commands with status `issued` or `in_flight`
- `expires_at < now`: mark `expired`, transition to `Recheck`
- Otherwise: mark `in_flight`, await a result until `expires_at`
- **Never re-publish a command that already has a `command_id` on the wire**
- Restore irrigation state from SQLite — never construct defaults for an existing plant
- Log a recovery summary at INFO

## Non-goals

- Device-side recovery (M9-013).

## Dependencies

- M6-010

## Implementation notes

'Never construct a default for an existing plant' is the rule that preserves
an in-progress absorption wait across a restart. A plant reset to `Normal` would
silently drop its cooldown and its dose count.

Not re-publishing is equally important: the original command may well have been
delivered, and republishing under a new id would double-water.

The recovery log is the operator's evidence; list counts, not a bare 'started'.

## Acceptance criteria

- [ ] Expired in-flight commands become `expired` and move to `Recheck`.
- [ ] Live in-flight commands are awaited until `expires_at`.
- [ ] **No command is re-published after a restart.**
- [ ] Irrigation state including `wait_until` and `doses_this_cycle` is restored exactly.
- [ ] A restart during `WaitForAbsorption` resumes with the original `wait_until`.
- [ ] The recovery summary is logged.
- [ ] **`safety_010_restart_mid_command_no_replay` passes.**

## Verification

```bash
cargo test -p edge-controller startup::reconcile
cargo test safety_010
cargo test --test integration restart_mid_command
```

## Tests required

- **`safety_010_restart_mid_command_no_replay`** — kill after publish, restart, assert one command and one event.
- `safety_010_terminal_commands_never_reissued` property test.
- SCEN-052 restart mid-absorption preserves `wait_until`.

## Documentation impact

- None.

## Files likely affected

```text
crates/edge-controller/src/startup.rs
crates/edge-controller/src/control/reconcile.rs
```
