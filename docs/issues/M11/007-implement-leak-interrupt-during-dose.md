# Issue M11-007 — Implement leak interruption of an active dose

**Milestone:** M11 · **PRD:** [PRD 110](../../prd/110-real-pump-and-safety-hardware.md) · **Depends on:** M11-006, M11-002

## Context

PRD 110 F-110-33: a leak asserted **during** a dose must stop the pump within
one second. Waiting for the dose to complete would deliver water into a known
leak.

## Goal

Stop an in-progress dose on a leak.

## Scope

- The leak sensor polled during actuation
- Assertion stops the pump within 1 s
- Partial delivery estimated from elapsed time and reported
- `command.result` with `status: "failed"` and `reason: "leak_detected"`
- The plant locked out

## Non-goals

- Resuming the dose afterwards.

## Dependencies

- M11-006
- M11-002

## Implementation notes

Polling during actuation means the pump loop cannot be a blocking sleep. Use
the same task structure as the run guard, checking both the deadline and the leak
state.

Report the partial delivery estimate rather than null: unlike an interrupted
reboot, the device knows how long it ran, so a time-based estimate is real
information.

## Acceptance criteria

- [ ] A leak during a dose stops the pump within 1 second.
- [ ] Partial delivery is estimated and reported.
- [ ] The result is `failed` with `reason: "leak_detected"`.
- [ ] The plant is locked out.
- [ ] The partial volume counts toward the daily total.
- [ ] The dose is not resumed.

## Verification

```bash
cd firmware/esp32-node && cargo test pump::leak_interrupt
```

## Tests required

- Interruption timing.
- Partial delivery reporting.
- No resumption.

## Documentation impact

- None.

## Files likely affected

```text
firmware/esp32-node/src/pump/mod.rs
```
