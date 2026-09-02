# Issue M8-016 — Implement offline autonomy and reconciliation scenarios

**Milestone:** M8 · **PRD:** [PRD 080](../../prd/080-end-to-end-test-environment.md) · **Depends on:** M8-015, M6-019, M6-020, M6-021

## Context

The reconnection seam, end to end: SAFETY-016 across real process boundaries
rather than in a unit test.

## Goal

Prove reconciliation is exactly-once and blocks premature dosing.

## Scope

- SCEN-095 policy update interrupted at every step
- SCEN-096 budget respected across isolation
- SCEN-097 device restart while isolated
- SCEN-098 isolated device with no wall clock
- SCEN-099, SCEN-105 required versus advisory measurement
- SCEN-100, SCEN-101, SCEN-102 reconciliation, duplicate replay, edge restart mid-replay
- SCEN-103 stale policy version after reconnect
- SCEN-104 buffer overflow gap

## Non-goals

- Firmware behaviour (M9 verifies the same properties on hardware).

## Dependencies

- M8-015
- M6-019
- M6-020
- M6-021

## Implementation notes

SCEN-102 is the hardest to get right: kill the edge deterministically midway
through a replay. Use a fault hook in the edge rather than racing it externally,
for the same reason SCEN-051 does.

Assert with an MQTT spy that **no command is published** while a plant is
reconciling. Checking the database afterwards is weaker — a command that was
published and then rolled back would still have reached the device.

## Acceptance criteria

- [x] All listed scenarios pass.
- [x] SCEN-100 shows autonomous doses folded into the budget exactly once.
- [x] SCEN-101 shows triple replay producing one event per `event_id`.
- [x] SCEN-102 shows an edge restart mid-replay losing nothing and duplicating nothing.
- [x] **No command is published during reconciliation**, asserted by spy.
- [x] SCEN-104 shows an explicit gap in history.
- [x] The suite still completes within its time budget.

## Verification

```bash
docker compose -f deploy/docker-compose.yml -f deploy/docker-compose.test.yml run --rm scenario-runner --scenario scenario_reconciliation
```

## Tests required

- SCEN-095…SCEN-105.

## Documentation impact

- failure-scenarios.md verified.

## Files likely affected

```text
test/scenarios/reconciliation.rs
```
