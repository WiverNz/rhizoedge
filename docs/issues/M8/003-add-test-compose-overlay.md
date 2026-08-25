# Issue M8-003 — Add the test Compose overlay

**Milestone:** M8 · **PRD:** [PRD 080](../../prd/080-end-to-end-test-environment.md) · **Depends on:** M8-002

## Context

Deployment-model section 1: the test overlay accelerates time, shortens
intervals consistently, and **disables restart policies** so a crash fails the
test rather than being papered over.

## Goal

Provide a deterministic, accelerated test topology.

## Scope

- `deploy/docker-compose.test.yml` overlaying the base file
- `RHIZO_SIM__TIME_SCALE` and the matching edge tick and TTL settings
- **`restart: "no"` on every service**
- A `scenario-runner` service
- Ephemeral volumes so each run starts clean

## Non-goals

- The scenarios themselves (M8-006 onward).

## Dependencies

- M8-002

## Implementation notes

Disabling restart is essential. With a restart policy, a crashing edge
silently recovers and the suite passes while hiding a real defect — which is the
opposite of what a test environment should do.

Intervals must scale together: a 600x clock with a real-time 30-second control
tick means the control loop runs once per five simulated hours.

## Acceptance criteria

- [ ] The overlay produces an accelerated topology.
- [ ] Time scale is consistent across the edge and simulator.
- [ ] **No service restarts on failure.**
- [ ] Each run starts with clean volumes.
- [ ] `--abort-on-container-exit --exit-code-from scenario-runner` works.
- [ ] A deliberately crashed edge **fails** the run.

## Verification

```bash
docker compose -f deploy/docker-compose.yml -f deploy/docker-compose.test.yml up --abort-on-container-exit --exit-code-from scenario-runner
```

## Tests required

- Overlay validity.
- **A crashed service fails the run.**

## Documentation impact

- docs/testing/local-development.md section 6 verified.

## Files likely affected

```text
deploy/docker-compose.test.yml
```
