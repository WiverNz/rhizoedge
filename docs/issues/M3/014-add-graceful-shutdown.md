# Issue M3-014 — Implement graceful shutdown and startup state restoration

**Milestone:** M3 · **PRD:** [PRD 030](../../prd/030-edge-ingestion-and-storage.md) · **Depends on:** M3-009

## Context

SAFETY-010 requires a restart to be safe. M3 has no commands yet, but the
restoration path is established now so M6 extends it rather than inventing it.

## Goal

Shut down cleanly and restore state from SQLite at startup.

## Scope

- SIGTERM/Ctrl-C stops intake, finishes the in-flight transaction, closes the pool, exits 0
- A shutdown timeout after which the process exits anyway
- At startup: migrate, then restore the device registry from SQLite
- A startup INFO log summarising what was restored
- Never construct defaults for a row that exists

## Non-goals

- Command reconciliation (M6-012).

## Dependencies

- M3-009

## Implementation notes

'Never construct defaults for existing rows' is the habit that matters. In M6
the same principle applies to irrigation state, and a plant reset to `Normal` on
restart would silently drop a cooldown or an absorption wait.

The startup log is the operator's evidence that recovery worked; make it list
counts rather than being a bare 'started'.

## Acceptance criteria

- [x] SIGTERM exits 0 with no partial transaction.
- [x] A hung task does not prevent exit past the timeout.
- [x] Startup restores the device registry from SQLite.
- [x] The startup log reports what was restored.
- [x] Ingest 50, restart, ingest 50 yields 100 rows with no duplicates.

## Verification

```bash
cargo test --test integration restart_preserves_history
cargo run -p edge-controller & kill -TERM $!
```

## Tests required

- SCEN-050 restart preserves history.
- Clean shutdown.
- Shutdown timeout with a hung task.

## Documentation impact

- None.

## Files likely affected

```text
crates/edge-controller/src/shutdown.rs
crates/edge-controller/src/startup.rs
```
