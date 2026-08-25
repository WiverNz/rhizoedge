# Issue M8-012 — Implement the first demo as an automated scenario

**Milestone:** M8 · **PRD:** [PRD 080](../../prd/080-end-to-end-test-environment.md) · **Depends on:** M8-006, M8-011

## Context

The project plan's section 46 demo, backed by automated tests rather than a
screen recording. It is the milestone's headline deliverable.

## Goal

Reproduce all eighteen demo steps as an asserted scenario.

## Scope

- `scenario_first_demo` covering all eighteen steps
- Each step asserted on observable state
- Human-readable progress output so it doubles as a demonstration

## Non-goals

- A GUI or video.

## Dependencies

- M8-006
- M8-011

## Implementation notes

The eighteen steps: simulator connects, device online, telemetry appears,
moisture decreases, dry soil detected, recommendation generated, automatic dose
issued, simulator applies water, absorption wait, recheck still dry, second dose,
moisture recovers, healthy state, cloud stopped, edge continues, events queue,
cloud restarts, events synchronise.

The progress output matters: this scenario is what gets shown to someone asking
what the project does, so it should read as a narrative rather than as test
output.

## Acceptance criteria

- [ ] All eighteen steps execute and are asserted.
- [ ] The scenario passes reliably (10 consecutive runs).
- [ ] Progress output is human-readable.
- [ ] It completes within the accelerated budget.
- [ ] It exercises the multi-dose path, not just a single dose.

## Verification

```bash
... run --rm scenario-runner --scenario scenario_first_demo
```

## Tests required

- The scenario, run ten times for stability.

## Documentation impact

- README.md demo section references it.

## Files likely affected

```text
test/scenarios/first_demo.rs
```
