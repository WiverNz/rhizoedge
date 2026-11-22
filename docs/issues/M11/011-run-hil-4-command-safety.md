# Issue M11-011 — Run HIL-4 command safety verification

> **Execution scope, 2026-11-22:** **Status: PLANNED — prototype slice.** Apply the current milestone PRD prototype scope and [testing policy](../../testing/strategy.md). Only relevant tests and a basic smoke are required; extended campaigns below are backlog reference, not mandatory local reruns. See [ROADMAP](../../../ROADMAP.md).


**Milestone:** M11 · **PRD:** [PRD 110](../../prd/110-real-pump-and-safety-hardware.md) · **Depends on:** M11-010

## Context

**Tests the device's independent veto** by publishing directly to the MQTT
command topic, bypassing the edge entirely. This is where SAFETY-007 stops being
a claim about the simulator.

## Goal

Demonstrate bounded command execution, communication-loss handling and restart on the real prototype.

## Scope

- Small cup dose through the real command path, with time and estimated volume bounded.
- Stop and MQTT loss during a dose; local run guard still enforces the limit.
- Duplicate command/restart does not water twice or replenish budget/cooldown.
- Relevant existing host tests cover invalid/expired/oversized commands without repeatedly dispensing water.

## Non-goals

- Any plant.

## Dependencies

- M11-010

## Implementation notes

Reuse demonstrated host command-validation coverage. Check the actual physical
boundary and local guard now; run an additional fault case only when a concrete
defect or changed behavior warrants it. Keep cup quantities small and a power
disconnect reachable. Protocol/simulator differences need investigation without
retroactively invalidating unrelated historical evidence.

## Acceptance criteria

- [ ] Small real dose is bounded by configured duration and conservative volume.
- [ ] Manual stop works; communication loss cannot defeat the local guard or start new doses.
- [ ] Duplicate delivery and ordinary restart cause no second dose; interrupted volume is conservatively accounted.
- [ ] Cumulative cap and frequency/cooldown remain effective across restart.
- [ ] Relevant command-validation host tests pass; exact physical observations are recorded briefly.

## Verification

Use the existing command/API path with a cup and small limits. Perform the stated stop/loss/restart checks; run only the relevant host validation tests. Record actual commands and results.

## Tests required

Targeted command/guard tests and the bounded physical smoke above.

## Documentation impact

Short issue result and limitations; extended HIL remains deferred.

## Files likely affected

```text
docs/testing/hil-runs/
```
