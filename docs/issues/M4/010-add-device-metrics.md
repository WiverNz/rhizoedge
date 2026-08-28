# Issue M4-010 — Add device lifecycle metrics

**Milestone:** M4 · **PRD:** [PRD 040](../../prd/040-device-registry-and-health.md) · **Depends on:** M4-004, M3-011

## Context

ADR-010's cardinality discipline permits `device_id` as a label on
`device_restarts_total` and nowhere else, because there the fleet-size
cardinality is exactly the point.

## Goal

Export the device metric set.

## Scope

- `devices_online`, `devices_offline` gauges
- `device_restarts_total{device_id}`
- Gauges updated by the liveness timer, not per message

## Non-goals

- Plant or control metrics (M5, M6).

## Dependencies

- M4-004
- M3-011

## Implementation notes

Update the gauges from the timer rather than on each message: gauges driven
by message arrival drift when a device goes silent, which is precisely when they
matter.

`device_id` appears here and on no other metric — the M3-012 cardinality test
guards that.

## Acceptance criteria

- [x] `devices_online` and `devices_offline` reflect actual state.
- [x] They update when a device goes silent, with no inbound message.
- [x] `device_restarts_total` increments on a `boot_id` change.
- [x] `device_id` appears on no other metric.
- [x] The cardinality test still passes.

## Verification

```bash
cargo test -p edge-controller metrics::devices
curl -s localhost:8080/metrics | grep devices_
```

## Tests required

- Gauge accuracy.
- Timer-driven update.
- Restart counter.

## Documentation impact

- ADR-010 catalogue verified.

## Files likely affected

```text
crates/edge-controller/src/metrics.rs
```
