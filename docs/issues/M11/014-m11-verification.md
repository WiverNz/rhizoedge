# Issue M11-014 — M11 verification and exit criteria

> **Execution scope, 2026-11-22:** **Status: PLANNED — prototype slice.** Apply the current milestone PRD prototype scope and [testing policy](../../testing/strategy.md). Only relevant tests and a basic smoke are required; extended campaigns below are backlog reference, not mandatory local reruns. See [ROADMAP](../../../ROADMAP.md).


**Milestone:** M11 · **PRD:** [PRD 110](../../prd/110-real-pump-and-safety-hardware.md) · **Depends on:** M11-001, M11-002, M11-003, M11-004, M11-005, M11-006, M11-007, M11-008, M11-009, M11-010, M11-011, M11-012, M11-013, M12-019

## Context

Prototype exit for M11, governed by the current PRD scope and ROADMAP. Deferred hardening is not a passing result or an entry gate for other features.

## Goal

Demonstrate the M11 prototype slice.

## Scope

Relevant tests, a local functional smoke, a short result in this issue and accurate roadmap status. Reuse prior evidence; no full campaign report required.

## Non-goals

Deferred production hardening and full historical verification campaigns.

## Dependencies

- M11-001
- M11-002
- M11-003
- M11-004
- M11-005
- M11-006
- M11-007
- M11-008
- M11-009
- M11-010
- M11-011
- M11-012
- M11-013
- M12-019

## Implementation notes

Use actual observed behavior and report limitations. The first plant experiment is supervised with small quantities; it does not require a month-long campaign.

## Acceptance criteria

- [ ] Essential driver startup/reset/watchdog/control-loss and electrical checks pass for the actual assembly.
- [ ] Small cup doses show conservative volume calibration, runtime/cumulative caps and cooldown.
- [ ] Missing/invalid inputs and communication loss cannot cause uncontrolled watering; required leak/tank handling works.
- [ ] Stop/power disconnect and conservative restart/no replay are demonstrated.
- [ ] One supervised real-plant cycle works through the complete system and minimal interface.
- [ ] Short result records exact configuration, limits and deferred coverage; no blanket production/invariant validation claim.

## Verification

Run the relevant tests for changed behavior and the prototype smoke described above using the current firmware/local development commands. Record actual commands and results; do not run an unrelated full suite.

## Tests required

Relevant unit/regression tests and the local prototype smoke; essential hardware checks where actuation is involved.

## Documentation impact

Current issue result and ROADMAP status only; retain historical reports and deferred criteria.

## Files likely affected

```text
ROADMAP.md
docs/architecture/safety-invariants.md
docs/testing/hil-runs/
```
