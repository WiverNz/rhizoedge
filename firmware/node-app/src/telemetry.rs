//! Telemetry sampling, the bounded ring, and status (M9-010, M9-020, M9-021).
//!
//! # A read error publishes `null`, not the last good value
//!
//! That is what makes staleness and stuck-value detection work upstream. A
//! device that repeated its last reading would be indistinguishable from a
//! plant that had stopped drying.
//!
//! # Sixteen samples, and then they are dropped
//!
//! A device is not a ledger for samples, and unbounded buffering exhausts a
//! 400 KB heap. Losing a sample is fail-safe: it makes data look older, and
//! stale data blocks watering. `command.result` is the exception and is handled
//! by [`crate::ledger`]; **do not generalise the result-durability machinery to
//! telemetry.**
//!
//! # Battery fields are absent, never zero
//!
//! `battery_voltage` is published where measurable and omitted where not.
//! `battery_percent` only from a configured chemistry curve — a fabricated
//! percentage is worse than an absent one, and for LiFePO4 the discharge curve
//! is famously flat across most of its range. Neither is ever an input to a
//! decision (ADR-018 §7).

use rhizo_mqtt_contract::payload::{
    MeasurementKind, MeasurementPoint, MeasurementSample, MeasurementValue, Quality,
    TelemetryBatch, Unit,
};
use uuid::Uuid;

use crate::ports::{BatterySensor, LeakSensor, Scale, SensorError, SoilSensor, TankSensor};

/// How many samples are buffered across a disconnect.
pub const TELEMETRY_RING: usize = 16;

/// A bounded ring of batches waiting for a connection.
#[derive(Clone, Debug, Default)]
pub struct TelemetryRing {
    batches: Vec<TelemetryBatch>,
    /// How many batches have been dropped because the ring was full.
    pub dropped: u32,
}

impl TelemetryRing {
    /// An empty ring.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// How many batches are held.
    #[must_use]
    pub fn len(&self) -> usize {
        self.batches.len()
    }

    /// Whether nothing is held.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.batches.is_empty()
    }

    /// Buffers a batch, dropping the oldest when full.
    pub fn push(&mut self, batch: TelemetryBatch) {
        self.batches.push(batch);
        while self.batches.len() > TELEMETRY_RING {
            self.batches.remove(0);
            self.dropped = self.dropped.saturating_add(1);
        }
    }

    /// Takes everything buffered, oldest first.
    pub fn drain(&mut self) -> Vec<TelemetryBatch> {
        std::mem::take(&mut self.batches)
    }
}

/// The peripherals a sampling cycle reads.
pub struct Sensors<'a> {
    /// Soil probe, where fitted.
    pub soil: Option<&'a mut dyn SoilSensor>,
    /// Reservoir level, where fitted.
    pub tank: Option<&'a mut dyn TankSensor>,
    /// Leak detector, where fitted.
    pub leak: Option<&'a mut dyn LeakSensor>,
    /// Pot scale, where fitted.
    pub scale: Option<&'a mut dyn Scale>,
    /// Battery divider, where fitted.
    ///
    /// `None` is what "this hardware cannot measure its supply" looks like, and
    /// it produces **no** battery sample rather than a zero one.
    pub battery: Option<&'a mut dyn BatterySensor>,
}

/// How a supply voltage becomes a percentage, when one is configured.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChemistryCurve {
    /// Millivolts at which the pack reads empty.
    pub empty_mv: u32,
    /// Millivolts at which the pack reads full.
    pub full_mv: u32,
}

impl ChemistryCurve {
    /// The percentage this curve reports for a reading.
    #[must_use]
    pub fn percent(&self, millivolts: u32) -> Option<f64> {
        if self.full_mv <= self.empty_mv {
            return None;
        }
        let span = f64::from(self.full_mv - self.empty_mv);
        let above = f64::from(millivolts.saturating_sub(self.empty_mv));
        Some((above / span * 100.0).clamp(0.0, 100.0))
    }
}

