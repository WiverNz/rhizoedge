# Issue M3-003 — Add the SQLite schema and migration runner

**Milestone:** M3 · **PRD:** [PRD 030](../../prd/030-edge-ingestion-and-storage.md) · **Depends on:** M3-002

## Context

ADR-004 defines the full edge schema. All tables are created in M3 even
though later milestones populate some of them — schema churn during feature work
is how migrations end up edited after being applied somewhere.

## Goal

Create every edge table with its indexes, and run migrations at startup.

## Scope

- Migrations in `migrations/edge/`, embedded via `sqlx::migrate!`
- Every table from ADR-004's schema section
- Indexes: `idx_meas_device_time`, `idx_processed_received`, `idx_commands_open`, `idx_watering_plant_time`, `idx_devevents_device_time`
- Migrations run **before** any other subsystem; failure is Fatal
- An automatic backup when the schema version changes

## Non-goals

- Populating tables (M3-009 onward).

## Dependencies

- M3-002

## Implementation notes

All timestamps are `INTEGER` Unix epoch milliseconds UTC (ADR-013). No TEXT
timestamps anywhere.

`commands.command_id` is the **primary key**, not a surrogate id with a unique
index — that makes a duplicate insert fail at the storage layer rather than
relying on application code to check first.

The pre-migration backup is a plain file copy after a WAL checkpoint, taken only
when the version actually changes.

## Acceptance criteria

- [x] All tables from ADR-004 exist with the documented columns.
- [x] All five indexes exist.
- [x] Migrations are idempotent — running twice is a no-op.
- [x] A failing migration exits the process non-zero.
- [x] A backup file appears when the schema version changes.
- [x] `commands.command_id` is the primary key.
- [x] Every timestamp column is INTEGER.

## Verification

```bash
cargo test -p rhizo-storage migrate::
sqlite3 ./data/edge.sqlite '.schema' | grep -c 'INTEGER NOT NULL'
```

## Tests required

- Idempotency.
- Fatal on failure.
- Backup created on version change.
- A schema assertion test comparing against ADR-004's column list.

## Documentation impact

- None; ADR-004 is normative.

## Files likely affected

```text
migrations/edge/0001_initial.sql
crates/storage/src/migrate.rs
```
