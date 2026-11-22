# Issue M10-001 — Define the real sensor adapter structure

> **Execution scope, 2026-11-22:** **Status: PLANNED — prototype slice.** Apply the current milestone PRD prototype scope and [testing policy](../../testing/strategy.md). Only relevant tests and a basic smoke are required; extended campaigns below are backlog reference, not mandatory local reruns. See [ROADMAP](../../../ROADMAP.md).


**Milestone:** M10 · **PRD:** [PRD 100](../../prd/100-real-soil-sensor.md) · **Depends on:** M9-005, M9-010, M9-012

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

- M9-005
- M9-010
- M9-012

## Implementation notes

Reuse the existing trait and Edge path. Inspect only the relevant adapter and
configuration code; fix concrete integration defects without redesigning the
architecture or treating zero Edge changes as an acceptance gate.

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