/// Builds one telemetry batch from whatever the fitted sensors report.
///
/// A failed read becomes a sample with `value: None` and `quality: Fault`, not
/// an omitted sample and not a stale value.
pub fn sample(
    batch_id: Uuid,
    sensors: &mut Sensors<'_>,
    curve: Option<ChemistryCurve>,
) -> (TelemetryBatch, SensorErrors) {
    let point = MeasurementPoint::parse("default").unwrap_or_else(|_| {
        // The grammar accepts "default"; this branch is unreachable and is
        // written as a fallback rather than an unwrap because the workspace
        // forbids panicking its way out of a classified failure.
        unreachable!("\"default\" is a valid measurement point")
    });
    let mut samples = Vec::new();
    let mut errors = SensorErrors::default();

    if let Some(soil) = sensors.soil.as_mut() {
        match soil.read() {
            Ok(reading) => {
                samples.push(scalar(
                    &point,
                    MeasurementKind::SoilMoisture,
                    Unit::VwcPercent,
                    f64::from(reading.vwc_percent),
                ));
                if let Some(temperature) = reading.temperature_c {
                    samples.push(scalar(
                        &point,
                        MeasurementKind::SoilTemperature,
                        Unit::Celsius,
                        f64::from(temperature),
                    ));
                }
                if let Some(ec) = reading.ec_us_cm {
                    samples.push(scalar(
                        &point,
                        MeasurementKind::SoilEc,
                        Unit::UsCm,
                        f64::from(ec),
                    ));
                }
            }
            Err(error) => {
                errors.record(error);
                samples.push(failed(
                    &point,
                    MeasurementKind::SoilMoisture,
                    Unit::VwcPercent,
                ));
            }
        }
    }

    if let Some(tank) = sensors.tank.as_mut() {
        match tank.read() {
            Ok(reading) => samples.push(scalar(
                &point,
                MeasurementKind::TankLevel,
                Unit::Percent,
                f64::from(reading.level_percent),
            )),
            Err(error) => {
                errors.record(error);
                samples.push(failed(&point, MeasurementKind::TankLevel, Unit::Percent));
            }
        }
    }

    if let Some(leak) = sensors.leak.as_mut() {
        match leak.read() {
            Ok(detected) => samples.push(MeasurementSample {
                point: point.clone(),
                kind: MeasurementKind::LeakState,
                value: Some(MeasurementValue::Boolean(detected)),
                unit: Unit::Boolean,
                quality: Quality::Ok,
                sensor_id: None,
                calibration_ref: None,
            }),
            Err(error) => {
                errors.record(error);
                samples.push(failed(&point, MeasurementKind::LeakState, Unit::Boolean));
            }
        }
    }

    if let Some(scale) = sensors.scale.as_mut() {
        match scale.read() {
            Ok(grams) => samples.push(scalar(
                &point,
                MeasurementKind::PotWeight,
                Unit::Gram,
                f64::from(grams),
            )),
            Err(error) => {
                errors.record(error);
                samples.push(failed(&point, MeasurementKind::PotWeight, Unit::Gram));
            }
        }
    }

    // Absent hardware produces no sample at all. Not zero, not null: the field
    // simply is not there, so nothing downstream can read a supply that was
    // never measured.
    if let Some(battery) = sensors.battery.as_mut() {
        match battery.read_mv() {
            Ok(millivolts) => {
                samples.push(scalar(
                    &point,
                    MeasurementKind::BatteryVoltage,
                    Unit::Volt,
                    f64::from(millivolts) / 1000.0,
                ));
                if let Some(percent) = curve.and_then(|curve| curve.percent(millivolts)) {
                    samples.push(scalar(
                        &point,
                        MeasurementKind::BatteryPercent,
                        Unit::Percent,
                        percent,
                    ));
                }
            }
            Err(error) => {
                errors.record(error);
                samples.push(failed(&point, MeasurementKind::BatteryVoltage, Unit::Volt));
            }
        }
    }

    (TelemetryBatch { batch_id, samples }, errors)
}

fn scalar(
    point: &MeasurementPoint,
    kind: MeasurementKind,
    unit: Unit,
    value: f64,
) -> MeasurementSample {
    MeasurementSample {
        point: point.clone(),
        kind,
        value: Some(MeasurementValue::Scalar(value)),
        unit,
        quality: Quality::Ok,
        sensor_id: None,
        calibration_ref: None,
    }
}

fn failed(point: &MeasurementPoint, kind: MeasurementKind, unit: Unit) -> MeasurementSample {
    MeasurementSample {
        point: point.clone(),
        kind,
        value: None,
        unit,
        quality: Quality::Fault,
        sensor_id: None,
        calibration_ref: None,
    }
}

/// Per-cycle sensor error counters, reported in `device.status`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SensorErrors {
    /// Reads that failed outright.
    pub read_failures: u32,
    /// Reads refused because the rail had not warmed up (M9-020).
    pub warmup_incomplete: u32,
    /// Reads against hardware this board does not have.
    pub not_present: u32,
}

impl SensorErrors {
    fn record(&mut self, error: SensorError) {
        match error {
            SensorError::ReadFailed => self.read_failures += 1,
            SensorError::WarmupIncomplete => self.warmup_incomplete += 1,
            SensorError::NotPresent => self.not_present += 1,
        }
    }

    /// How many errors of any kind occurred.
    #[must_use]
    pub const fn total(&self) -> u32 {
        self.read_failures + self.warmup_incomplete + self.not_present
    }
}

