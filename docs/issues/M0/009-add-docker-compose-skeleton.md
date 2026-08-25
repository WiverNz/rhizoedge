# Issue M0-009 — Add the Docker Compose skeleton

**Milestone:** M0 · **PRD:** [PRD 000](../../prd/000-platform-foundation.md) · **Depends on:** M0-008

## Context

The deployment model calls for one command to start the software topology.
M0 establishes the file with the services that exist so far; later milestones
add their own service rather than restructuring it.

## Goal

Create a working Compose file with Mosquitto, and placeholders wired for the services to come.

## Scope

- `deploy/docker-compose.yml` with the `mosquitto` service, healthcheck, and named volumes
- Service definitions for edge-controller, device-simulator, cloud-api, and postgres, commented until their Dockerfiles exist
- A `rhizo` network
- Env var wiring from `.env`

## Non-goals

- Dockerfiles for the Rust services (M8-001).
- The test overlay (M8-003).

## Dependencies

- M0-008

## Implementation notes

`docker compose config` must parse from this issue onward — it is part of
the CI gate. Commented-out services keep the file honest: it always reflects
what actually runs.

Mosquitto's healthcheck gates dependent services later. The edge tolerates a
missing broker anyway (failure-model 1.1); the healthcheck exists to keep
startup logs readable.

## Acceptance criteria

- [ ] `docker compose -f deploy/docker-compose.yml config` exits 0.
- [ ] `docker compose up mosquitto` starts a healthy broker.
- [ ] Volumes persist across `down`/`up` without `-v`.
- [ ] `down -v` resets cleanly.
- [ ] No secret appears literally in the Compose file.

## Verification

```bash
docker compose -f deploy/docker-compose.yml config
docker compose -f deploy/docker-compose.yml up -d mosquitto
docker compose -f deploy/docker-compose.yml ps
```

## Tests required

- `docker compose config` in CI (M0-012).

## Documentation impact

- docs/testing/local-development.md first-run section.

## Files likely affected

```text
deploy/docker-compose.yml
deploy/mosquitto/mosquitto.conf
.env.example
```
