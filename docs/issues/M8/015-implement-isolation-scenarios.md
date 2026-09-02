# Issue M8-015 — Implement device isolation scenarios

**Milestone:** M8 · **PRD:** [PRD 080](../../prd/080-end-to-end-test-environment.md) · **Depends on:** M8-006, M8-005, M6-019, M6-021

## Context

[failure-scenarios.md](../../testing/failure-scenarios.md) §J. True network
isolation is a different failure from a cloud outage and needs its own scenarios
in the assembled system.

## Goal

Prove mode-C behaviour end to end.

## Scope

- Compose support for isolating one device from the broker without stopping it
- SCEN-090 Wi-Fi loss while monitoring, no policy
- SCEN-091 Wi-Fi loss before dryness, automation enabled
- SCEN-092 Wi-Fi loss during a commanded dose
- SCEN-093, SCEN-094 missing and corrupt policy
- SCEN-107 long isolation with the edge host down
- SCEN-077 reconnect refuses commands until a fresh `edge.time` is applied

## Non-goals

- Reconciliation scenarios (M8-016).

## Dependencies

- M8-006
- M8-005
- M6-019
- M6-021

## Implementation notes

Isolate at the network layer, not by killing the simulator. The device must keep
running, keep sampling, and keep evaluating — that is the whole point of mode C,
and stopping the container would test a different thing entirely.

The evaluation exercised here is the shared evaluator implemented and wired
into the simulator by M6-019. M2 supplied isolation mechanics only; no scenario
may treat M2 by itself as the source of autonomous decisions.

SCEN-107 stops the **edge**, not just the broker. A dead edge host is the failure
an owner actually experiences, and it is the scenario that justifies the whole
feature.

## Acceptance criteria

- [x] A device can be network-isolated while still running.
- [x] All seven listed scenarios pass.
- [x] SCEN-090 shows a device with no policy never actuating.
- [x] SCEN-091 shows exactly one bounded dose delivered autonomously.
- [x] SCEN-107 shows the provisioned plant watered and the unprovisioned one not.
- [x] SCEN-077 shows autonomy unaffected while the reconnecting device still refuses a command until it is resynchronised.
- [x] Failures dump device state and buffered events.

## Verification

```bash
docker compose -f deploy/docker-compose.yml -f deploy/docker-compose.test.yml run --rm scenario-runner --scenario scenario_isolation
```

## Tests required

- SCEN-090, -091, -092, -093, -094, -107, -077.

## Documentation impact

- failure-scenarios.md verified.

## Files likely affected

```text
test/scenarios/isolation.rs
deploy/docker-compose.test.yml
```
