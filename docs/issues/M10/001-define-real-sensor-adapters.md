# Issue M10-001 — Define the real sensor adapter structure

**Milestone:** M10 · **PRD:** [PRD 100](../../prd/100-real-soil-sensor.md) · **Depends on:** M9-022

## Context

PRD 100 F-100-01/02: real adapters implement the **existing** trait, and the
sensor type is selected by configuration rather than by a separate build.

## Goal

Establish the adapter selection mechanism.

## Scope

- Sensor kind selected from device config
- A factory constructing the right adapter at boot
- The `SoilSensor` trait **unchanged** from M9
- Fake adapters retained for host tests

## Non-goals

- The adapters themselves (M10-004, M10-005).

## Dependencies

- M9-022

## Implementation notes

If the trait needs changing here, that is a signal M9's abstraction was
wrong — investigate rather than widening it. The measure of M9's success is that
M10 changes no edge code and no trait.

## Acceptance criteria

- [ ] Sensor kind is configuration-selected.
- [ ] The trait is unchanged from M9.
- [ ] Fakes still work for host tests.
- [ ] An unknown kind fails at startup with a clear message.

## Verification

```bash
cd firmware/esp32-node && cargo test sensors::factory
```

## Tests required

- Factory selection.
- Unknown kind handling.

## Documentation impact

- None.

## Files likely affected

```text
firmware/esp32-node/src/sensors/factory.rs
```
