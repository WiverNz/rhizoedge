# Issue M11-013 — Run HIL-6 full dry cycle verification

**Milestone:** M11 · **PRD:** [PRD 110](../../prd/110-real-pump-and-safety-hardware.md) · **Depends on:** M11-012

## Context

The complete automatic cycle against real hardware — first with the outlet in
a separate measuring cup so control logic is decoupled from where the water
goes, then into the soil.

## Goal

Verify the full multi-dose cycle physically.

## Scope

- Dry a soil sample below `target_min`
- Confirm the state sequence matches SCEN-002
- Verify each dose's volume in the cup against calibration
- Confirm the absorption wait is honoured in real time
- Confirm the cycle stops at `max_doses_per_cycle` with `MaxDosesReached`
- Confirm the rolling daily cap holds across cycles
- Repeat with the outlet in the soil

## Non-goals

- A plant (HIL-7).

## Dependencies

- M11-012

## Implementation notes

The two-phase approach separates two questions that a single test would
conflate: does the control logic sequence correctly, and does the water actually
reach the soil and register. Running the first into a cup makes a control bug
diagnosable without soil physics in the way.

## Acceptance criteria

- [ ] The state sequence matches SCEN-002 exactly.
- [ ] Each dose's measured volume matches the request within calibration tolerance.
- [ ] The absorption wait is honoured in real time.
- [ ] The cycle stops at the dose limit with `MaxDosesReached`.
- [ ] The rolling daily cap holds across cycles.
- [ ] With the outlet in soil, moisture rises and the cycle terminates on recovery.
- [ ] The overshoot-then-settle behaviour resembles the simulator's model.

## Verification

```bash
# manual: HIL-6 checklist, both phases
```

## Tests required

- The HIL-6 checklist.

## Documentation impact

- hil-runs record; simulator model refined if reality diverges materially.

## Files likely affected

```text
docs/testing/hil-runs/
```
