# Issue M4-011 — Ingest and expose device capabilities

**Milestone:** M4 · **PRD:** [PRD 040](../../prd/040-device-registry-and-health.md) · **Depends on:** M4-001

## Context

[ADR-016](../../adr/016-plant-binding-and-policy-model.md) requires the edge to
know what a device declared, and to reject any binding naming something it did
not. That starts with storing the declaration.

## Goal

Record declared capabilities and expose them for binding validation.

## Scope

- Parse `capabilities` from `device.status` into `device_capabilities`
- Expose sensors and actuators in `GET /api/v1/devices/{id}`
- Detect capability changes across reboots and raise `capabilities_changed`
- Provide a lookup used by binding validation in M5-013

## Non-goals

- Binding creation or validation (M5-013).

## Dependencies

- M4-001

## Implementation notes

A device that loses a capability across a reboot — a sensor that failed to
initialise — is a real and important signal. Raise an event rather than silently
overwriting, because a plant bound to that sensor is about to start refusing to
water and the operator needs the cause.

Store the declaration as rows rather than an opaque blob, so validation can query
"does device D have actuator A" without deserialising JSON on every check.

## Acceptance criteria

- [x] Declared capabilities are stored and exposed in the device API.
- [x] A device with no actuators is represented correctly, not as an error.
- [x] A capability disappearing across a reboot raises `capabilities_changed`.
- [x] The lookup answers capability queries without a JSON parse.
- [x] Re-declaring identical capabilities creates no event.

## Verification

```bash
cargo test -p edge-controller capabilities::
curl -s localhost:8080/api/v1/devices/plant-node-01 | jq .capabilities
```

## Tests required

- Ingestion and exposure.
- Capability-loss detection.
- No-actuator device.

## Documentation impact

- http-api-boundaries.md device response shape.

## Files likely affected

```text
crates/edge-controller/src/device/capabilities.rs
crates/storage/src/repo/device.rs
```
