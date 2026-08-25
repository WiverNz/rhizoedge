# Issue M5-012 — Add recommendation endpoints, evaluation loop, and metrics

**Milestone:** M5 · **PRD:** [PRD 050](../../prd/050-plant-model-and-recommendations.md) · **Depends on:** M5-009, M5-010

## Context

Recommendations are evaluated on a tick and exposed over HTTP. This issue
introduces the periodic evaluation loop that M6 extends into the control loop.

## Goal

Evaluate recommendations periodically and expose them.

## Scope

- A tick loop evaluating every plant
- Persist a recommendation row **only when the decision or reason set changes**
- `GET /plants/{id}/recommendation` per http-api-boundaries section 2.5
- Reasons rendered to human-readable text in one place
- Metrics: `plants_total`, `plant_state{state}`, `recommendations_total{decision}`, `manual_watering_detected_total`
- **No MQTT command is published anywhere in M5**

## Non-goals

- Issuing commands — that is M6, and its absence here is a checked property.

## Dependencies

- M5-009
- M5-010

## Implementation notes

Writing a recommendation row per tick would produce 2 880 rows per plant per
day recording that nothing happened. Persist on change.

Log at INFO only when a recommendation **changes**. A tick reaching the same
conclusion is not news (ADR-010).

Assert in a test that M5 publishes nothing to any command topic — the separation
between recommending and acting is what lets the engine be validated against a
real plant for a week before anything can pump.

## Acceptance criteria

- [ ] Recommendations are evaluated on the tick.
- [ ] A row is written only on change.
- [ ] The endpoint returns the documented shape with structured reasons.
- [ ] Reasons render to prose in exactly one place.
- [ ] Metrics are exported.
- [ ] **An integration test asserts zero MQTT command publishes during a full drying cycle.**

## Verification

```bash
cargo test -p edge-controller recommend::
cargo test --test integration no_commands_in_m5
curl -s localhost:8080/api/v1/plants/monstera-01/recommendation | jq
```

## Tests required

- Change-only persistence.
- Endpoint shape.
- **SCEN-003: zero commands published.**
- Metrics.

## Documentation impact

- http-api-boundaries.md verified.

## Files likely affected

```text
crates/edge-controller/src/control/tick.rs
crates/edge-controller/src/api/recommendation.rs
```
