# Issue M8-007 — Implement the MQTT failure scenarios

**Milestone:** M8 · **PRD:** [PRD 080](../../prd/080-end-to-end-test-environment.md) · **Depends on:** M8-006

## Context

SCEN-011, -012. The broker restart scenario asserts **re-subscription**, not
merely reconnection — the distinction that failure-model 1.2 calls out.

## Goal

Prove MQTT failure handling end to end.

## Scope

- SCEN-011 duplicate water command actuates once
- SCEN-012 broker restart with resubscription and recovery

## Non-goals

- Unit-level dedup, already covered in M3.

## Dependencies

- M8-006

## Implementation notes

SCEN-011 is the end-to-end proof of SAFETY-001 across process boundaries.
Assert three results (one real, two stored replays), one actuation, and one
watering event.

SCEN-012 must assert that telemetry **resumes** after the restart, which is the
only way to detect a missing re-subscribe.

## Acceptance criteria

- [x] SCEN-011: one actuation, three results, one watering event.
- [x] The daily total counts the dose once.
- [x] SCEN-012: telemetry resumes after the broker restart.
- [x] Retained status is redelivered.
- [x] If the outage exceeded the staleness window, the plant locked out and recovered.

## Verification

```bash
... run --rm scenario-runner --scenario scenario_duplicate_command scenario_broker_restart
```

## Tests required

- SCEN-011, SCEN-012.

## Documentation impact

- None.

## Files likely affected

```text
test/scenarios/mqtt_failures.rs
```
