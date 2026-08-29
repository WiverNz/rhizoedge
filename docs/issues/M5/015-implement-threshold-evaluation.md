# Issue M5-015 — Implement warning and critical threshold evaluation

**Milestone:** M5 · **PRD:** [PRD 050](../../prd/050-plant-model-and-recommendations.md) · **Depends on:** M5-014

## Context

A warning and a control condition are different things. A temperature warning
must not imply actuation, and a critical threshold must raise a visible alert even
when nothing can act on it — which is the normal situation for a monitoring-only
plant.

## Goal

Evaluate thresholds per plant and raise events, without touching actuation.

## Scope

- Evaluate every bound kind against its policy on each tick
- Raise `threshold.warning` and `threshold.critical` events on crossing, with hysteresis and confirm duration honoured
- Crossings recorded once per transition, not once per tick
- Alerts raised regardless of whether the plant has an actuator
- Expose current threshold state per measurement in the plant API
- Metrics `threshold_crossings_total{kind,severity}`

## Non-goals

- Any actuation consequence — thresholds inform, they do not water.
- Notification delivery (M13-007).

## Dependencies

- M5-014

## Implementation notes

Keep this strictly separate from the irrigation gate. A critical ambient
temperature is real and worth alerting on, and it is **not** a reason to pump
water. Wiring thresholds into actuation would be a category error that the
role model (M5-013) exists to prevent.

Hysteresis and confirm duration apply here for the same reason they apply to
irrigation: a value hovering on a threshold otherwise produces an alert per tick,
and an operator who is alerted constantly stops reading alerts.

## Acceptance criteria

- [x] Warning and critical crossings raise events with the right severity.
- [x] A crossing raises one event per transition, not one per tick.
- [x] Hysteresis prevents oscillation at the boundary.
- [x] A monitoring-only plant raises critical alerts normally.
- [x] **No threshold crossing of any kind triggers actuation.**
- [x] Threshold state is visible per measurement in the plant API.

## Verification

```bash
cargo test -p rhizo-domain threshold::
cargo test --test integration threshold_alerts
```

## Tests required

- Crossing detection and hysteresis.
- One-event-per-transition.
- Monitoring-only alerting.
- An explicit test that thresholds never actuate.

## Documentation impact

- None.

## Files likely affected

```text
crates/domain/src/threshold.rs
crates/edge-controller/src/control/threshold.rs
```
