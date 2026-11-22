# PRD 100 — Real Soil Sensor Integration

## Prototype scope — supersession, 2026-11-22

M10 is **READY** for SEN0601 via ESP32-C3/RS485. Entry uses the demonstrated
M9 traits/config/telemetry, not full M9 acceptance. Implement the minimum Modbus
read client and SEN0601 register map first, reusing existing Edge storage and API.
One real sensor and one plant suffice; no analogue adapter or new purchase is
required. Basic units/scaling and calibration provenance, startup settling in the
intended soil, plausible repeatable readings and fail-closed read errors are
required. Manufacturer-scaled moisture must be labelled by its actual calibration
basis, not claimed as laboratory-validated VWC. Missing conversion/calibration
information remains invalid; never pass raw counts as physical units.

For this prototype, F-100-05, generic model expansion, optional compensation,
F-100-30…32 and the full F-100-40…49 campaigns are deferred. Basic settling still
must be observed before using a reading; use always-on power initially. M10-005
and M10-010…012 stay open in the hardening backlog. No cloud delivery, four-week
wait or zero-Edge-diff rule gates completion. The original detailed design below
is retained for later scope; this section and current acceptance take precedence.

See [ROADMAP](../../ROADMAP.md) and [testing policy](../testing/strategy.md).


**Milestone:** M10 · **Status:** READY · **Depends on:** Existing M9 sensor/config/telemetry interfaces; not full M9 closure

## Summary

Replace the fake soil sensor with real hardware behind the existing
`SoilSensor` trait — SEN0601 over RS485/Modbus RTU first — with configured scaling,
calibration provenance and fail-closed errors. Analogue support is deferred.

## Problem

Soil moisture sensing is where cheap hardware meets soil physics, and both
disappoint. Capacitive probes drift, corrode at the junction, respond to
temperature and salinity, and report percentages that are not volumetric water
content in any defensible sense. Industrial RS485 probes are better but cost
significantly more and need a transceiver and wiring discipline.

This milestone must produce readings the safety logic can trust — or must make
their untrustworthiness visible.

## Goals

1. A `SoilSensor` implementation for real hardware.
2. A generic Modbus RTU abstraction, not a single-model driver.
3. Calibration from raw readings to VWC.
4. Robust handling of read errors, timeouts, and implausible values.
5. Validation of readings against a reference so the numbers mean something.

## Non-goals

- Laboratory-grade accuracy. The system needs *repeatable* readings with known
  error bounds, not certified ones.
- **N/P/K inference from EC.** Explicitly and permanently out of scope: cheap
  NPK probes report values derived from EC by an undisclosed formula, and
  presenting them as nutrient measurements would be a false claim
  ([PRD 140](140-field-readiness.md)).
- pH sensing (deferred).
- Multi-depth probes (M14 — the `point` column is already reserved).

## User/system flows

```text
operator wires probe → configures sensor type and calibration → device reboots
   → real readings replace fake ones
   → edge sees no difference: same MQTT, same validation, same lockouts
```

Calibration:

```text
probe in air        → record raw_dry
probe in water      → record raw_wet
probe in known soil → optional two-point check
   → store calibration in device config → linear mapping to VWC
```

## Functional requirements

### Sensor abstraction

| ID | Requirement |
|---|---|
| F-100-01 | Real adapters implement the **existing** `SoilSensor` trait; the trait does not change |
| F-100-02 | Sensor type selected by configuration, not by a separate firmware build |
| F-100-03 | A generic Modbus RTU client: read holding/input registers, configurable slave address, register map, and scaling |
| F-100-04 | Register maps are **data, not code** — a new probe model is a configuration entry |
| F-100-05 | Analogue capacitive adapter via ADC with oversampling (16 samples, median) |
| F-100-06 | RS485 half-duplex direction switching is provided by the interface hardware. On the reference build that is the **DFRobot DFR0845** isolated RS485-to-UART module, which switches direction itself: firmware drives plain UART TX/RX, and there is no DE or /RE GPIO and no firmware-configurable turnaround timing. A future board profile with a bare transceiver would supply DE/RE control from its own board layer (ADR-007) |

### Calibration

| ID | Requirement |
|---|---|
| F-100-10 | Two-point calibration (`raw_dry`, `raw_wet`) → linear VWC mapping |
| F-100-11 | Calibration stored in device config, versioned like any other config |
| F-100-12 | Readings outside the calibrated range are clamped **and flagged**, never silently extrapolated |
| F-100-13 | Optional temperature compensation where the probe reports temperature |
| F-100-14 | An uncalibrated sensor publishes `moisture_vwc: null` — **not** a raw value pretending to be VWC |

### Error handling

