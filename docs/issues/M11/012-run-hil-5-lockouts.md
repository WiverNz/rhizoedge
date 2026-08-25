# Issue M11-012 — Run HIL-5 safety hardware lockout verification

**Milestone:** M11 · **PRD:** [PRD 110](../../prd/110-real-pump-and-safety-hardware.md) · **Depends on:** M11-011, M11-007

## Context

SAFETY-003 and SAFETY-004 against real sensors and real water, including the
manual-refusal path that ADR-006 deliberately encodes.

## Goal

Verify leak and tank lockouts physically.

## Scope

- Wet the leak sensor -> lockout within one tick; queued dose does not run
- **`POST /water` while wet -> 409**
- Clear while wet -> 409; clear when dry -> succeeds
- Leak during an active dose -> pump stops, partial delivery recorded
- Drain the reservoir -> lockout; device refuses independently
- Disconnect the tank sensor -> lockout
- Disconnect the soil probe -> `SensorFault`; **manual watering still works**

## Non-goals

- Any plant.

## Dependencies

- M11-011
- M11-007

## Implementation notes

The last item verifies the deliberate asymmetry: manual watering is permitted
under sensor fault (a human has looked at the plant) but never under leak (a
human has not yet looked at the floor). Confirm both halves.

## Acceptance criteria

- [ ] A wet leak sensor locks out within one control tick.
- [ ] `POST /water` returns 409 while wet.
- [ ] Clearing while wet returns 409; clearing when dry succeeds.
- [ ] A leak during a dose stops the pump and records partial delivery.
- [ ] A drained reservoir locks out and the device refuses independently.
- [ ] A disconnected tank sensor locks out.
- [ ] **A disconnected soil probe blocks automatic but permits manual watering.**

## Verification

```bash
# manual: HIL-5 checklist
```

## Tests required

- The HIL-5 checklist.

## Documentation impact

- hil-runs record.

## Files likely affected

```text
docs/testing/hil-runs/
```
