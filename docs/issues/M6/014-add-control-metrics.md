# Issue M6-014 — Add control and lockout metrics

**Milestone:** M6 · **PRD:** [PRD 060](../../prd/060-irrigation-control-and-safety.md) · **Depends on:** M6-006, M3-011

## Context

ADR-010: `plants_locked_out` is one of the three most operationally valuable
series in the system — it answers 'is anything stuck?'.

## Goal

Export the control metric set.

## Scope

- `watering_commands_total{mode,outcome}`, `watering_delivered_ml_total{mode}`
- `watering_failures_total{reason}`, `irrigation_state_transitions_total{from,to}`
- `plants_locked_out` gauge, `lockouts_total{reason}`
- `control_tick_duration_seconds` histogram
- Every state transition persisted as an event

## Non-goals

- Cloud metrics (M7-009).

## Dependencies

- M6-006
- M3-011

## Implementation notes

Persisting every transition is what makes 'what did the system think, and
when' reconstructable months later — the question actually asked when a plant
dies. It is a low-frequency write and worth the space.

`control_tick_duration_seconds` is the metric that signals when the single-loop
design needs revisiting at scale (M13).

Log at INFO for every dose issued, every result, and every lockout set or
cleared. These are world-changing events.

## Acceptance criteria

- [x] Every listed metric is exported.
- [x] `plants_locked_out` reflects actual lockouts.
- [x] Every state transition is persisted as an event.
- [x] Dose, result, and lockout events log at INFO.
- [x] The cardinality guard still passes.

## Verification

```bash
cargo test -p edge-controller metrics::control
curl -s localhost:8080/metrics | grep -E 'watering_|lockouts_|plants_locked'
```

## Tests required

- Counter increments per scenario.
- Gauge accuracy.
- Transition persistence.

## Documentation impact

- ADR-010 catalogue verified.

## Files likely affected

```text
crates/edge-controller/src/metrics.rs
```
