# Issue M11-002 — Implement the independent run-duration guard

**Milestone:** M11 · **PRD:** [PRD 110](../../prd/110-real-pump-and-safety-hardware.md) · **Depends on:** M11-001

## Context

PRD 110 F-110-05: the guard must run on a **separate task or timer from
MQTT**. A hung network task with the pump energised is the worst reachable state
in this system.

## Goal

Guarantee the pump stops even if the main task hangs.

## Scope

- A run-duration timer independent of the MQTT task
- De-energise at `FIRMWARE_MAX_RUN_SECONDS` unconditionally
- Hardware watchdog enabled
- A watchdog reset leaves the pump off
- Overrun marks the pump faulted

## Non-goals

- Diagnosing why the overrun happened.

## Dependencies

- M11-001

## Implementation notes

Independence is the requirement, and it is easy to lose: a guard implemented
as a check inside the same loop that publishes MQTT is not independent at all.
Use a separate task, and test it by deliberately blocking the main task.

The `pump-stuck-on` simulator fault (M2-013) exists to exercise this path before
hardware.

## Acceptance criteria

- [ ] The guard de-energises at the hard limit.
- [ ] **It works while the main task is deliberately blocked.**
- [ ] The watchdog is enabled and a reset leaves the pump off.
- [ ] An overrun marks the pump faulted and refuses further commands.
- [ ] The guard is in a separate task from MQTT.

## Verification

```bash
cd firmware/esp32-node && cargo test pump::guard
cargo test safety_007
```

## Tests required

- **Guard fires while the main task is blocked.**
- Watchdog path.
- Fault state after overrun.

## Documentation impact

- None.

## Files likely affected

```text
firmware/esp32-node/src/pump/guard.rs
```
