# Issue M16-012 — Add the actuator maintenance state

**Milestone:** M16 · **PRD:** [PRD 160](../../prd/160-verified-watering.md) · **Depends on:** M16-011

## Context

Repeated failures need somewhere to accumulate. A plant lockout says "do not
water this plant"; a device status says "this node is reachable". Neither says
"this pump has failed to deliver three times and someone should look at the
tube", which is the sentence an operator actually needs.

The constraint is not to build a parallel status system. The project already has
`devices.status`, `PlantState`, `LockoutReason`, `sensor_stuck_state`, and a
latched pump fault; a fourth vocabulary that overlaps all of them would make
every screen ambiguous.

## Goal

`actuator_health` as a derived, explicit state on the existing device and plant
surfaces, with an operator clear that cannot actuate.

## Scope

- States: `Healthy`, `Degraded`, `NeedsInspection`, `Locked`.
- Transitions: `Degraded` on one `PartialDelivery` or stale calibration;
  `NeedsInspection` on repeated partials or one `NoFlow`; `Locked` on
  `UnexpectedFlow`, `OverDelivery`, or a latched witness or pump fault.
- Derived from `watering_deliveries` history, with the derived answer
  authoritative on read.
- `POST /api/v1/devices/{id}/actuators/{aid}/clear` — an explicit operator clear
  recording who and when.
- `plant_events` and `device_events` rows on every transition.
- Surfaced on `GET /devices/{id}` and on the plant view, not on a new one.

## Non-goals

- A new top-level status vocabulary. This attaches to the actuator, which had
  none, and reuses the device and plant surfaces for presentation.
- Auto-recovery. Every state above `Healthy` needs a person.
- Clearing a *latched device-side* fault. That still needs a reboot (M11-003);
  this clears the edge's record after inspection, and the two are documented as
  the two halves they are.

## Dependencies

- M16-011

## Implementation notes

**Derive on read, like `connectivity`.** `devices.connectivity_mode` is stored so
the liveness timer has somewhere to record its transition, and what is *reported*
re-checks `overdue_at` on every read — because a stored state needs a writer, and
a writer that dies leaves a device asleep for ever. The same trap applies here: a
stored actuator health that stops being updated is a pump reported healthy
because nothing wrote otherwise. Store the row for the transition, the event, and
the counter; derive the answer.

The clear endpoint has no override, force, or bypass semantics, and its doc
comment must say so. "Clear" next to an actuator is exactly the shape of thing a
future contributor would extend into "clear and water", and the API-boundaries
document should record that it cannot cause an actuation.

Clearing the edge record does not clear the device's latched fault, and the
reverse is also true. Report both, separately, on the device view — an operator
who clears one and finds the pump still refusing needs to see why without
reading source.

## Acceptance criteria

- [ ] Each transition fires on its documented trigger and no other.
- [ ] The reported state is derived on read, not read back from the column.
- [ ] Nothing above `Healthy` clears automatically.
- [ ] The clear endpoint records actor and time and cannot actuate.
- [ ] Device-side latched faults and edge-side health are reported separately.
- [ ] Transitions appear in the plant and device event histories.
- [ ] No new top-level status vocabulary is introduced.

## Verification

```bash
cargo test -p edge-controller delivery::health
cargo test -p edge-controller api::actuators
curl -s localhost:8080/api/v1/devices/plant-node-01 | jq .actuators
```

## Tests required

- Each transition and each non-transition.
- Derived-on-read: a stale column with a fresh history reports the history.
- The clear endpoint's authorisation record, and its inability to actuate.
- A device-side latched fault surviving an edge-side clear.

## Documentation impact

- `docs/protocol/http-api-boundaries.md`: the actuator view and the clear
  endpoint, with its explicit no-actuation note.
- PRD 040 state model: the actuator health states alongside device health.

## Files likely affected

```text
crates/edge-controller/src/delivery/health.rs
crates/edge-controller/src/api/devices.rs
crates/edge-controller/src/api/actuators.rs
crates/storage/src/repo/delivery.rs
docs/protocol/http-api-boundaries.md
```