/// How often status is republished while the clock is unsynchronised.
///
/// Protocol §5.12: the retained status is the request for `edge.time`, and this
/// bounds how often the device asks.
pub const UNSYNCED_STATUS_REPUBLISH_MS: u64 = 60_000;

/// How often the heartbeat status is published, in telemetry intervals.
pub const STATUS_HEARTBEAT_INTERVALS: u32 = 5;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fakes::{FakeBattery, FakeLeak, FakeScale, FakeSoil, FakeTank};

    fn kinds(batch: &TelemetryBatch) -> Vec<String> {
        batch
            .samples
            .iter()
            .map(|s| s.kind.as_str().to_owned())
            .collect()
    }

    #[test]
    fn a_full_cycle_produces_a_valid_batch() {
        let mut soil = FakeSoil::reading(32.0);
        let mut tank = FakeTank::at(70.0);
        let mut leak = FakeLeak::clear();
        let mut scale = FakeScale {
            grams: Some(1_200.0),
        };
        let mut sensors = Sensors {
            soil: Some(&mut soil),
            tank: Some(&mut tank),
            leak: Some(&mut leak),
            scale: Some(&mut scale),
            battery: None,
        };
        let (batch, errors) = sample(Uuid::from_u128(1), &mut sensors, None);
        assert!(batch.validate().is_ok());
        assert_eq!(errors.total(), 0);
        assert_eq!(
            kinds(&batch),
            ["soil_moisture", "tank_level", "leak_state", "pot_weight"]
        );
        for s in &batch.samples {
            assert!(s.validate().is_valid(), "{s:?}");
        }
    }

    /// A read error publishes `null`, never the last good value.
    #[test]
    fn a_failed_read_publishes_null_and_counts_an_error() {
        let mut soil = FakeSoil::failing();
        let mut sensors = Sensors {
            soil: Some(&mut soil),
            tank: None,
            leak: None,
            scale: None,
            battery: None,
        };
        let (batch, errors) = sample(Uuid::from_u128(1), &mut sensors, None);
        assert_eq!(errors.read_failures, 1);
        let sample = &batch.samples[0];
        assert_eq!(sample.value, None);
        assert_eq!(sample.quality, Quality::Fault);
        assert!(
            sample.validate().is_valid(),
            "null is valid when quality is fault"
        );
    }

    /// Absent, never zero, never a guess.
    #[test]
    fn battery_fields_are_absent_on_hardware_that_cannot_measure_them() {
        let mut sensors = Sensors {
            soil: None,
            tank: None,
            leak: None,
            scale: None,
            battery: None,
        };
        let (batch, _) = sample(Uuid::from_u128(1), &mut sensors, None);
        assert!(
            !kinds(&batch).iter().any(|k| k.starts_with("battery")),
            "no battery sample at all"
        );
    }

    #[test]
    fn battery_percent_needs_a_configured_chemistry_curve() {
        let mut battery = FakeBattery {
            millivolts: Some(3_400),
        };
        let mut sensors = Sensors {
            soil: None,
            tank: None,
            leak: None,
            scale: None,
            battery: Some(&mut battery),
        };
        let (without, _) = sample(Uuid::from_u128(1), &mut sensors, None);
        assert_eq!(kinds(&without), ["battery_voltage"]);

        let mut battery = FakeBattery {
            millivolts: Some(3_400),
        };
        let mut sensors = Sensors {
            soil: None,
            tank: None,
            leak: None,
            scale: None,
            battery: Some(&mut battery),
        };
        let curve = ChemistryCurve {
            empty_mv: 3_000,
            full_mv: 3_600,
        };
        let (with, _) = sample(Uuid::from_u128(1), &mut sensors, Some(curve));
        assert_eq!(kinds(&with), ["battery_voltage", "battery_percent"]);
    }

    #[test]
    fn the_telemetry_ring_caps_at_sixteen_and_counts_what_it_drops() {
        let mut ring = TelemetryRing::new();
        for n in 0..20u128 {
            ring.push(TelemetryBatch {
                batch_id: Uuid::from_u128(n),
                samples: Vec::new(),
            });
        }
        assert_eq!(ring.len(), TELEMETRY_RING);
        assert_eq!(ring.dropped, 4);
        let drained = ring.drain();
        assert_eq!(drained.len(), TELEMETRY_RING);
        assert_eq!(drained[0].batch_id, Uuid::from_u128(4), "oldest dropped");
        assert!(ring.is_empty());
    }

    #[test]
    fn a_curve_that_cannot_be_evaluated_reports_nothing_rather_than_a_guess() {
        let curve = ChemistryCurve {
            empty_mv: 3_600,
            full_mv: 3_000,
        };
        assert_eq!(curve.percent(3_300), None);
    }
}
