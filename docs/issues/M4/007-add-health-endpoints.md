# Issue M4-007 — Implement /health/live and /health/ready

**Milestone:** M4 · **PRD:** [PRD 040](../../prd/040-device-registry-and-health.md) · **Depends on:** M4-001, M3-005

## Context

ADR-010 distinguishes liveness ('should the supervisor restart me?') from
readiness ('am I doing my job?'). Cloud reachability is deliberately excluded
from readiness — an edge with the cloud down is fully functional (SAFETY-008),
and reporting otherwise could trigger a pointless restart loop.

## Goal

Report health accurately and self-diagnostically.

## Scope

- `/health/live`: 200 while running with no task panic
- `/health/ready`: 200 only when migrations applied, MQTT **subscribed**, and the control loop ticked within 3 intervals
- A JSON body listing each check with its status
- The MQTT check tolerates a disconnect shorter than one backoff cycle
- **Cloud reachability excluded**

## Non-goals

- Cloud health as a readiness input — explicitly excluded.

## Dependencies

- M4-001
- M3-005

## Implementation notes

Readiness depends on `Subscribed`, not `Connected` — a connected but
unsubscribed edge receives nothing while looking healthy.

Until M6 exists there is no control loop; report that check as `not_applicable`
rather than fabricating a pass.

The flap tolerance matters: normal MQTT reconnects would otherwise make
readiness oscillate.

## Acceptance criteria

- [ ] `/health/live` returns 200 while running.
- [ ] `/health/ready` returns 200 in normal operation.
- [ ] `/health/ready` returns 503 with `mqtt: disconnected` when the broker is stopped.
- [ ] **`/health/ready` returns 200 when the cloud is stopped.**
- [ ] The body lists each check with a specific status.
- [ ] A brief reconnect does not flip readiness.

## Verification

```bash
curl -s localhost:8080/health/ready | jq
docker compose stop mosquitto && curl -i localhost:8080/health/ready   # 503
docker compose stop cloud-api && curl -i localhost:8080/health/ready   # 200
```

## Tests required

- Each check in isolation.
- **Cloud down yields ready.**
- Flap tolerance.

## Documentation impact

- None.

## Files likely affected

```text
crates/edge-controller/src/api/health.rs
```
