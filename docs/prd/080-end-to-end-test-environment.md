# PRD 080 — End-to-End Test Environment

**Milestone:** M8 · **Status:** DELIVERED · **Depends on:** M7

## Summary

Make the entire software system reproducible and verifiable with one command, no
hardware, and no manual steps — and prove the first major demo with automated
tests rather than a screen recording.

## Problem

By M7 every component works and is tested in isolation or in pairs. What is not
proven is that the assembled system behaves correctly across process boundaries,
restarts, and outages happening together. Integration bugs live exactly in those
seams, and a demo performed by hand proves nothing repeatable.

There is also a practical problem: a full watering cycle takes an hour in real
time. Without accelerated time, an end-to-end suite is not something anyone runs.

## Goals

1. `docker compose up --build` starts the complete software topology.
2. A one-command end-to-end test suite that exits non-zero on failure.
3. Accelerated virtual time so the whole suite runs in minutes.
4. Every scenario in [failure-scenarios.md](../testing/failure-scenarios.md)
   marked `e2e` implemented and green.
5. The first major demo (project plan §46) reproduced as an automated test.
6. CI runs the suite on every change.

## Non-goals

- Hardware (M9+).
- The UI. It is a Tauri desktop app and deliberately not part of this
  environment ([ADR-009](../adr/009-ui-architecture-and-rust-web-stack.md)),
  which keeps M8 headless and CI-runnable.
- Performance or load testing. The volumes here are trivial; a load suite would
  test Docker, not Rhizo Edge.

## User/system flows

```text
developer / CI
     │
     ├─ docker compose -f deploy/docker-compose.yml up --build
     │     → mosquitto, device-simulator, edge-controller, cloud-api, postgres
     │     → curl localhost:8080/api/v1/overview
     │
     └─ ./scripts/run-scenarios.sh
           → brings the accelerated topology up and waits on its health gates
           → scenario-runner drives and asserts every scenario
           → exit 0 = the system works
```

## Functional requirements

### Environment

| ID | Requirement |
|---|---|
| F-080-01 | One command starts the full topology; health checks gate startup order |
| F-080-02 | Named volumes for SQLite and PostgreSQL; a documented reset procedure |
| F-080-03 | The test overlay sets `time-scale`, shortens tick and TTLs consistently, and **disables restart policies** so a crash fails the test rather than being papered over |
| F-080-04 | Edge and simulator read the same time scale from one compose variable and **assert agreement at startup** |
| F-080-05 | Each scenario runs against a clean database; no cross-scenario state |
| F-080-06 | The suite exits non-zero on any failure and prints which scenario failed |
| F-080-07 | Scenarios can be run individually by name |
| F-080-08 | Total suite runtime under 10 minutes |

### Scenario runner

| ID | Requirement |
|---|---|
| F-080-10 | A Rust binary (or `rhizo-testkit` integration test) — **not** a shell script, so assertions are typed and failures are legible |
| F-080-11 | Drives the system through the edge REST API and the simulator control API |
| F-080-12 | Asserts on **observable state**: API responses, database rows, and MQTT messages captured by a spy subscriber. **Never on log strings.** |
| F-080-13 | Can stop and start containers to simulate outages |
| F-080-14 | Deterministic: a seed produces the same sequence |
| F-080-15 | On failure, dumps the relevant database rows and recent MQTT traffic |

### Coverage

| ID | Requirement |
|---|---|
| F-080-20 | All `e2e` scenarios: SCEN-002, -011, -012, -022, -023, -025, -031, -040, -042, -044, -051, -060, -061, -062 |
| F-080-21 | The project-plan §46 demo implemented as `scenario_first_demo` |
| F-080-22 | Each scenario names the invariants it proves in its test name or metadata |
| F-080-23 | M8-015 covers device isolation with enabled/disabled/invalid policy, stale inputs, monotonic cooldown/budget, and bounded autonomous dosing |
| F-080-24 | M8-016 covers reconnect replay, exact-once `event_id` reconciliation, sealed gaps, lost-ACK replay, and command suppression until reconciliation completes |

## Interfaces

```text
# full environment
docker compose -f deploy/docker-compose.yml up --build

# whole suite
./scripts/run-scenarios.sh

# one scenario
./scripts/run-scenarios.sh --scenario scenario_cloud_outage_recovery

# reset
docker compose -f deploy/docker-compose.yml down -v
```

> **Amended during M8 execution.** The suite was specified as
> `up --abort-on-container-exit --exit-code-from scenario-runner`, and that
> command cannot work: `--abort-on-container-exit` ends the run the moment any
> container exits, and stopping containers is what a third of these scenarios
> do. SCEN-051 kills the edge, SCEN-012 restarts the broker, SCEN-060 stops the
> cloud, and the harness stops the edge and simulator between every pair of
> scenarios to give each a clean database (F-080-05). Measured, not assumed:
> under that flag the runner is SIGKILLed with exit 137 during the second
> scenario, which reads as a scenario failure and is not one. `run-scenarios.sh`
> brings the topology up, waits on its health gates, runs the runner as a
> one-shot container, collects diagnostics on failure, and exits with the
> runner's status — still one command, and still non-zero on any failure.

Scenario-runner interfaces: the edge REST API
([http-api-boundaries.md](../protocol/http-api-boundaries.md) §2), the simulator
control API ([PRD 020](020-device-simulator.md)), the Docker API for
stop/start, and direct SQLite/PostgreSQL reads for assertions.

