# Issue M6-009 — Implement water command publication

**Milestone:** M6 · **PRD:** [PRD 060](../../prd/060-irrigation-control-and-safety.md) · **Depends on:** M6-008

## Context

Protocol section 5.8. The command is published QoS 1, never retained — a
retained command would be redelivered on every reconnect indefinitely.

## Goal

Publish commands to devices correctly.

## Scope

- Build the `command.water` envelope from the persisted row
- Publish QoS 1, **retain false**
- Record `published_at`
- Transition to `DoseIssued` / `WaitingForResult`
- Tare and calibrate commands on the same path

## Non-goals

- Retry semantics (M6-011).

## Dependencies

- M6-008

## Implementation notes

`retain = false` on every command publish, asserted by a test. ADR-002 calls
retaining a command topic the single most damaging mistake available in this
protocol.

Build the envelope from the **persisted row**, not from the in-memory decision,
so what is published is exactly what was recorded.

## Acceptance criteria

- [ ] A command is published with a valid envelope and QoS 1.
- [ ] `retain` is false.
- [ ] `published_at` is recorded.
- [ ] The published `command_id` matches the persisted row.
- [ ] Tare and calibrate use the same path.
- [ ] No retained message appears on any command topic after a cycle.

## Verification

```bash
cargo test -p edge-controller command::publish
cargo test --test integration no_retained_commands
```

## Tests required

- Envelope validity.
- **retain=false assertion.**
- Row/publish consistency.
- SCEN-015.

## Documentation impact

- None.

## Files likely affected

```text
crates/edge-controller/src/control/publish.rs
```
