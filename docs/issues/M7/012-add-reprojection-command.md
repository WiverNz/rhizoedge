# Issue M7-012 — Add the reprojection command

**Milestone:** M7 · **PRD:** [PRD 070](../../prd/070-cloud-sync-and-storage.md) · **Depends on:** M7-004

## Context

ADR-005 risk: projection drift after a bug. The two-layer design exists so
projections can be rebuilt from the ledger without asking the edge to resend
anything — but only if the tooling exists.

## Goal

Rebuild projections from the ledger and detect drift.

## Scope

- `cloud-api reproject --edge-id <id>` replaying `synced_events` in order
- A consistency check comparing ledger counts to projection counts
- Idempotent: reprojecting twice yields identical tables
- Progress reporting for large ledgers

## Non-goals

- Automatic drift repair.

## Dependencies

- M7-004

## Implementation notes

Reprojection must be safe to run against live data: replay into a
transaction, or into shadow tables that are swapped. A partially reprojected
table is worse than a slightly stale one.

The consistency check is the more valuable half — it turns 'we could rebuild' into
'we know whether we need to'.

## Acceptance criteria

- [x] Reprojection reproduces byte-identical projection tables.
- [x] Running it twice is idempotent.
- [x] The consistency check detects a deliberately corrupted projection.
- [x] It is safe against live data.
- [x] Progress is reported for large ledgers.

## Verification

```bash
cargo run -p cloud-api -- reproject --edge-id home-01
cargo test -p cloud-api reproject::
```

## Tests required

- Identical rebuild.
- Idempotency.
- Corruption detection.
- Safety under concurrent writes.

## Documentation impact

- None.

## Files likely affected

```text
crates/cloud-api/src/cmd/reproject.rs
```