## Data model

None new. M8 consumes the edge and cloud schemas.

The runner reads both databases directly for assertions — deliberately, because
asserting through the API alone would leave a bug in the API able to hide a bug
in the data.

## State model

Per scenario:

```text
setup (clean DB, fixtures, start containers)
  → actions (API calls, fault injection, container stop/start, time advance)
  → assertions (API + DB + captured MQTT)
  → teardown (stop, collect diagnostics on failure)
```

Scenarios do not share state. Isolation costs a few seconds of container restart
and removes an entire class of flaky, order-dependent failure.

## Failure modes

| Failure | Behaviour |
|---|---|
| A container fails to become healthy | suite aborts with which container and its logs |
| A scenario times out | fails with the last known state dumped, not a bare timeout |
| Docker unavailable | clear message; the suite is skipped in environments without Docker rather than reported as passing |
| Flaky ordering | scenarios are isolated and seeded; a genuinely flaky scenario is quarantined and fixed, **never retried to green** |
| Time-scale mismatch | F-080-04 asserts agreement at startup and fails loudly |

The "never retried to green" rule matters: an automatically retried flaky
safety test is a safety test that does not work.

## Safety implications

M8 enforces no new invariant but **re-verifies ten of them in the assembled
system**, where cross-process interactions can break what unit tests proved:

SAFETY-001, -002, -003, -004, -005, -006, -007, -008, -009, -010.

SAFETY-011 requires a device restart mid-dose and is verified here against the
simulator (SCEN-026 moves to M9 for firmware, M11 for hardware).

The two scenarios that justify the milestone on their own:

- **SCEN-061** (`safety_009_decisions_identical_with_cloud_down`) — the same
  seeded run with the cloud up and down produces identical command sequences.
  This is the strongest single statement the project can make about edge-first
  correctness.
- **SCEN-051** (restart mid-command) — proves SAFETY-010 across real process
  boundaries rather than in a simulated restart.

## Observability

The suite asserts that the observability itself works:

- `/metrics` exposes the expected series after a scenario.
- `/health/ready` returns 200 with the cloud stopped and 503 with the broker
  stopped.
- Device events are recorded for leak, offline, boot, and sensor faults.

On failure, the runner collects: the last 200 log lines per container, a dump of
`measurements`, `commands`, `watering_events`, `irrigation_state`, and
`pending_cloud_events`, plus captured MQTT traffic — so a CI failure is
diagnosable without reproducing it locally.

## Testing strategy

M8 *is* testing infrastructure, so its own correctness is verified by
**negative tests**: a deliberately broken build must make the suite fail.

Specifically, the following mutations must each turn the suite red, and are run
once during M8 acceptance:

| Mutation | Expected failing scenario |
|---|---|
| remove the leak check from the gate | SCEN-040 |
| use `device_time_ms` for staleness | SCEN-022 / SCEN-070 |
| make the outbox drain blocking | SCEN-060 |
| re-publish commands on restart | SCEN-051 |
| use a calendar day for the cap | SCEN-034 |
| let the simulator skip `validate_water_command` | SCEN-032 |

A test suite that stays green when the safety logic is removed is decoration.
This table is how M8 proves it is not.

## Acceptance criteria

- [ ] **The entire suite runs with no hardware: no ESP32, no pump, no plant.**
- [ ] `docker compose up --build` brings up all five services with no manual steps.
- [ ] The full suite runs with one command and exits 0.
- [ ] The suite exits non-zero when any scenario fails.
- [ ] Total runtime under 10 minutes.
- [ ] Every `e2e` scenario in [failure-scenarios.md](../testing/failure-scenarios.md)
      is implemented and green.
- [ ] `scenario_first_demo` reproduces all 18 steps of the project-plan demo.
- [ ] Each of the seven mutations turns the suite red — the six above, plus
      M8-017's: publishing a command immediately to a sleeping device instead of
      holding it as an intent.
- [ ] CI runs the suite on every change to `crates/**` or `deploy/**`.
- [ ] A failing scenario prints database state and MQTT traffic, not just an
      assertion message.
- [ ] Device-isolation and reconciliation scenarios from M8-015/M8-016 pass and
      re-verify the applicable SAFETY-013…020 invariants.
- [ ] SCEN-113…SCEN-117 run the battery and sleep seam against a simulator in
      battery mode, with a spy subscriber confirming **nothing** is published on
      any `commands/*` topic while a device sleeps, and re-verify SAFETY-021
      end to end (M8-017).

## Dependencies

- M7 (cloud, the last component of the topology).
- M2 (simulator control API and fault injection).
- M6 (irrigation, the behaviour most scenarios exercise).

## Open questions

1. **`testcontainers` vs compose-managed containers** for the runner. Compose is
   chosen for M8 because the same file defines the developer environment and the
   test environment, which keeps them from drifting. Revisit if container
   lifecycle control proves awkward for stop/start scenarios (M8-005).
2. **Whether the scenario DSL from the implementation prompt §17 is worth
   building.** Deliberately deferred: with ~14 scenarios, typed Rust functions
   using testkit helpers are clearer and better-checked than a YAML interpreter.
   Revisit at ~30 scenarios (likely M13).

## Future work

- Hardware-in-the-loop as an optional overlay (M11).
- Multi-device scenarios (M13).
- Nightly long-running soak scenario at real time (post-V1).
