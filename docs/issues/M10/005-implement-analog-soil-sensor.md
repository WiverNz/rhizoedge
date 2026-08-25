# Issue M10-005 — Implement the analogue capacitive soil sensor adapter

**Milestone:** M10 · **PRD:** [PRD 100](../../prd/100-real-soil-sensor.md) · **Depends on:** M10-001

## Context

PRD 100 open question 3: the analogue path de-risks M10 by separating
'sensor wiring' from 'Modbus wiring', and it is roughly a hundred lines.

## Goal

Read a capacitive probe via ADC.

## Scope

- `AnalogSoilSensor` implementing `SoilSensor`
- Oversampling: 16 samples, median
- Calibration mapping raw to VWC
- Rail readings (open or shorted) detected as errors
- No temperature or EC — fields omitted

## Non-goals

- Claiming accuracy — a capacitive probe reports a repeatable index, not certified VWC.

## Dependencies

- M10-001

## Implementation notes

Rail detection is the useful failure check: an unplugged probe reads at one
extreme, a shorted one at the other, and both are outside any plausible
calibrated range. Treat them as read errors rather than as very dry or very wet.

Median rather than mean rejects the occasional ADC outlier without a filter that
lags.

## Acceptance criteria

- [ ] Readings map from raw to VWC via calibration.
- [ ] Oversampling with a median is applied.
- [ ] Rail readings are detected as errors, not as extreme moisture.
- [ ] Temperature and EC fields are omitted, not zeroed.
- [ ] Host tests cover it with a fake ADC.

## Verification

```bash
cd firmware/esp32-node && cargo test sensors::analog_soil
```

## Tests required

- Calibration mapping.
- Median filtering.
- **Rail detection.**
- Field omission.

## Documentation impact

- None.

## Files likely affected

```text
firmware/esp32-node/src/sensors/soil/analog.rs
```
