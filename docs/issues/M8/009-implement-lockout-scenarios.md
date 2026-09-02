# Issue M8-009 — Implement the safety lockout scenarios

**Milestone:** M8 · **PRD:** [PRD 080](../../prd/080-end-to-end-test-environment.md) · **Depends on:** M8-006

## Context

SCEN-040, -042, -044. SCEN-040 is the one that proves manual watering cannot
bypass a leak — the asymmetry ADR-006 deliberately encodes.

## Goal

Prove the hardware-safety lockouts end to end.

## Scope

- SCEN-040 leak: automatic stops, **manual API returns 409**, clear refused while wet, explicit reset works
- SCEN-042 tank empty: lockout, device refuses independently, refill auto-clears
- SCEN-044 no delivery: lockout after two unresponsive doses, escalation stops

## Non-goals

- Physical hardware (M11).

## Dependencies

- M8-006

## Implementation notes

SCEN-040's manual-refusal assertion is the important one. It must call the
real endpoint and assert a 409 with the leak reason, and confirm via the MQTT
spy that nothing was published.

SCEN-044 must assert that a **third** dose is never issued. The failure being
prevented is escalation into a plant that may already be flooded by a leak the
sensor cannot see.

## Acceptance criteria

- [x] SCEN-040: automatic watering stops; `POST /water` returns **409**; nothing published.
- [x] SCEN-040: clearing while wet returns 409; clearing when dry succeeds.
- [x] SCEN-042: lockout; the device also refuses; refill auto-clears.
- [x] SCEN-044: lockout after two unresponsive doses; **no third dose**.
- [x] SCEN-044's lockout requires an explicit clear.

## Verification

```bash
... run --rm scenario-runner --scenario scenario_leak scenario_tank_empty scenario_no_delivery
```

## Tests required

- SCEN-040, -042, -044.

## Documentation impact

- None.

## Files likely affected

```text
test/scenarios/lockouts.rs
```
