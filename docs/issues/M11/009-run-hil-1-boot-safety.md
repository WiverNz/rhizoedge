# Issue M11-009 — Run HIL-1 boot safety verification

> **Execution scope, 2026-11-22:** **Status: PLANNED — prototype slice.** Apply the current milestone PRD prototype scope and [testing policy](../../testing/strategy.md). Only relevant tests and a basic smoke are required; extended campaigns below are backlog reference, not mandatory local reruns. See [ROADMAP](../../../ROADMAP.md).


**Milestone:** M11 · **PRD:** [PRD 110](../../prd/110-real-pump-and-safety-hardware.md) · **Depends on:** M11-002, M11-008

## Context

Targeted prototype hardware check under PRD 110 and the testing policy. The full HIL catalogue remains deferred reference, not a mandatory repetition matrix.

## Goal

Establish safe control behavior before pump operation.

## Scope

Perform and record the acceptance checks below on the actual setup. Reuse valid previous evidence when wiring and relevant firmware have not changed.

## Non-goals

Extended repetition campaigns, exhaustive fault injection and production validation.

## Dependencies

- M11-002
- M11-008

## Implementation notes

Keep the pump disabled until essential electrical/control checks pass. Start with a cup and a reachable power disconnect. Record actual observations briefly; no new full campaign report is needed.

## Acceptance criteria

- [ ] Actual wiring, supply/polarity, control timing and safe off-state are verified with motor disabled.
- [ ] Boot, reset, watchdog reset and lost control/MCU power leave the actual driver safely off; observe pulses with suitable capture, not a DMM alone.
- [ ] A reachable pump-power disconnect is identified and usable.
- [ ] Short record states observed coverage and remaining limits; no exhaustive transient-proof claim.
- [ ] Unexpected actuation blocks pump operation until fixed; sensor/UI work can continue.

## Verification

Perform the specific physical checks above with human supervision; record actual setup, observations and remaining limits in this issue. Full HIL repetitions are deferred.

## Tests required

The essential prototype checks above only, plus a relevant regression if a defect is fixed.

## Documentation impact

Short issue result; retain earlier evidence and do not mark extended HIL passed.

## Files likely affected

```text
docs/testing/hil-runs/
```
