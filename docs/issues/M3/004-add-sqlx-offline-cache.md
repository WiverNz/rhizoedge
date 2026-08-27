# Issue M3-004 — Add the sqlx offline query cache and its CI check

**Milestone:** M3 · **PRD:** [PRD 030](../../prd/030-edge-ingestion-and-storage.md) · **Depends on:** M3-003

## Context

`sqlx::query!` verifies SQL against a real schema at compile time. CI has no
database, so an offline cache is committed — and a stale cache produces
confusing compile errors, so CI must verify it is current.

## Goal

Commit the offline cache and prevent it going stale unnoticed.

## Scope

- `cargo sqlx prepare` output committed under `.sqlx/`
- A CI step verifying the cache matches the current queries
- The regeneration procedure documented

## Non-goals

- Replacing `query!` with runtime-checked `query` — that would discard the benefit.

## Dependencies

- M3-003

## Implementation notes

The CI check is `cargo sqlx prepare --check`, which fails if regeneration
would change anything. Without it, a developer who forgets to regenerate breaks
the build for everyone else with an error that does not mention the cache.

Document the regeneration steps prominently — this is the single most confusing
failure mode `sqlx` produces.

## Acceptance criteria

- [x] `.sqlx/` is committed.
- [x] CI verifies the cache is current.
- [x] Changing a query without regenerating fails CI.
- [x] The regeneration procedure is documented.

## Verification

```bash
export DATABASE_URL="sqlite://$PWD/data/edge.sqlite"
sqlx database create && sqlx migrate run --source migrations/edge
cargo sqlx prepare --workspace --check
```

## Tests required

- CI check itself; manual verification of the failure case.

## Documentation impact

- docs/testing/local-development.md section 9 already documents it; verify accurate.

## Files likely affected

```text
.sqlx/*.json
.github/workflows/ci.yml
```
