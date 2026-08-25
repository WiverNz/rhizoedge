# Issue M7-009 — Add cloud synchronisation metrics

**Milestone:** M7 · **PRD:** [PRD 070](../../prd/070-cloud-sync-and-storage.md) · **Depends on:** M7-008

## Context

ADR-010: `pending_cloud_events` and `cloud_last_success_timestamp_seconds` are
two of the three most operationally valuable series in the system.

## Goal

Make sync health obvious at a glance.

## Scope

- `pending_cloud_events` gauge, `cloud_sync_attempts_total{outcome}`
- `cloud_sync_duration_seconds`, `cloud_events_quarantined_total`
- `cloud_events_dropped_total`, `cloud_last_success_timestamp_seconds`
- `GET /api/v1/sync/status` and `/sync/quarantined`

## Non-goals

- Alerting rules (a deployment concern, M13).

## Dependencies

- M7-008

## Implementation notes

`cloud_last_success_timestamp_seconds` answers 'how long has sync been
broken?' directly, which a counter cannot. It is the series that turns
retry-forever from a silent behaviour into a visible one.

The `/sync/quarantined` endpoint exists because quarantined events need operator
attention and nothing drains them automatically.

## Acceptance criteria

- [ ] All metrics are exported and accurate.
- [ ] `pending_cloud_events` tracks the real backlog.
- [ ] `cloud_last_success_timestamp_seconds` updates on success.
- [ ] The sync endpoints return the documented shapes.
- [ ] The cardinality guard still passes.

## Verification

```bash
cargo test -p edge-controller metrics::cloud
curl -s localhost:8080/api/v1/sync/status | jq
```

## Tests required

- Metric accuracy.
- Endpoint shapes.

## Documentation impact

- ADR-010 catalogue verified.

## Files likely affected

```text
crates/edge-controller/src/metrics.rs
crates/edge-controller/src/api/sync.rs
```
