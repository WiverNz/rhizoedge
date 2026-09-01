# Issue M16-015 — Run the verified-watering hardware-in-the-loop suite

**Milestone:** M16 · **PRD:** [PRD 160](../../prd/160-verified-watering.md) · **Depends on:** M16-014

## Context

This feature crosses the software/physical boundary in a way nothing before it
has: every other safety property in the project can be proven in software and
re-verified with hardware, but "the tube is blocked" has no software definition.
It only exists on a bench, with a real pump, a real tube, and a person who
pinched it.

The existing stage gate runs HIL-1 through HIL-7. This adds **HIL-8**, after
HIL-6's dry cycle and before HIL-7's supervised plant — because a node that
cannot tell a blocked tube from a delivered dose has no business being pointed at
a plant.

## Goal

HIL-8, run, measured, and recorded, with the physical failure cases as required
gates.

## Scope

- HIL-8 added to `docs/testing/hardware-in-the-loop.md` with its prerequisites,
  procedure, and pass criteria.
- **Required gates:** verified delivery against a measuring cup; blocked tube;
  disconnected tube; empty reservoir; restricted (partial) flow; network
  disconnect mid-dose; ESP32 restart mid-dose; edge restart mid-dose; leak
  asserted mid-dose.
- **Best-effort, recorded either way:** a stuck pump or valve, if it can be
  simulated safely on the bench.
- Bench measurements that settle the starting-value constants:
  `FLOW_START_TIMEOUT_MS`, `FLOW_SETTLE_MS`, `PARTIAL_DELIVERY_FRACTION`,
  `OVER_DELIVERY_FACTOR`, and the witness's real resolution and noise floor.
- Results recorded in `docs/testing/hil-runs/`.

## Non-goals

- New hardware beyond the reservoir load cell.
- Automating HIL. It is a supervised bench procedure by design.
- Deciding the shared-reservoir case. PRD 160 §Open questions 2 is M13's.

## Dependencies

- M16-014

## Implementation notes

A physical pump power cut-off must be present throughout, as F-110-42 already
requires of every HIL stage. The unexpected-flow cases are the first in this
project deliberately designed to make water move when it should not, and they
are run with the outlet in a measuring cup, never over anything that matters.

**Measure, do not confirm.** The constants in M16-001 are starting values, and
this is where they stop being guesses. `FLOW_START_TIMEOUT_MS` in particular is a
real trade-off with a real number behind it: long enough for a peristaltic head
to prime and a compliant tube to pressurise, short enough that a blocked line
does not run for seconds. Record the priming time actually observed, over several
runs, from both a primed and an unprimed start — an unprimed head is the slow
case and the one that sets the bound.

Record the disturbance behaviour PRD 160 §Open questions 1 asks about: what a
refill, a hand on the shelf, and a knock do to the reservoir scale, in grams and
in duration. That measurement decides whether a settling filter is needed and it
cannot be answered any other way.

The hardware guide's numbers are starting points its own text lists as needing
measurement. Nothing here derives a constant from it; this issue is where the
guide's TBDs for this subsystem get filled in from the bench.

## Acceptance criteria

- [ ] HIL-8 is documented with prerequisites, procedure, and pass criteria.
- [ ] Every required gate is run and recorded, with measured volumes.
- [ ] A blocked tube produces `no_flow` on the first dose and stops the pump.
- [ ] A disconnected tube is distinguishable from a blocked one in the record, or
      the fact that it is not is recorded explicitly.
- [ ] An empty reservoir refuses before actuation, as today.
- [ ] Restricted flow produces `partial_delivery` within the measured tolerance.
- [ ] Disconnect, ESP32 restart, and edge restart mid-dose each produce their
      documented outcome and never a zero delivery.
- [ ] A leak asserted mid-dose still stops the pump within one second.
- [ ] Every starting-value constant is either measured and updated, or confirmed
      with its evidence.
- [ ] Scale disturbance behaviour is measured and recorded.
- [ ] Results are in `docs/testing/hil-runs/`.

## Verification

```bash
# Supervised bench procedure; see docs/testing/hardware-in-the-loop.md HIL-8.
cargo test safety_023
cargo test safety_024
```

## Tests required

- The HIL stage itself. Its evidence is the recorded run, not a test binary.
- Every constant changed here re-verified in the unit and simulator suites.

## Documentation impact

- `docs/testing/hardware-in-the-loop.md`: HIL-8 and its place in the stage gate
  order.
- `docs/hardware/home-node-hardware-guide.md`: measured values replacing this
  subsystem's TBDs.
- PRD 160 §Open questions 1 and 3, answered with measurements.

## Files likely affected

```text
docs/testing/hardware-in-the-loop.md
docs/testing/hil-runs/
docs/hardware/home-node-hardware-guide.md
crates/domain/src/delivery/types.rs
```
