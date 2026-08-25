# Issue M8-008 — Implement the device failure scenarios

**Milestone:** M8 · **PRD:** [PRD 080](../../prd/080-end-to-end-test-environment.md) · **Depends on:** M8-006

## Context

SCEN-022, -023, -025, -031. These prove SAFETY-005 and SAFETY-002 in the
assembled system.

## Goal

Prove device failure handling end to end.

## Scope

- SCEN-022 stale sensor locks out and auto-recovers
- SCEN-023 invalid sensor values null the field and lock out
- SCEN-025 clock unsynced refuses commands while telemetry continues
- SCEN-031 queued command after a long disconnect is refused

## Non-goals

- Device restart mid-dose, which needs firmware (M9).

## Dependencies

- M8-006

## Implementation notes

SCEN-025 is worth care: it must confirm that **monitoring continues normally**
while watering is refused. A device with an unsynced clock is still a useful
sensor, and an implementation that stopped telemetry too would be wrong.

SCEN-031 also implicitly verifies `clean_session = true` — with a persistent
session the broker would have queued the command.

## Acceptance criteria

- [ ] SCEN-022: lockout after the staleness window, auto-clear on resumption, **no command issued**.
- [ ] SCEN-023: field nulled, event raised, `SensorFault` lockout.
- [ ] SCEN-025: command refused with `clock_unsynced`; **telemetry continues**.
- [ ] SCEN-031: no queued command is delivered or executed.
- [ ] Each lockout is visible in the API with its reason.

## Verification

```bash
... run --rm scenario-runner --scenario scenario_stale_sensor scenario_clock_unsynced
```

## Tests required

- SCEN-022, -023, -025, -031.

## Documentation impact

- None.

## Files likely affected

```text
test/scenarios/device_failures.rs
```
