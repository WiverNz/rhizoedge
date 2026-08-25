# Issue M3-001 — Create the edge-controller binary and task supervisor

**Milestone:** M3 · **PRD:** [PRD 030](../../prd/030-edge-ingestion-and-storage.md) · **Depends on:** M0-013, M1-014

## Context

The edge controller is the control plane. Its task structure is established
now, including the supervisor that exits the process on a task panic — a process
that is up but not evaluating safety is worse than one that is down.

## Goal

Create the binary, its Tokio task structure, and the supervising runtime.

## Scope

- Binary with config loading (M0-005) and telemetry init (M0-006)
- A task supervisor spawning named long-running tasks
- On a task panic: log with context, increment `task_panics_total`, **exit non-zero**
- Graceful shutdown on SIGTERM: stop intake, finish in-flight work, exit 0
- Placeholder tasks for mqtt_ingress, pipeline, api, retention

## Non-goals

- MQTT (M3-005).
- Storage (M3-002).
- The API (M4).

## Dependencies

- M0-013
- M1-014

## Implementation notes

The supervisor is the important part. Watching a `JoinHandle` and logging
the error is not enough — the process must exit, or supervision and alerting
will report 'healthy' while nothing watches the plant (failure-model 3.6).

Distinguish shutdown from panic: SIGTERM exits 0, a panic exits non-zero, so a
supervisor's restart policy behaves correctly.

## Acceptance criteria

- [ ] The binary starts, loads config, and initialises logging.
- [ ] SIGTERM shuts down cleanly with exit 0.
- [ ] A panic in any supervised task logs, increments the counter, and exits non-zero.
- [ ] Task names appear in logs and in the metric label.
- [ ] Shutdown waits for in-flight work up to a timeout.

## Verification

```bash
cargo run -p edge-controller &
kill -TERM $!  # exit 0
cargo test -p edge-controller supervisor::
```

## Tests required

- Supervisor exits non-zero on a panic.
- Graceful shutdown path.
- Shutdown timeout when a task hangs.

## Documentation impact

- None.

## Files likely affected

```text
crates/edge-controller/src/main.rs
crates/edge-controller/src/supervisor.rs
```
