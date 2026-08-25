# Issue M7-003 — Implement the idempotent event ingestion endpoint

**Milestone:** M7 · **PRD:** [PRD 070](../../prd/070-cloud-sync-and-storage.md) · **Depends on:** M7-002

## Context

ADR-005. The design point that matters: **`duplicate` is a success**.
Returning an error for a replayed event would wedge the queue after every
ambiguous timeout — which post-outage is every request.

## Goal

Accept event batches idempotently with per-event results.

## Scope

- `POST /api/v1/edges/{edge_id}/events`, batch <= 500 and <= 5 MiB
- **HTTP 200 with per-event results even on partial failure**
- `accepted | duplicate | rejected` per event
- One transaction per batch with `ON CONFLICT DO NOTHING`
- A rejected event does not abort the batch
- **Unknown kinds stored in the ledger** and reported `rejected` for projection only
- 4xx only for a malformed request envelope; 5xx for server faults

## Non-goals

- Projections (M7-004).

## Dependencies

- M7-002

## Implementation notes

Storing unknown kinds in the ledger while reporting them as rejected is the
behaviour that guarantees history is never lost to a cloud that is behind the
edge's version. The projection can be added later and the data reprojected.

Rejecting the batch on one bad event would block the queue behind it forever —
the exact failure the per-event design prevents.

## Acceptance criteria

- [ ] A batch of new events returns all `accepted`.
- [ ] Re-POSTing the identical batch returns all `duplicate` and creates no rows.
- [ ] A malformed event returns `rejected` while the rest succeed.
- [ ] An unknown kind is **stored in `synced_events`** and reported `rejected`.
- [ ] A malformed request envelope returns 4xx.
- [ ] A batch over 500 events or 5 MiB is rejected.
- [ ] The batch is atomic.

## Verification

```bash
cargo test -p cloud-api ingest::
curl -s -X POST localhost:8081/api/v1/edges/home-01/events -d @batch.json | jq
```

## Tests required

- Accepted, duplicate, rejected paths.
- **Replay creates no rows.**
- Unknown kind stored in the ledger.
- Batch size limits.
- Atomicity.

## Documentation impact

- http-api-boundaries.md verified.

## Files likely affected

```text
crates/cloud-api/src/api/ingest.rs
```
