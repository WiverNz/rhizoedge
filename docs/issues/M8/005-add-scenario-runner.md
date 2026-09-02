# Issue M8-005 — Implement the scenario runner harness

**Milestone:** M8 · **PRD:** [PRD 080](../../prd/080-end-to-end-test-environment.md) · **Depends on:** M8-004

## Context

PRD 080: a Rust binary rather than a shell script, so assertions are typed and
failures are legible. It asserts on **observable state** — API responses,
database rows, and captured MQTT — never on log strings.

## Goal

Build the harness every scenario is written against.

## Scope

- A Rust binary with `--scenario <name>` and an all-scenarios default
- Helpers: API client, direct SQLite and PostgreSQL readers, an MQTT spy
- Container stop/start control
- Per-scenario isolation with a clean database
- Deterministic seeding
- **On failure: dump database rows and recent MQTT traffic**

## Non-goals

- A YAML scenario DSL — deferred until ~30 scenarios justify it (PRD 080).

## Dependencies

- M8-004

## Implementation notes

The failure dump is what makes CI failures diagnosable without local
reproduction. Collect: last 200 log lines per container, `measurements`,
`commands`, `watering_events`, `irrigation_state`, `pending_cloud_events`, and
captured MQTT.

Reading both databases directly is deliberate — asserting only through the API
would let an API bug hide a data bug.

Per-scenario isolation costs container restarts and removes an entire class of
order-dependent flake.

## Acceptance criteria

- [x] The runner executes one scenario or all of them.
- [x] Each scenario starts with a clean database.
- [x] The MQTT spy captures all traffic.
- [x] Container stop/start works.
- [x] Runs are deterministic under a fixed seed.
- [x] **A failure dumps database state and MQTT traffic.**
- [x] Exit code is non-zero on any failure.

## Verification

```bash
docker compose -f deploy/docker-compose.yml -f deploy/docker-compose.test.yml run --rm scenario-runner --list
```

## Tests required

- Harness self-test: a deliberately failing scenario produces a dump and a non-zero exit.

## Documentation impact

- None.

## Files likely affected

```text
test/scenarios/runner.rs
test/scenarios/harness/mod.rs
```
