# Issue M11-001 — Implement the real pump driver adapter

> **Execution scope, 2026-11-22:** **Status: PLANNED — prototype slice.** Apply the current milestone PRD prototype scope and [testing policy](../../testing/strategy.md). Only relevant tests and a basic smoke are required; extended campaigns below are backlog reference, not mandatory local reruns. See [ROADMAP](../../../ROADMAP.md).


**Milestone:** M11 · **PRD:** [PRD 110](../../prd/110-real-pump-and-safety-hardware.md) · **Depends on:** M9-005, M9-011, M9-012

## Context

Use DFR0523's integrated driver. A generic MOSFET gate-low assumption does
not establish its off-state. Verify waveform, signal compatibility and loss-of-
control behavior before motor power; see PRD 110 prototype scope.

## Goal

Drive a real pump safely.

## Scope

- Real adapter behind the existing Pump interface, using verified DFR0523 control timing.
- Separate verified pump supply and safe power wiring.
- Duration from conservative calibration, clamped to firmware limits.
- Actual runtime reported; no raw/bypass actuation path.

## Non-goals

- The run guard (M11-002).

## Dependencies

- M9-005
- M9-011
- M9-012

## Implementation notes

Inspect the current board profile and actual hardware before selecting pins.
Verify the real driver's startup/reset and absent-signal behavior with motor
power disabled. Document the confirmed waveform/off-state; do not invent pulse
timing from conflicting specifications or add a generic MOSFET by default.

## Acceptance criteria

- [ ] Adapter uses the confirmed DFR0523 control interface and existing shared command gate.
- [ ] Duration is computed, measured and clamped to firmware maximum.
- [ ] Confirmed control/off-state and separate supply requirements are documented.
- [ ] Relevant host tests cover timing/clamping with a fake output.
- [ ] Motor remains disabled until essential physical safety checks pass.

## Verification

```bash
cd firmware/esp32-node && cargo test pump::real
```

## Tests required

- Duration computation and clamping.
- Measured duration reporting.

## Documentation impact

- Board-profile wiring requirements.

## Files likely affected

```text
firmware/esp32-node/src/pump/real.rs
firmware/esp32-node/src/board/devkitm1.rs
```
