//! A plausible battery, not a prediction (M5-021).
//!
//! The model exists to produce a telemetry series a test can steer, and to make
//! `battery_voltage` and `battery_percent` come from somewhere rather than being
//! invented at publication time. It is **not** an energy budget: real current
//! draw is measured on hardware in M10-012, and any number here would be a guess
//! dressed as a measurement.
//!
//! Power is telemetry. Nothing in this file, and nothing that reads it, may
//! reach an irrigation decision
//! ([ADR-018](../../../../docs/adr/018-battery-and-deep-sleep-device-mode.md) §7).
use crate::rng::SplitMix64;

/// Battery model parameters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BatteryParams {
    /// Terminal voltage at a full charge.
    pub full_volts: f64,
    /// Terminal voltage at an empty pack.
    pub empty_volts: f64,
    /// Charge spent by one complete wake cycle, in percentage points.
    pub drain_per_wake_percent: f64,
    /// Charge spent per second of pump run, in percentage points. Actuation is
    /// the expensive thing a plant node does.
    pub drain_per_pump_second_percent: f64,
    /// Measurement noise on the voltage reading, in volts.
    pub voltage_noise_volts: f64,
}

impl Default for BatteryParams {
    fn default() -> Self {
        Self {
            // A single 18650 cell: the pack ADR-018's deployment assumes.
            full_volts: 4.2,
            empty_volts: 3.0,
            drain_per_wake_percent: 0.05,
            drain_per_pump_second_percent: 0.20,
            voltage_noise_volts: 0.01,
        }
    }
}

/// The simulated pack.
#[derive(Clone, Debug, PartialEq)]
pub struct BatteryModel {
    params: BatteryParams,
    percent: f64,
}

impl BatteryModel {
    /// A pack at the given state of charge.
    #[must_use]
    pub fn new(params: BatteryParams, percent: f64) -> Self {
        Self {
            params,
            percent: percent.clamp(0.0, 100.0),
        }
    }

    /// The true state of charge, before measurement noise.
    #[must_use]
    pub const fn true_percent(&self) -> f64 {
        self.percent
    }

    /// Sets the level directly, for a test that needs a low battery now.
    pub fn set_percent(&mut self, percent: f64) {
        if percent.is_finite() {
            self.percent = percent.clamp(0.0, 100.0);
        }
    }

    /// Spends the charge of one complete wake cycle.
    pub fn drain_wake(&mut self) {
        self.percent = (self.percent - self.params.drain_per_wake_percent).clamp(0.0, 100.0);
    }

    /// Spends the charge of a pump run.
    pub fn drain_pump(&mut self, seconds: f64) {
        if !seconds.is_finite() || seconds <= 0.0 {
            return;
        }
        self.percent =
            (self.percent - seconds * self.params.drain_per_pump_second_percent).clamp(0.0, 100.0);
    }

    /// The reported state of charge.
    #[must_use]
    pub fn sample_percent(&self) -> f64 {
        self.percent
    }

    /// The reported terminal voltage in millivolts, as the status block carries
    /// it. Noise-free: a diagnostic field is not a measurement series.
    #[must_use]
    pub fn sample_millivolts(&self) -> u32 {
        let span = self.params.full_volts - self.params.empty_volts;
        let volts = self.params.empty_volts + span * (self.percent / 100.0);
        (volts * 1_000.0).clamp(0.0, 30_000.0) as u32
    }

    /// The reported terminal voltage.
    ///
    /// Linear between empty and full — a real discharge curve is not, and
    /// pretending otherwise here would be a claim about chemistry the model has
    /// no business making.
    #[must_use]
    pub fn sample_volts(&self, rng: &mut SplitMix64, noise: bool) -> f64 {
        let span = self.params.full_volts - self.params.empty_volts;
        let volts = self.params.empty_volts + span * (self.percent / 100.0);
        let measured = if noise {
            volts + rng.gaussian(self.params.voltage_noise_volts)
        } else {
            volts
        };
        measured.clamp(0.0, 30.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn charge_falls_with_wakes_and_faster_with_the_pump() {
        let mut battery = BatteryModel::new(BatteryParams::default(), 100.0);
        for _ in 0..10 {
            battery.drain_wake();
        }
        let after_wakes = battery.true_percent();
        assert!(after_wakes < 100.0 && after_wakes > 99.0);
        battery.drain_pump(10.0);
        assert!(
            battery.true_percent() < after_wakes - 1.0,
            "actuation is the expensive thing a plant node does"
        );
    }

    #[test]
    fn the_level_is_bounded_and_steerable() {
        let mut battery = BatteryModel::new(BatteryParams::default(), 100.0);
        battery.set_percent(12.5);
        assert_eq!(battery.true_percent(), 12.5);
        battery.set_percent(-5.0);
        assert_eq!(battery.true_percent(), 0.0);
        battery.set_percent(f64::NAN);
        assert_eq!(battery.true_percent(), 0.0, "a NaN steers nothing");
        battery.drain_pump(1_000.0);
        assert_eq!(battery.true_percent(), 0.0, "the pack cannot go negative");
    }

    #[test]
    fn voltage_tracks_charge_and_stays_in_range() {
        let mut rng = SplitMix64::new(1);
        let full = BatteryModel::new(BatteryParams::default(), 100.0).sample_volts(&mut rng, false);
        let empty = BatteryModel::new(BatteryParams::default(), 0.0).sample_volts(&mut rng, false);
        assert!((full - 4.2).abs() < 1e-9);
        assert!((empty - 3.0).abs() < 1e-9);
        // Noise moves the reading without leaving the contract's valid range.
        for _ in 0..200 {
            let v = BatteryModel::new(BatteryParams::default(), 50.0).sample_volts(&mut rng, true);
            assert!((0.0..=30.0).contains(&v), "{v}");
        }
    }
}
