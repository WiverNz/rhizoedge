# Issue M10-009 — Add sensor metrics and diagnostic events

**Milestone:** M10 · **PRD:** [PRD 100](../../prd/100-real-soil-sensor.md) · **Depends on:** M10-007

## Context

ADR-010: a failure is not handled until it is observable. Real sensors produce
real faults that need diagnosis at a distance.

## Goal

Make sensor behaviour observable from the edge.

## Scope

- `sensor_errors_total{sensor,reason}` extended with the new reasons
- `sensor_read_duration_seconds{sensor}`
- Events: `sensor_invalid`, `sensor_stuck`, `sensor_unhealthy`, `calibration_missing`
- Raw alongside converted values logged at DEBUG for calibration work

## Non-goals

- Alerting.

## Dependencies

- M10-007

## Implementation notes

Logging raw values at DEBUG is what makes field calibration possible without
a serial console — the operator can enable DEBUG briefly and see what the probe
is actually reporting.

Read duration is a useful early warning for a degrading bus: CRC errors rise
after latency does.

## Acceptance criteria

- [ ] All error reasons are counted with correct labels.
- [ ] Read duration is measured.
- [ ] Each event kind is raised under its condition.
- [ ] Raw values appear at DEBUG.
- [ ] The cardinality guard still passes.

## Verification

```bash
curl -s localhost:8080/metrics | grep sensor_
curl -s localhost:8080/api/v1/devices/plant-node-01/events | jq
```

## Tests required

- Counter labels.
- Event emission.

## Documentation impact

- ADR-010 catalogue verified.

## Files likely affected

```text
firmware/esp32-node/src/sensors/health.rs
crates/edge-controller/src/metrics.rs
```
