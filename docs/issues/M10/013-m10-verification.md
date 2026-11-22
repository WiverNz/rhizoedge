# Issue M10-013 — M10 verification and exit criteria

> **Execution scope, 2026-11-22:** **Status: PLANNED — prototype slice.** Apply the current milestone PRD prototype scope and [testing policy](../../testing/strategy.md). Only relevant tests and a basic smoke are required; extended campaigns below are backlog reference, not mandatory local reruns. See [ROADMAP](../../../ROADMAP.md).


**Milestone:** M10 · **PRD:** [PRD 100](../../prd/100-real-soil-sensor.md) · **Depends on:** M10-001, M10-002, M10-003, M10-004, M10-006, M10-007, M10-008, M10-009

## Context

Prototype exit for M10, governed by the current PRD scope and ROADMAP. Deferred hardening is not a passing result or an entry gate for other features.

## Goal

Demonstrate the M10 prototype slice.

## Scope

Relevant tests, a local functional smoke, a short result in this issue and accurate roadmap status. Reuse prior evidence; no full campaign report required.

## Non-goals

Deferred production hardening and full historical verification campaigns.

## Dependencies

- M10-001
- M10-002
- M10-003
- M10-004
- M10-006
- M10-007
- M10-008
- M10-009

## Implementation notes

Use actual observed behavior and report limitations. The first plant experiment is supervised with small quantities; it does not require a month-long campaign.

## Acceptance criteria

- [ ] SEN0601 → ESP32-C3 → MQTT → Edge → SQLite/API works with real readings.
- [ ] Configured units/scaling/calibration provenance and basic settling are documented.
- [ ] Readings are plausible/repeatable in intended soil; missing/invalid/stale soil values block automatic watering.
- [ ] Relevant host tests and one local sensor smoke pass.
- [ ] Short outcome recorded; roadmap updated only to demonstrated status; deferred work remains open.

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