| ID | Requirement |
|---|---|
| F-100-20 | Modbus timeout, CRC error, and exception response handled distinctly and counted separately |
| F-100-21 | Read failure publishes `null` for the affected field, never a stale or default value |
| F-100-22 | 3 consecutive failures mark the sensor unhealthy in status |
| F-100-23 | Stuck detection: `stuck_sample_count` bit-identical raw readings marks it unhealthy |
| F-100-24 | Out-of-physical-range raw values rejected before conversion |
| F-100-25 | Bus contention and framing errors do not stall the telemetry loop |

### Validation

| ID | Requirement |
|---|---|
| F-100-30 | Readings compared against a gravimetric reference at three moisture levels; error bounds documented |
| F-100-31 | Drift observed over ≥ 4 weeks and recorded |
| F-100-32 | Documented error bounds feed the recommendation `confidence` |

### Power and stabilization (M10-011, M10-012)

| ID | Requirement |
|---|---|
| F-100-40 | **Sensor power-on → time until stable usable reading** measured per supported sensor, over ≥ 20 power-on cycles, reported as a distribution rather than a single number |
| F-100-41 | The definition of "stable" written down **before** data collection and recorded with the results |
| F-100-42 | Measured in dry and saturated soil and at cold as well as room temperature — a bench figure at 22 °C is not a balcony figure in February |
| F-100-43 | Time to first valid Modbus response reported **separately** from time to stable value; electrical settling reported separately from sensor settling |
| F-100-44 | A recommended `sensor_warmup_ms` derived from the measured distribution with a stated margin and its reasoning; the firmware takes it from configuration (F-090-56) and hardcodes nothing |
| F-100-45 | **Complete-system sleep current** measured on the assembled device with sensor and RS485 attached, at a stated supply voltage, with the instrument named |
| F-100-46 | **Chip deep-sleep current reported separately and explicitly labelled as not being the system figure** — the two are routinely confused and differ by an order of magnitude or more |
| F-100-47 | Wake-cycle energy integrated and broken into named phases (rail-on and warm-up, sampling, Wi-Fi association, MQTT, receive window, sleep entry); a watering wake measured separately |
| F-100-48 | The energy-budget model recorded with **every input labelled measured or assumed**, the dominant term identified, and a sensitivity statement per input |
| F-100-49 | An explicit verdict on the ≥ 6-month target at a 15-minute wake interval, with the numbers behind it; until it exists, every autonomy figure in the repository stays labelled a target |

F-100-40 is here rather than in M9 because it directly sets the battery energy
budget: at a 15-minute wake interval, every extra second of warm-up is roughly 96
extra seconds of powered sensor per day, against a daily budget measured in
single-digit milliamp-hours ([ADR-018](../adr/018-battery-and-deep-sleep-device-mode.md)
§8). Reading too early is the worse failure of the two — a probe that has not
settled returns a *plausible* wrong value, not an obviously bad one — so the
answer is measured, and if it turns out to be uncomfortably long that is a
finding for the budget rather than a reason to shorten the wait.

## Interfaces

```rust
// unchanged from M9
pub trait SoilSensor { fn read(&mut self) -> Result<SoilReading, SensorError>; }

pub struct ModbusSoilSensor<U: Uart> {
    uart: U, slave: u8, map: RegisterMap, calibration: Calibration,
}

pub struct RegisterMap {          // data, not code
    pub moisture: RegisterSpec,   // address, count, scale, offset, signed
    pub temperature: Option<RegisterSpec>,
    pub ec: Option<RegisterSpec>,
}

pub struct AnalogSoilSensor<A: AdcChannel> { adc: A, calibration: Calibration }

pub struct Calibration { pub raw_dry: u16, pub raw_wet: u16, pub temp_coeff: Option<f32> }

pub enum SensorError { Timeout, Crc, ModbusException(u8), OutOfRange(u16),
                       Stuck, NotCalibrated }
```

Device config gains:

```json
"soil_sensor": {
  "kind": "modbus" | "analog",
  "slave_address": 1,
  "register_map": "generic_3in1",
  "calibration": { "raw_dry": 3200, "raw_wet": 1400 }
}
```

## Data model

No measurement-table shape change. M3 already stores narrow typed rows using
`MeasurementKind`, `point`, numeric/boolean value, canonical `unit`, `quality`,
sensor identity, and `calibration_ref`; `point` supports multi-probe use
([ADR-004](../adr/004-sqlite-edge-persistence-model.md)).

Calibration lives in device config, not in the database, because it is a
property of the physical installation and must survive an edge rebuild.

## State model

```text
Sensor: Uninitialised ──► Calibrating ──► Healthy ◄──┐
                                            │        │
                                    3 failures       │ successful read
                                            ▼        │
                                        Unhealthy ───┘

Unhealthy publishes null → edge raises SensorFault → automatic watering locked
```

