# Issue M8-002 — Complete the Docker Compose topology

**Milestone:** M8 · **PRD:** [PRD 080](../../prd/080-end-to-end-test-environment.md) · **Depends on:** M8-001

## Context

Deployment-model section 1. The M0 skeleton had commented services; M8
enables all five with correct dependencies and health gating.

## Goal

Make `docker compose up --build` start the complete software system.

## Scope

- All five services enabled with healthchecks and `depends_on: condition: service_healthy`
- Named volumes for SQLite and PostgreSQL
- Ports exposed: edge 8080, cloud 8081, mosquitto 1883
- Environment from `.env`
- A documented reset procedure
- `device-simulator` scalable via replicas

## Non-goals

- The test overlay (M8-003).

## Dependencies

- M8-001

## Implementation notes

Health gating makes logs readable rather than being strictly required — the
edge tolerates a missing broker (failure-model 1.1) and that behaviour must not
regress just because Compose now waits.

Scaling the simulator needs a device id derived from the replica index; a
fixed id would make replicas collide on the broker.

## Acceptance criteria

- [x] `docker compose up --build` starts all five services.
- [x] Health gating produces clean startup logs.
- [x] `curl localhost:8080/api/v1/overview` works after startup.
- [x] Volumes persist across `down`/`up`.
- [x] `down -v` resets cleanly.
- [x] `--scale device-simulator=3` produces three distinct devices.
- [x] The edge still starts when the broker is deliberately unavailable.

## Verification

```bash
docker compose -f deploy/docker-compose.yml up --build -d
curl -s localhost:8080/api/v1/overview | jq
docker compose down -v
```

## Tests required

- Full startup.
- Scaling.
- Reset.
- Edge tolerates a missing broker.

## Documentation impact

- docs/testing/local-development.md first-run section verified.

## Files likely affected

```text
deploy/docker-compose.yml
```
