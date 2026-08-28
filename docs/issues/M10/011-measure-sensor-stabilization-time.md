# Issue M10-011 — Measure sensor power-on stabilization time

**Milestone:** M10 · **PRD:** [PRD 100](../../prd/100-real-soil-sensor.md) · **Depends on:** M10-010

## Context

M9-020 gates the sensor rail and waits `sensor_warmup_ms` before reading, and it
deliberately hardcodes no value for any part — including the SEN0601. Nobody
knows the real number. The datasheet, where it says anything at all, describes a
part on a bench at room temperature and not a probe in cold soil at the end of a
few metres of cable.

The number matters twice. Read too early and the probe returns a plausible wrong
value, which is the worst failure mode a sensor has. Wait too long and the
warm-up dominates the wake cycle's energy — at a 15-minute interval, every extra
second of warm-up is roughly 96 extra seconds of powered sensor per day, and the
budget in M10-012 is measured in single-digit milliamp-hours.

## Goal

Establish, by measurement, how long each supported sensor takes to produce a
stable usable reading after power-on.

## Scope

- The measurement, per supported sensor (Modbus SEN0601 and the analogue probe):

  ```text
  sensor power-on
    → time until stable usable reading
  ```

- A repeatable procedure: power the rail, poll at a fixed short interval, record
  every reading with its offset from power-on, until the value has settled
- An explicit, stated definition of "stable": the reading is within a documented
  band of its final value and stays there for a documented number of consecutive
  polls — chosen and written down **before** the data is collected
- At least 20 power-on cycles per sensor, so the figure has a spread rather than
  being one anecdote
- Repeat at the extremes of the deployment envelope: dry soil and saturated soil,
  and at a cold temperature as well as room temperature, since a warm-up figure
  taken only at 22 °C on a bench will be wrong on a balcony in February
- A recommended `sensor_warmup_ms` derived from the measured distribution with a
  stated margin, and the margin's reasoning
- Results recorded in `docs/testing/hil-runs/` with the raw series, not only the
  summary

## Non-goals

- Changing the firmware default. This issue produces a measured number and a
  recommendation; M9-020 already takes it from configuration, which is the point.
- Optimising the warm-up. If the answer is uncomfortably long, that is a finding
  for M10-012's budget, not a reason to shorten the wait.
- Measuring current draw. That is M10-012, and it uses this issue's timing as an
  input.

## Dependencies

- M10-010

## Implementation notes

Define "stable" first and record the definition, because the temptation once the
data is in front of you is to pick the definition that gives the pleasant answer.
A serviceable starting point, to be confirmed or replaced before collection:
within ±1 % VWC of the final value for five consecutive polls at 100 ms.

Poll faster than the expected settling time — 50–100 ms — or the resolution of
the answer is the poll interval, and a figure of "under 500 ms" derived from
500 ms polls says nothing.

Watch for two distinct effects and report them separately, because they have
different fixes. **Electrical settling** is the supply rail and the transceiver
coming up, and is short. **Sensor settling** is the probe's own measurement
converging, and for a capacitive probe in soil it can be much longer. If the
Modbus device also needs a startup delay before it will answer at all, that is a
third number — time to first valid response, distinct from time to stable value.

If the measured figure makes a 15-minute wake interval energetically
unattractive, say so plainly in the results. That is exactly the kind of finding
this measurement exists to surface, and it feeds directly into whether the
reference workload in [PRD 140](../../prd/140-field-readiness.md) is realistic.

## Acceptance criteria

- [ ] The stability definition is written down before data collection and
      recorded with the results.
- [ ] At least 20 power-on cycles measured per supported sensor.
- [ ] Time-to-stable reported as a distribution — median, worst case, spread —
      not as a single number.
- [ ] Measured in dry and saturated soil, and at cold as well as room
      temperature.
- [ ] Time to first valid Modbus response reported separately from time to stable
      value.
- [ ] A recommended `sensor_warmup_ms` with an explicit margin and its reasoning.
- [ ] Raw series recorded in `docs/testing/hil-runs/`, not just the summary.
- [ ] The energy consequence at a 15-minute wake interval is stated in the
      results and referenced by M10-012.

## Verification

```bash
# instrumented firmware build with rail control and fast polling
cd firmware/esp32-node && cargo run --release --bin warmup-probe
# results recorded in docs/testing/hil-runs/sensor-warmup.md
cargo run -p rhizo-docscheck
```

## Tests required

- Manual hardware measurement; the recorded series is the artefact.
- A host test asserting the recommended value round-trips through configuration
  and is honoured by M9-020's warm-up gate.

## Documentation impact

- `docs/testing/hil-runs/sensor-warmup.md` — new.
- [PRD 100](../../prd/100-real-soil-sensor.md) — the measured figure.
- [ADR-018](../../adr/018-battery-and-deep-sleep-device-mode.md) §8 — an input to
  the budget.

## Files likely affected

```text
docs/testing/hil-runs/sensor-warmup.md
docs/prd/100-real-soil-sensor.md
firmware/esp32-node/src/bin/warmup-probe.rs
```
