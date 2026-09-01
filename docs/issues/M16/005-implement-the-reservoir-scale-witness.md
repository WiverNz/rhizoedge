# Issue M16-005 — Implement the reservoir-scale witness

**Milestone:** M16 · **PRD:** [PRD 160](../../prd/160-verified-watering.md) · **Depends on:** M16-004

## Context

[ADR-020](../../adr/020-verified-watering-and-delivery-evidence.md) §4 chose a
load cell under the reservoir over an inline flow meter, and
[PRD 110](../../prd/110-real-pump-and-safety-hardware.md) §Open questions 3 is
why: at a calibrated 8.2 ml/s the pump moves about 0.49 L/min, and inexpensive
turbine meters are least accurate at the bottom of a range that starts above it.
A scale measures volume by subtraction, at 1 g per ml, with no minimum flow rate
— and it is the same part class the project already uses for `pot_weight`.

## Goal

`ReservoirScale`, the one new piece of hardware in this milestone, behind the
trait from M16-004.

## Scope

- `ReservoirScale<S: Scale>` implementing `DeliveryWitness` over the existing
  scale driver.
- Baseline capture, and `cumulative_ml` as `(baseline_g - current_g)` with a
  documented 1 g = 1 ml conversion.
- Health: unreadable, non-monotonic, implausible rate, or a step beyond the
  reservoir's plausible capacity all degrade or fault rather than reporting a
  volume.
- Publishing `reservoir_weight` samples on the normal telemetry cadence.
- Reservoir tare, reusing the existing `command.tare` path rather than adding a
  command.
- Hardware config: the new sensor in the device config schema.
- Hardware guide: the part, its wiring, and its placement.

## Non-goals

- The execution state machine. M16-007.
- Disturbance rejection beyond plausibility bounds. PRD 160 §Open questions 1 is
  a bench question and M16-015 answers it with measurements.
- A flow meter.

## Dependencies

- M16-004

## Implementation notes

**1 g = 1 ml is exact enough and its error must be stated, not hidden.** Water
is 0.998 g/ml at 20 °C, and nutrient solution is denser. Over a 40 ml dose the
error is well under a millilitre — smaller than the load cell's own noise — so
the conversion is a documented constant rather than a temperature-compensated
model. Say so in the doc comment, with the number, so nobody later "improves" it
into a dependency on a soil thermometer.

The baseline is captured **once per dose**, immediately before actuation and
after the NVS in-flight write. A baseline captured after the pump starts is
already wrong; a baseline reused across doses accumulates every disturbance
between them.

Reject the disturbances a scale under a reservoir actually sees: a **rise** in
mass during a dose is a refill or a hand on the shelf and is `Faulted`, never a
negative delivery; a step larger than the reservoir can hold is implausible; and
a rate beyond `MAX_PLAUSIBLE_FLOW_ML_S` means the reading, not the water, is
wrong.

An HX711 is not free on a battery node — it must be power-gated with the other
peripherals (M9-020) and its per-dose energy is part of the budget M10-012
measures. PRD 160 §Open questions 5 owns whether a battery node ships one at all;
this issue must not decide it by making the witness mandatory.

The hardware guide is **not normative** and its numbers are starting points. Add
the part and the wiring; do not derive a threshold, a constant, or a firmware
default from it.

## Acceptance criteria

- [ ] `ReservoirScale` implements the trait with no change to it.
- [ ] The baseline is taken once per dose, after the in-flight write and before
      actuation.
- [ ] A mass rise during a dose is `Faulted`, never a negative volume.
- [ ] An implausible step or rate degrades rather than reporting a volume.
- [ ] An unreadable scale answers `None`, never `0.0`.
- [ ] `reservoir_weight` is published on the normal telemetry cadence.
- [ ] Reservoir tare reuses `command.tare`; no new command exists.
- [ ] The 1 g = 1 ml conversion and its error bound are documented in the code.
- [ ] The sensor is power-gated on a battery node.

## Verification

```bash
cd firmware/esp32-node && cargo test witness::reservoir
cd firmware/esp32-node && cargo test scale::
```

## Tests required

- Cumulative volume over a scripted mass series.
- Rise-during-dose, implausible step, implausible rate, and unreadable cases.
- Baseline lifecycle across a dose and across a reboot.
- Telemetry publication of the new kind.

## Documentation impact

- `docs/hardware/home-node-hardware-guide.md`: the reservoir load cell, its
  amplifier, wiring, and placement.
- `docs/protocol/mqtt-v1.md` §5.7: the device config schema gains the sensor.
- PRD 160 §Data model, if the health rules deviate.

## Files likely affected

```text
firmware/esp32-node/src/witness/reservoir.rs
firmware/esp32-node/src/config.rs
docs/hardware/home-node-hardware-guide.md
docs/protocol/mqtt-v1.md
```
