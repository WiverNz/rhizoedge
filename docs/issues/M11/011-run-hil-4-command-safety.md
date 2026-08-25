# Issue M11-011 — Run HIL-4 command safety verification

**Milestone:** M11 · **PRD:** [PRD 110](../../prd/110-real-pump-and-safety-hardware.md) · **Depends on:** M11-010

## Context

**Tests the device's independent veto** by publishing directly to the MQTT
command topic, bypassing the edge entirely. This is where SAFETY-007 stops being
a claim about the simulator.

## Goal

Verify every command safety rule against real water.

## Scope

- `requested_ml: 10000` -> **measure the cup**, never above the hard limit
- Expired command -> rejected, pump silent
- Same `command_id` three times -> one actuation
- Negative and zero volumes -> rejected
- Commands past `FIRMWARE_MAX_DAILY_ML` -> rejected
- Power cycle then repeat a previous `command_id` -> still deduplicated
- SNTP blocked -> `clock_unsynced` refusal

## Non-goals

- Any plant.

## Dependencies

- M11-010

## Implementation notes

Measure the cup for the oversized command. Reading a log that says 'clamped'
proves the firmware believes it clamped; the cup proves it did.

Any divergence from simulator behaviour invalidates every simulator-based safety
test until resolved — treat it as a milestone-blocking finding, not a note.

## Acceptance criteria

- [ ] `requested_ml: 10000` delivers no more than `FIRMWARE_MAX_ML_PER_RUN`, **measured**.
- [ ] An expired command is rejected with the pump silent.
- [ ] Three identical `command_id`s cause one actuation and three results.
- [ ] Negative and zero volumes are rejected.
- [ ] The device daily cap is enforced.
- [ ] Dedup survives a power cycle.
- [ ] Blocked SNTP refuses every command.
- [ ] **Every behaviour matches the simulator exactly.**

## Verification

```bash
mosquitto_pub -t 'rhizo/v1/devices/plant-node-01/commands/water' -q 1 -m '{...requested_ml:10000...}'
# then measure the cup
```

## Tests required

- The HIL-4 checklist.

## Documentation impact

- hil-runs record.

## Files likely affected

```text
docs/testing/hil-runs/
```
