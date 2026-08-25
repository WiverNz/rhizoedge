# Issue M7-002 — Add the PostgreSQL schema and migrations

**Milestone:** M7 · **PRD:** [PRD 070](../../prd/070-cloud-sync-and-storage.md) · **Depends on:** M7-001

## Context

ADR-005's two-layer design: `synced_events` is the append-only ledger and the
idempotency boundary; the projections are derived views maintained in the same
transaction and rebuildable from the ledger.

## Goal

Create the cloud schema with its idempotency constraint.

## Scope

- Migrations in `migrations/cloud/`
- `edge_instances`, `synced_events`, and the projection tables
- **`UNIQUE (edge_id, event_id)` on `synced_events`**
- Every projection table carries `edge_id` in its primary key
- `TIMESTAMPTZ` for time, `JSONB` for payloads
- Migrations run at startup; failure is Fatal

## Non-goals

- Projection logic (M7-004).

## Dependencies

- M7-001

## Implementation notes

`edge_id` in the unique key rather than trusting `event_id` alone: a
misconfigured edge cloning another's database must not corrupt a neighbour's
history.

Partitioning by `edge_id` from day one even though V1 has one edge. Retrofitting
a tenant key into a schema that already has history is far more expensive than
carrying a column now.

Storing measurement data twice (JSONB ledger plus projection columns) is
deliberate — it buys rebuildability, which is worth more than the disk.

## Acceptance criteria

- [ ] All tables exist with the documented columns.
- [ ] `UNIQUE (edge_id, event_id)` is present on `synced_events`.
- [ ] Every projection table includes `edge_id` in its primary key.
- [ ] Migrations are idempotent.
- [ ] A failing migration exits non-zero.
- [ ] Time columns are `TIMESTAMPTZ`, payloads `JSONB`.

## Verification

```bash
cargo test -p cloud-api migrate::
docker compose exec postgres psql -U rhizo -c '\d synced_events'
```

## Tests required

- Idempotency.
- Unique constraint enforced.
- Fatal on failure.

## Documentation impact

- ADR-005 verified accurate.

## Files likely affected

```text
migrations/cloud/0001_initial.sql
crates/cloud-api/src/db.rs
```
