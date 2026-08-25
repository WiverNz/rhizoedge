# Issue M3-010 — Add the latest-sample read cache

**Milestone:** M3 · **PRD:** [PRD 030](../../prd/030-edge-ingestion-and-storage.md) · **Depends on:** M3-009

## Context

The read API and the control loop both need the latest sample per device. The
API can use a cache; the control loop reads from SQLite regardless (ADR-006), so
the cache is an optimisation and never a source of truth.

## Goal

Serve latest-sample reads without a query per request.

## Scope

- An in-memory map of device to latest sample, refreshed on ingest
- Rebuilt from SQLite at startup
- Explicitly documented as **not** authoritative
- Cache updated inside the pipeline after a successful commit

## Non-goals

- Using it for control decisions — forbidden by ADR-006.

## Dependencies

- M3-009

## Implementation notes

Update the cache **after** the transaction commits, not inside it. Updating
first would leave the cache ahead of the database if the commit fails, and the
API would report a measurement that does not exist.

Document the non-authoritative status in the type's doc comment; the risk is a
future contributor reaching for it in the control loop because it is convenient.

## Acceptance criteria

- [ ] The cache reflects the newest sample after ingest.
- [ ] It is rebuilt correctly at startup.
- [ ] A failed transaction does not update it.
- [ ] The doc comment states it is not authoritative.
- [ ] `grep` shows no use of it in any control path.

## Verification

```bash
cargo test -p edge-controller cache::
```

## Tests required

- Refresh on ingest.
- Rebuild at startup.
- No update on rollback.

## Documentation impact

- Doc comment on the cache type.

## Files likely affected

```text
crates/edge-controller/src/state/cache.rs
```
