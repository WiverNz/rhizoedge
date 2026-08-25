# Issue M13-001 — Verify and harden multi-device operation

**Milestone:** M13 · **PRD:** [PRD 130](../../prd/130-multi-plant-home.md) · **Depends on:** M12-013

## Context

The schema already supports several devices and plants. M13 verifies the
behaviour and hardens the paths that only matter at scale — especially
cross-plant isolation.

## Goal

Operate reliably with 10 devices and 20 plants.

## Scope

- Control loop evaluates all plants within one tick
- Plants evaluated in a stable order so none starves
- One device's failure affects only its plants
- Simultaneous in-flight commands to different devices
- `control_tick_duration_seconds` monitored

## Non-goals

- Parallelising the control loop — not needed at 20 plants.

## Dependencies

- M12-013

## Implementation notes

**SCEN-080 is the important test**: force every failure mode on plant A and
assert plant B's state is byte-identical to a control run. Cross-plant
interference would be a new class of bug that single-plant testing cannot reveal.

Stable ordering matters when the tick budget is tight: a random order would
starve different plants on different ticks.

## Acceptance criteria

- [ ] 10 devices and 20 plants operate independently.
- [ ] **SCEN-080 passes: byte-identical state for unaffected plants.**
- [ ] The tick completes within its period at 20 plants.
- [ ] Simultaneous commands to different devices work.
- [ ] Plants are evaluated in a stable order.
- [ ] Tick duration is monitored.

## Verification

```bash
docker compose up --scale device-simulator=10
cargo test --test integration multi_device
```

## Tests required

- **SCEN-080 cross-plant isolation.**
- Tick budget at 20 plants.
- Simultaneous commands.

## Documentation impact

- None.

## Files likely affected

```text
crates/edge-controller/src/control/tick.rs
```
