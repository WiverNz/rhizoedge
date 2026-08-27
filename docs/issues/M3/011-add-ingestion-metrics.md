# Issue M3-011 — Add ingestion metrics

**Milestone:** M3 · **PRD:** [PRD 030](../../prd/030-edge-ingestion-and-storage.md) · **Depends on:** M3-009, M0-006

## Context

ADR-010 states that a failure is not handled until it is observable. The
ingestion metrics are the first concrete entries in the catalogue.

## Goal

Export the ingestion metric set.

## Scope

- `mqtt_messages_received_total{kind}`, `mqtt_decode_errors_total{reason}`, `mqtt_duplicate_messages_total{kind}`, `mqtt_reconnects_total`, `mqtt_connection_state`
- `measurements_processed_total{kind}`, `sensor_errors_total{sensor,reason}`
- `sqlite_busy_total`, `storage_bytes`, `task_panics_total{task}`
- `mqtt_processing_duration_seconds` histogram
- Names as constants in `rhizo-telemetry::names`

## Non-goals

- Device or control metrics (M4-010, M6-014).

## Dependencies

- M3-009
- M0-006

## Implementation notes

**No `device_id` label on any of these.** ADR-010's cardinality discipline
limits `device_id` to `device_restarts_total`, where the fleet-size cardinality
is the point. Adding it here multiplies every series by the device count.

`storage_bytes` is sampled periodically rather than computed per request.

## Acceptance criteria

- [x] Every listed metric is exported.
- [x] Names come from constants, not string literals.
- [x] No `device_id` label appears on any ingestion metric.
- [x] `/metrics` output parses as valid exposition format.
- [x] Counters increment in the expected scenarios.

## Verification

```bash
cargo test -p edge-controller metrics::
curl -s localhost:8080/metrics | promtool check metrics
```

## Tests required

- Each counter increments in its scenario.
- Exposition format validity.

## Documentation impact

- ADR-010's catalogue verified accurate.

## Files likely affected

```text
crates/edge-controller/src/metrics.rs
crates/telemetry/src/names.rs
```
