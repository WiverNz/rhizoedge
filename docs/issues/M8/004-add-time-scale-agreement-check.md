# Issue M8-004 — Assert time-scale agreement across services

**Milestone:** M8 · **PRD:** [PRD 080](../../prd/080-end-to-end-test-environment.md) · **Depends on:** M8-003

## Context

ADR-013: mixing accelerated and real time across a topology corrupts the
rolling-window computation, and the resulting failures are extremely confusing
to diagnose.

## Goal

Fail loudly at startup when services disagree about time.

## Scope

- Both the edge and simulator read the scale from one Compose variable
- Each reports its scale at startup and via an endpoint
- The scenario runner asserts agreement before running anything
- A mismatch fails immediately with a clear message

## Non-goals

- Runtime scale changes.

## Dependencies

- M8-003

## Implementation notes

Assert **before** the first scenario, not during. A mismatch discovered
halfway through a suite wastes a full run and produces failures that look like
logic bugs.

The error message should name both values and the variable, because the fix is
always a Compose configuration change.

## Acceptance criteria

- [ ] Both services report their scale at startup.
- [ ] The runner asserts agreement before any scenario.
- [ ] A deliberate mismatch fails immediately with a clear message.
- [ ] Both read from one Compose variable.
- [ ] The scale is queryable at runtime.

## Verification

```bash
curl -s localhost:9090/sim/scale
curl -s localhost:8080/api/v1/overview | jq .time_scale
```

## Tests required

- Agreement check.
- Mismatch detection.

## Documentation impact

- None.

## Files likely affected

```text
deploy/docker-compose.test.yml
crates/edge-controller/src/api/overview.rs
test/scenarios/runner.rs
```
