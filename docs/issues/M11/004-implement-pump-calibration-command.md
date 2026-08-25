# Issue M11-004 — Implement the pump calibration command

**Milestone:** M11 · **PRD:** [PRD 110](../../prd/110-real-pump-and-safety-hardware.md) · **Depends on:** M11-001

## Context

PRD 110 F-110-10 through F-110-14. Calibration is how requested millilitres
become a duration, and its accuracy bounds everything downstream.

## Goal

Support measured calibration of pump flow.

## Scope

- `command.calibrate` running the pump for a fixed duration
- Subject to the same safety validation as a water command
- Calibration volume counts toward `FIRMWARE_MAX_DAILY_ML`
- `ml_per_second` stored in device config with a `calibrated_at` date
- **Rejected if the standard deviation exceeds 5% of the mean**

## Non-goals

- Automatic calibration.

## Dependencies

- M11-001

## Implementation notes

The variance rejection matters: high variance means an air lock, a partially
occluded tube, or a failing pump head. Averaging it away produces a calibration
that is wrong in a way nobody notices until doses are consistently short.

Recording `calibrated_at` is what makes drift detectable across months —
peristaltic tubing hardens and flow degrades predictably.

## Acceptance criteria

- [ ] Calibration runs the pump for the requested duration.
- [ ] It passes the same safety checks as a water command.
- [ ] Its volume counts toward the device daily cap.
- [ ] `ml_per_second` and `calibrated_at` are stored.
- [ ] **A five-run set with σ above 5% of the mean is rejected.**
- [ ] A 40 ml request then delivers within ±10%.

## Verification

```bash
curl -X POST localhost:8080/api/v1/devices/plant-node-01/commands/calibrate -d '{"run_seconds":10}'
```

## Tests required

- Calibration command path.
- Safety validation applied.
- **Variance rejection.**
- Daily cap accounting.

## Documentation impact

- Calibration procedure in hardware-in-the-loop.md HIL-3.

## Files likely affected

```text
firmware/esp32-node/src/app/calibrate.rs
```
