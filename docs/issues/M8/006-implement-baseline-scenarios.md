# Issue M8-006 — Implement the baseline operation scenarios

**Milestone:** M8 · **PRD:** [PRD 080](../../prd/080-end-to-end-test-environment.md) · **Depends on:** M8-005

## Context

SCEN-001, -002, -003. The full watering cycle is the scenario the whole
project builds toward.

## Goal

Prove normal operation end to end.

## Scope

- SCEN-001 normal telemetry ingestion
- SCEN-002 full watering cycle with the exact documented state sequence
- SCEN-003 recommendation without automation — **zero commands published**

## Non-goals

- Failure scenarios (M8-007 onward).

## Dependencies

- M8-005

## Implementation notes

SCEN-002 must assert the **exact** state sequence, not merely that the plant
ended up healthy. A cycle that reached the right end state by the wrong path
would hide a real defect.

SCEN-003's zero-command assertion uses the MQTT spy, which is stronger than
checking the database — it proves nothing was published even transiently.

## Acceptance criteria

- [ ] SCEN-001 passes: telemetry stored with correct counts.
- [ ] SCEN-002 passes with the exact state sequence asserted.
- [ ] SCEN-002 never exceeds `max_daily_ml`.
- [ ] Every watering event has a matching terminal command.
- [ ] SCEN-003 passes with **zero MQTT commands captured**.
- [ ] All three complete within the accelerated time budget.

## Verification

```bash
docker compose -f deploy/docker-compose.yml -f deploy/docker-compose.test.yml run --rm scenario-runner --scenario scenario_full_watering_cycle
```

## Tests required

- SCEN-001, SCEN-002, SCEN-003.

## Documentation impact

- failure-scenarios.md verified.

## Files likely affected

```text
test/scenarios/baseline.rs
```
