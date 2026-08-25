# Issue M11-009 — Run HIL-1 boot safety verification

**Milestone:** M11 · **PRD:** [PRD 110](../../prd/110-real-pump-and-safety-hardware.md) · **Depends on:** M11-002, M11-008

## Context

**The gate that everything else waits behind.** SAFETY-011 verified
electrically, with a multimeter, before any water is in the system.

## Goal

Prove the pump line never asserts except on a validated command.

## Scope

- The full HIL-1 checklist from hardware-in-the-loop.md
- Multimeter on the pump driver input throughout
- Tubing disconnected, reservoir empty
- 20 resets, flashing while powered, a watchdog reset, 10 mid-boot power cuts
- Results recorded in `docs/testing/hil-runs/`

## Non-goals

- Anything involving water.

## Dependencies

- M11-002
- M11-008

## Implementation notes

If the pump line asserts even momentarily, the gate pull-down is wrong. Fix
the hardware before proceeding — no firmware change compensates for a pin that
floats high during reset.

An in-line pump power switch must be present even though the reservoir is empty:
the habit matters more than this particular run.

## Acceptance criteria

- [ ] The pump line never asserts during boot, across 20 resets.
- [ ] It does not pulse during flashing.
- [ ] A watchdog reset leaves it inactive.
- [ ] 10 mid-boot power cuts produce no actuation.
- [ ] Results are recorded with the multimeter readings.
- [ ] **Any twitch stops the milestone until the hardware is fixed.**

## Verification

```bash
# manual checklist with a multimeter; recorded in docs/testing/hil-runs/
```

## Tests required

- The HIL-1 checklist.

## Documentation impact

- hil-runs record.

## Files likely affected

```text
docs/testing/hil-runs/
```