The device does not decide what an unhealthy sensor *means* — it reports, and
the edge locks out. That division is unchanged from M6.

## Failure modes

| Failure | Behaviour |
|---|---|
| Probe unplugged | Modbus timeout / ADC rail reading → `null` → unhealthy → `SensorFault` lockout |
| RS485 wires swapped | persistent timeouts; documented as the first thing to check |
| Wrong slave address | Modbus exception; distinct counter and log |
| CRC errors from a long cable | counted; a rate above 5 % logs a wiring warning |
| Probe corroded, reading drifts | detected by the M10-010 gravimetric comparison, not automatically |
| Probe out of the soil | reads dry, which is indistinguishable from dry soil — **this is why multi-dose with no-delivery detection exists** (SCEN-044) |
| Temperature sensor absent | field omitted; no compensation applied |
| Uncalibrated | `null` published; lockout rather than a plausible-looking wrong number |

The "probe out of the soil" row is the important one. No sensor design detects
it directly; the mitigation is architectural — bounded doses, absorption
re-checks, no-delivery detection, and the daily cap.

## Safety implications

M10 introduces no new invariant but is where **SAFETY-005 becomes real**. Until
now, "invalid or stale moisture" was injected by a fault flag. From M10 it is
produced by physics: corroded contacts, a broken wire, a probe pulled out by a
cat.

Requirements carrying safety weight:

- **F-100-14** — an uncalibrated sensor publishes `null`, not a raw ADC count
  scaled to look like a percentage. A plausible wrong number is far more
  dangerous than an absent one, because the lockout never fires (SAFETY-012).
- **F-100-21** — a read failure publishes `null`, never the last good value.
  Repeating a stale value would defeat both staleness detection and stuck
  detection.
- **F-100-12** — out-of-calibration-range readings are flagged rather than
  extrapolated.

Reuse the same Edge lockouts, gate and storage. Small integration fixes are
allowed when an actual defect is found; avoid architectural redesign.

## Observability

```text
sensor_errors_total{sensor="soil",reason="timeout|crc|exception|range|stuck"}
sensor_read_duration_seconds{sensor}
```

Status reports per-sensor `present`, `healthy`, and `errors`. Raw readings
alongside converted values are published at DEBUG for calibration work.

Device events: `sensor_invalid`, `sensor_stuck`, `sensor_unhealthy`,
`calibration_missing`.

## Testing strategy

- Host unit: Modbus frame encode/decode including CRC; exception response
  parsing; register map application (scale, offset, signedness); calibration
  linear mapping; clamping and flagging at range edges; stuck detection; ADC
  median filtering.
- Host unit with a fake UART: timeout, CRC error, and exception paths each
  producing the correct `SensorError` and the correct `null` publication.
- Integration: firmware with a mock Modbus responder; assert telemetry shape and
  unhealthy transitions.
- Hardware: HIL-2 extended — probe in air, in water, in damp soil, readings
  plausible and repeatable.
- **Validation (F-100-30):** gravimetric reference at three moisture levels,
  documented error bounds. This is the test that decides whether the numbers
  mean anything.

## Acceptance criteria

- [ ] One real SEN0601 reports useful moisture/temperature/EC through ESP32-C3, MQTT, Edge, SQLite and REST.
- [ ] Sensor selection, address, scaling and calibration provenance are configured; fakes remain for host tests.
- [ ] Readings are plausible and repeatable in the intended soil after basic startup settling; limitations are recorded briefly.
- [ ] Missing, invalid and stale soil readings are visible and block automatic watering, without repeating old data as fresh.
- [ ] A relevant host test and one local end-to-end smoke demonstrate the slice.

The analogue adapter, gravimetric campaign, four-week drift, full stabilization
matrix and energy budget remain deferred in M10-005 and M10-010…012. No claim
of measured accuracy or battery lifetime is made.

## Dependencies

- Existing M9 sensor/config/telemetry interfaces and the already implemented local Edge path.
- The requested SEN0601 and an RS485 interface compatible with ESP32-C3 logic.
- Before power: verify the actual supply, connectors, wiring and isolation from the local inventory. Do not hot-plug the DFR0845; test sensor loss by safe removal of probe power or power-off rewiring.
- No analogue sensor, reference oven, power profiler or cloud service is required for this prototype.

## Open questions

The first model is SEN0601. Verify its actual revision/register specification
when implementing the driver. Analogue support and temperature compensation
remain deferred unless usage establishes a need.

## Future work

- Multi-depth probes on one bus (M14).
- pH sensing (post-V1).
- Automated drift detection against pot weight (M11+).
- Per-soil-mixture calibration profiles (post-V1).
