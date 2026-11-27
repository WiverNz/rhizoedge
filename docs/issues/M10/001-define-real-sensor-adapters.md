# Issue M10-001 — Define the real sensor adapter structure

> **Execution scope, 2026-11-22:** **Status: IMPLEMENTED — software prototype; physical verification pending.** Apply the current milestone PRD prototype scope and [testing policy](../../testing/strategy.md). Only relevant tests and a basic smoke are required; extended campaigns below are backlog reference, not mandatory local reruns. See [ROADMAP](../../../ROADMAP.md).


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

## Implemented prototype slice

Runtime NVS provisioning selects absent, fake or SEN0601 soil adapters without
changing `SoilSensor`. The requested slice also implements SEN0601 function-03
reads, CRC/frame validation, signed temperature decoding, and up to three unique
addresses on the board UART. Existing telemetry/MQTT paths carry stable sensor
IDs and points; measurements remain uncalibrated. See the
[firmware configuration instructions](../../../firmware/README.md#sen0601-prototype-configuration-m10-001).
Focused host tests and the ESP32 release build pass. Wiring and real measurements
remain unverified; this does not close M10's physical acceptance criteria.

## Non-goals

- Generic register maps, analogue adapters, calibration campaigns and pump control.

## Dependencies

- M9-005
- M9-010
- M9-012

## Implementation notes

Reuse the existing trait and Edge path. Inspect only the relevant adapter and
configuration code; fix concrete integration defects without redesigning the
architecture or treating zero Edge changes as an acceptance gate.

## Acceptance criteria

- [x] Sensor kind is configuration-selected.
- [x] The trait is unchanged from M9.
- [x] Fakes still work for host tests.
- [x] An unknown kind fails at startup with a clear message.

## Verification

```bash
cd firmware/node-app
cargo test --lib sensors
cargo test --lib provision
```

## Tests required

- Factory selection.
- Unknown kind handling.

## Documentation impact

- Firmware README: serial configuration and physical-verification limits.

## Files likely affected

```text
firmware/node-app/src/sensors.rs
firmware/esp32-node/src/hal/soil_bus.rs
```
