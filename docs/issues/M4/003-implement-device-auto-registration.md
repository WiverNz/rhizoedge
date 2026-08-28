# Issue M4-003 — Implement device auto-registration without plant attachment

**Milestone:** M4 · **PRD:** [PRD 040](../../prd/040-device-registry-and-health.md) · **Depends on:** M4-001

## Context

ADR-012 applies SAFETY-012 to onboarding: a device that appears on the network
is registered as a **device**, never as a plant. With no plant, there is no
profile, no `auto_watering_enabled`, and no path to actuation.

## Goal

Register unknown devices safely.

## Scope

- An unknown `device_id` inserts a `devices` row
- **No plant is created, ever**
- The device appears in the device list for the operator
- A `device_registered` event
- Handle a `boot_id` thrashing pattern (two devices claiming one id) as an event

## Non-goals

- Plant creation — that is an explicit operator action (M5-002).

## Dependencies

- M4-001

## Implementation notes

Write an explicit test asserting that after auto-registration the `plants`
table is empty. The property is easy to state and easy to break later by someone
adding a helpful 'create a default plant' convenience.

Broker ACLs prevent two devices sharing an id, but if `boot_id` thrashes anyway
it indicates a misconfiguration worth surfacing.

## Acceptance criteria

- [x] An unknown device is registered on its first message.
- [x] **`plants` remains empty** after registration.
- [x] The device appears in `GET /api/v1/devices` with no plant.
- [x] A `device_registered` event is recorded.
- [x] Rapid `boot_id` alternation raises an event.

## Verification

```bash
cargo test -p edge-controller device::registration
cargo test --test integration auto_registration_creates_no_plant
```

## Tests required

- Registration.
- **The no-plant assertion.**
- boot_id thrashing detection.

## Documentation impact

- None.

## Files likely affected

```text
crates/edge-controller/src/pipeline/status.rs
crates/storage/src/repo/device.rs
```
