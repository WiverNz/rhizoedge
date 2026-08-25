# Issue M13-004 — Implement shared reservoir lockout

**Milestone:** M13 · **PRD:** [PRD 130](../../prd/130-multi-plant-home.md) · **Depends on:** M13-003

## Context

PRD 130 F-130-22/23: a low reservoir locks out **every** plant drawing from
it, and disagreeing sensors resolve to the **lowest** value — with two
disagreeing sensors, the safe belief is the one that prevents pumping.

## Goal

Extend SAFETY-004 to shared reservoirs.

## Scope

- Reservoir level as the minimum of its devices' readings
- **Unknown from any sensor makes the reservoir unknown** -> lockout
- A low reservoir locks out all its plants
- Refill clears all of them
- The reservoir level exposed as a metric

## Non-goals

- Predicting depletion.

## Dependencies

- M13-003

## Implementation notes

Taking the minimum and treating any unknown as unknown are both the
conservative direction, consistent with SAFETY-012. An averaging rule would let
one optimistic sensor mask an empty tank.

Note that per-device daily caps still apply independently, so the device-level
bound remains regardless of reservoir modelling.

## Acceptance criteria

- [ ] The reservoir level is the minimum of its sensors.
- [ ] **An unknown reading from any sensor makes the reservoir unknown.**
- [ ] A low reservoir locks out every plant on it.
- [ ] A refill clears them all.
- [ ] Devices without a reservoir are unaffected.
- [ ] The level is exported as a metric.

## Verification

```bash
cargo test -p rhizo-domain reservoir::
cargo test safety_004
```

## Tests required

- **Minimum-wins resolution.**
- **Unknown propagation.**
- Multi-plant lockout and clear.

## Documentation impact

- safety-invariants.md SAFETY-004 extended for shared reservoirs.

## Files likely affected

```text
crates/domain/src/reservoir.rs
crates/edge-controller/src/control/tick.rs
```
