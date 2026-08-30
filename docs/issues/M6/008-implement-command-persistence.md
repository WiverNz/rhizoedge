# Issue M6-008 — Implement command persistence and lifecycle

**Milestone:** M6 · **PRD:** [PRD 060](../../prd/060-irrigation-control-and-safety.md) · **Depends on:** M6-007

## Context

SAFETY-010's mechanism. The command row is committed with status `issued`
**before** the MQTT publish. The reverse order would allow a pump to run with no
record that it was ever asked to.

## Goal

Persist commands transactionally with their state transition.

## Scope

- Insert `commands` with `command_id` as primary key, status `issued`
- `expires_at = issued_at + profile.command_ttl` (default 120 s)
- The insert, the irrigation state transition, and the outbox row in **one transaction**, committed before publishing
- Terminal statuses: completed, rejected, expired, failed, interrupted
- A result for a terminal command is ignored

## Non-goals

- Publishing (M6-009).
- Restart reconciliation (M6-012).

## Dependencies

- M6-007

## Implementation notes

`command_id` as the primary key rather than a surrogate id means a duplicate
insert fails at the storage layer, so the guarantee holds even if someone later
writes a check-then-insert race.

Commit before publish. Write a test that kills the process between the two and
asserts the row exists with no result — that is the state M6-012 reconciles.

## Acceptance criteria

- [x] A command row is committed with status `issued` before any publish.
- [x] The insert, transition, and outbox row share one transaction.
- [x] A duplicate `command_id` insert fails at the storage layer.
- [x] `expires_at` is computed from the profile TTL.
- [x] A result for an already-terminal command changes nothing.
- [x] A crash between commit and publish leaves an `issued` row.

## Verification

```bash
cargo test -p edge-controller command::persist
cargo test safety_010
```

## Tests required

- Transactional atomicity.
- Duplicate primary key rejection.
- Terminal-status idempotency.
- Crash-between-commit-and-publish state.

## Documentation impact

- None.

## Files likely affected

```text
crates/edge-controller/src/control/command.rs
crates/storage/src/repo/command.rs
```
