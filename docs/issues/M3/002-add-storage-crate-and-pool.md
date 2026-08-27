# Issue M3-002 — Create rhizo-storage with the SQLite pool

**Milestone:** M3 · **PRD:** [PRD 030](../../prd/030-edge-ingestion-and-storage.md) · **Depends on:** M3-001

## Context

ADR-004 specifies SQLite via `sqlx` in WAL mode with a single writer. The
pragmas are not incidental: WAL plus `synchronous = NORMAL` is what makes crash
recovery correct while keeping SD-card wear tolerable.

## Goal

Provide the connection pool with the correct pragmas and a transaction API.

## Scope

- `EdgeDb::connect(path)` creating the database if absent
- Pragmas: `journal_mode=WAL`, `synchronous=NORMAL`, `busy_timeout=5000`, `foreign_keys=ON`
- `begin()` returning a transaction
- Repository API shape: writes take `&mut Transaction`, reads take `&pool`
- An in-memory constructor for tests

## Non-goals

- The schema (M3-003).
- Repository methods (M3-009).

## Dependencies

- M3-001

## Implementation notes

`foreign_keys=ON` must be set per connection, not once — SQLite defaults it
off and the pool opens several connections.

The single-writer discipline is enforced by API shape rather than by types:
only the pipeline owns something that can `begin()`. Document that, since the
compiler will not.

## Acceptance criteria

- [x] `connect` creates the file and applies all four pragmas.
- [x] Pragmas are verified on a **second** pooled connection, not just the first.
- [x] `begin`/`commit`/`rollback` work.
- [x] The in-memory constructor works for tests.
- [x] Concurrent readers proceed during a write (WAL confirmed).

## Verification

```bash
cargo test -p rhizo-storage pool::
```

## Tests required

- Pragma verification across two connections.
- Transaction commit and rollback.
- Concurrent read during write.

## Documentation impact

- Crate docs stating the single-writer rule.

## Files likely affected

```text
crates/storage/src/lib.rs
crates/storage/src/pool.rs
```
