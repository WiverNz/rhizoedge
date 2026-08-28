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
- [x] Every statically known storage query uses `query!` / `query_as!` /
      `query_scalar!`, so the check has real subject matter. The whole
      `crates/storage/src/repo` layer is checked; the only exceptions are
      `PRAGMA` statements, which sqlx cannot describe, and `migrate.rs`, which
      queries the migrator's own bookkeeping table before it exists. Both carry
      their justification in a comment at the call site.

### Correction, post-M3

The first implementation satisfied the four original criteria with a single
checked query (`SELECT count(*) FROM devices`) against 81 runtime-checked
`sqlx::query(...)` calls — the exact substitution this issue's Non-goals
forbid. `cargo sqlx prepare --check` passed, and proved almost nothing: a
column renamed in a later migration would have compiled and failed at runtime.
The repository layer was converted afterwards, taking the cache from 1 query to
30, and the gate was verified by negative control rather than by assumption
(see [docs/reports/M3.md](../../reports/M3.md) §Negative controls).

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
