# Issue M6-010 — Implement command result handling

**Milestone:** M6 · **PRD:** [PRD 060](../../prd/060-irrigation-control-and-safety.md) · **Depends on:** M6-009

## Context

Protocol section 5.10. The result settles the command and creates the
watering event — or explicitly does not, for outcomes that delivered nothing.

## Goal

Process results and update state and history correctly.

## Scope

- Handle `command.result` through the M3 dedup path
- `completed`: create a `watering_event` with `delivered_ml`, transition to `WaitForAbsorption`
- `rejected`: record the reason, **no watering event**, transition to `Recheck`
- `interrupted`/`failed`: credit `requested_ml`, **no watering event**, transition to `Recheck`
- A result for an unknown `command_id` is logged and ignored
- All updates in one transaction

## Non-goals

- No-delivery detection (M6-017).

## Dependencies

- M6-009

## Implementation notes

A rejected or failed command must **never** create a watering event. A
watering event asserts that water reached the plant; recording one for a refused
command would corrupt the daily total and the cooldown in the permissive
direction.

An unknown `command_id` is logged, not invented into existence — the edge does
not create a command row to match a result it did not issue.

## Acceptance criteria

- [ ] `completed` creates exactly one `watering_event`.
- [ ] `rejected` creates **none** and records the reason.
- [ ] `interrupted` creates none but credits `requested_ml` to the budget.
- [ ] An unknown `command_id` is ignored with a log.
- [ ] A duplicate result creates no second event.
- [ ] All updates are atomic.

## Verification

```bash
cargo test -p edge-controller command::result
cargo test safety_001
```

## Tests required

- Each result status.
- **No event on rejection or failure.**
- Duplicate result idempotency.
- Unknown command_id handling.

## Documentation impact

- None.

## Files likely affected

```text
crates/edge-controller/src/control/result.rs
```
