# Issue M2-015 — Declare device capabilities in status

**Milestone:** M2 · **PRD:** [PRD 020](../../prd/020-device-simulator.md) · **Depends on:** M2-003

## Context

[ADR-016](../../adr/016-plant-binding-and-policy-model.md) forbids the edge from
assuming what a device can do. `device == pump controller` is exactly the
assumption that makes monitoring-only plants second-class, so capabilities are
declared, not inferred.

## Goal

Publish an accurate capability declaration in the simulator's retained status.

## Scope

- `capabilities.sensors[]` with `sensor_id`, `point`, `kinds[]`, health, `calibrated`
- `capabilities.actuators[]` with `actuator_id` and `kind`
- Derived from `--sensors` and a new `--actuators` flag, so a simulator can be configured with none
- `connectivity.mode` and `applied_policy_versions` reported in status

## Non-goals

- Edge-side ingestion (M4-011).
- Binding validation (M5-013).

## Dependencies

- M2-003

## Implementation notes

Support `--actuators ''` producing a device with **no** actuators. That
configuration is not an edge case to tolerate — it is the shape most real plants
have, and SCEN-106 depends on being able to simulate it.

Capabilities must match what the simulator actually publishes. A device that
declares `illuminance` and never sends it is a bug the conformance test should
catch, so derive the declaration from the same configuration that drives
sampling rather than hard-coding it.

## Acceptance criteria

- [ ] Declared capabilities match the sampled kinds exactly.
- [ ] `--actuators ''` produces a device with an empty `actuators` array that still runs.
- [ ] `sensor_id` values are stable across restarts.
- [ ] An uncalibrated sensor declares `calibrated: false` and publishes matching `quality`.
- [ ] `applied_policy_versions` is present and empty before any policy arrives.

## Verification

```bash
cargo test -p device-simulator capabilities::
```

## Tests required

- Declaration matches sampling.
- Empty-actuator device.
- Stability across restart.

## Documentation impact

- None.

## Files likely affected

```text
crates/device-simulator/src/status.rs
crates/device-simulator/src/cli.rs
```
