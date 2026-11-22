# Issue M12-019 — M12 verification and exit criteria

> **Execution scope, 2026-11-22:** **Status: PLANNED — prototype slice.** Apply the current milestone PRD prototype scope and [testing policy](../../testing/strategy.md). Only relevant tests and a basic smoke are required; extended campaigns below are backlog reference, not mandatory local reruns. See [ROADMAP](../../../ROADMAP.md).


**Milestone:** M12 · **PRD:** [PRD 120](../../prd/120-rust-ui.md) · **Depends on:** M12-001, M12-002, M12-003, M12-004, M12-006, M12-008, M12-010, M12-013, M12-014

## Context

Prototype exit for M12, governed by the current PRD scope and ROADMAP. Deferred hardening is not a passing result or an entry gate for other features.

## Goal

Demonstrate the M12 prototype slice.

## Scope

Relevant tests, a local functional smoke, a short result in this issue and accurate roadmap status. Reuse prior evidence; no full campaign report required.

## Non-goals

Deferred production hardening and full historical verification campaigns.

## Dependencies

- M12-001
- M12-002
- M12-003
- M12-004
- M12-006
- M12-008
- M12-010
- M12-013
- M12-014

## Implementation notes

Use actual observed behavior and report limitations. The first plant experiment is supervised with small quantities; it does not require a month-long campaign.

## Acceptance criteria

- [ ] Minimal interface runs on the development host using Edge REST only.
- [ ] Measurements, age, faults, bindings, thresholds and watering limits are usable for one plant.
- [ ] Manual water/stop, refusal display and disconnected state work.
- [ ] Automation shows limits; no bypass control exists.
- [ ] One live local smoke passes; short result recorded; deferred UI remains open.

## Verification

Run the relevant tests for changed behavior and the prototype smoke described above using the current firmware/local development commands. Record actual commands and results; do not run an unrelated full suite.

## Tests required

Relevant unit/regression tests and the local prototype smoke; essential hardware checks where actuation is involved.

## Documentation impact

Current issue result and ROADMAP status only; retain historical reports and deferred criteria.

## Files likely affected

```text
ROADMAP.md
```
