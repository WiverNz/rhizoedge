# Issue M8-001 — Add Dockerfiles for the Rust services

**Milestone:** M8 · **PRD:** [PRD 080](../../prd/080-end-to-end-test-environment.md) · **Depends on:** M7-015

## Context

M8 requires one command to start the full topology. That needs images, built
reproducibly and small enough to rebuild often during development.

## Goal

Containerise the edge controller, simulator, and cloud API.

## Scope

- Multi-stage Dockerfiles with `cargo chef` or an equivalent dependency cache
- Slim runtime images (distroless or debian-slim)
- Non-root user
- Correct signal handling so SIGTERM reaches the process
- `.dockerignore` excluding `target/` and local data

## Non-goals

- The UI image — it is a desktop app (ADR-009).

## Dependencies

- M7-015

## Implementation notes

Signal handling is the one that bites: without an init or correct PID 1
behaviour, `docker compose stop` sends SIGTERM to a shell rather than the
binary, and the graceful shutdown path (M3-014) never runs. Test it explicitly.

Dependency caching matters here more than image size — the whole suite is
rebuilt on every change during M8 development.

## Acceptance criteria

- [ ] All three images build.
- [ ] A dependency-only change rebuilds quickly (cache hit).
- [ ] Images run as non-root.
- [ ] **SIGTERM reaches the process and triggers graceful shutdown.**
- [ ] Images are under 100 MB.
- [ ] `.dockerignore` excludes build artefacts.

## Verification

```bash
docker compose build
docker compose up -d edge-controller && docker compose stop edge-controller
docker compose logs edge-controller | grep 'graceful shutdown'
```

## Tests required

- Build succeeds.
- **Graceful shutdown on `docker compose stop`.**

## Documentation impact

- None.

## Files likely affected

```text
deploy/edge/Dockerfile
deploy/cloud/Dockerfile
deploy/simulator/Dockerfile
.dockerignore
```
