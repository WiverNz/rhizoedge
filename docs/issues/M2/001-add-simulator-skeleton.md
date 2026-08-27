# Issue M2-001 — Create the device-simulator binary skeleton

**Milestone:** M2 · **PRD:** [PRD 020](../../prd/020-device-simulator.md) · **Depends on:** M1-019, M0-006

## Context

PRD 020 makes the simulator the reference device for M3-M8. It is a real
component with a real CLI, not a test helper.

## Goal

Create the binary with its CLI, configuration, and logging.

## Scope

- Binary with `clap` for the full CLI in simulator-strategy.md section 7
- Tokio runtime and task structure
- `rhizo-telemetry` logging with `device_id` on every event
- Graceful shutdown on SIGTERM/Ctrl-C

## Non-goals

- MQTT (M2-002).
- The physical model (M2-004).

## Dependencies

- M1-019
- M0-006

## Implementation notes

Every CLI flag from simulator-strategy.md section 7 is defined now, even
where the behaviour lands later — it keeps the interface stable while the
implementation fills in, and prevents flag churn across five issues.

Log `device_id` as a structured field on every event; with several simulators
running, unlabelled logs are unusable.

## Acceptance criteria

- [x] `cargo run -p device-simulator -- --help` lists every documented flag.
- [x] The binary starts and shuts down cleanly on SIGTERM.
- [x] Logs carry `device_id` as a structured field.
- [x] Invalid arguments exit non-zero with a useful message.

## Verification

```bash
cargo run -p device-simulator -- --help
cargo run -p device-simulator -- --device-id plant-node-01 &  # then SIGTERM
```

## Tests required

- CLI parsing including invalid combinations.

## Documentation impact

- None; simulator-strategy.md section 7 already documents the CLI.

## Files likely affected

```text
crates/device-simulator/src/main.rs
crates/device-simulator/src/cli.rs
```
