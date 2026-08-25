# Issue M10-011 — M10 verification and exit criteria

**Milestone:** M10 · **PRD:** [PRD 100](../../prd/100-real-soil-sensor.md) · **Depends on:** M10-001, M10-002, M10-003, M10-004, M10-005, M10-006, M10-007, M10-008, M10-009, M10-010

## Context

Final gate for M10. The decisive criterion is that **no edge-side code
changed** to accommodate real sensors — that is the test of M9's abstraction.

## Goal

Verify every PRD 100 acceptance criterion.

## Scope

- Host tests, hardware readings, and the validation record
- Confirm zero edge-side changes
- Update ROADMAP.md; record the report

## Non-goals

- The pump (M11).

## Dependencies

- M10-001
- M10-002
- M10-003
- M10-004
- M10-005
- M10-006
- M10-007
- M10-008
- M10-009
- M10-010

## Implementation notes

Run `git diff` across `crates/` for the whole milestone. If the edge changed
to accommodate a sensor, the trait boundary leaked and it is worth understanding
why before M11 adds actuators behind the same pattern.

## Acceptance criteria

- [ ] Real readings flow end to end: sensor to ESP32 to MQTT to edge to SQLite to cloud.
- [ ] Switching analogue to Modbus is a configuration change.
- [ ] Adding a probe model is a register-map entry.
- [ ] Unplugging the probe produces `null`, unhealthy, and a `SensorFault` lockout.
- [ ] An uncalibrated sensor publishes `null`.
- [ ] Readings match the gravimetric reference within documented bounds.
- [ ] Four weeks of drift are recorded.
- [ ] **`git diff` shows no edge-side change for sensor support.**
- [ ] ROADMAP.md updated; report recorded.

## Verification

```bash
cd firmware/esp32-node && cargo test
git diff --stat <m10-start>..HEAD -- crates/   # expect no sensor-driven changes
```

## Tests required

- Host suite plus the hardware checklist.

## Documentation impact

- ROADMAP.md.
- Validation record.
- Milestone report.

## Files likely affected

```text
ROADMAP.md
```
