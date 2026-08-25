# Issue M11-006 — Implement the leak sensor adapter

**Milestone:** M11 · **PRD:** [PRD 110](../../prd/110-real-pump-and-safety-hardware.md) · **Depends on:** M10-011

## Context

SAFETY-003. A leak means water is already going where it should not; the one
thing that must not happen next is more water.

## Goal

Detect a leak and refuse everything.

## Scope

- `RealLeakSensor` implementing the existing trait
- Conductive pad or optical, active level configurable
- **A leak transition published immediately**, not at the next interval
- The device refuses all water commands while asserted
- Sensor absent or failed produces `null` -> Unknown -> refusal

## Non-goals

- Locating the leak.

## Dependencies

- M10-011

## Implementation notes

Immediate publication is the requirement: an hour-late leak notification is
useless. Trigger a publish on the transition rather than waiting for the
telemetry schedule.

Debounce briefly (a few hundred milliseconds) to avoid a splash triggering a
permanent lockout — but err toward triggering, since a false lockout is cheap
and a missed leak is not.

## Acceptance criteria

- [ ] A leak is detected and published immediately.
- [ ] The device refuses all water commands while asserted.
- [ ] A disconnected sensor produces `null` and a refusal.
- [ ] Brief debouncing prevents splash false positives without missing a real leak.
- [ ] The edge locks out within one control tick.

## Verification

```bash
cd firmware/esp32-node && cargo test sensors::leak
cargo test safety_003
```

## Tests required

- Detection and immediate publication.
- Refusal while asserted.
- **Null handling.**
- Debounce behaviour.

## Documentation impact

- None.

## Files likely affected

```text
firmware/esp32-node/src/sensors/leak/real.rs
```
