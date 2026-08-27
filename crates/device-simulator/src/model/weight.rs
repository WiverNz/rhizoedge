//! Pot weight.
//!
//! ```text
//! weight_g = dry_weight_g + water_g + noise
//! water_g += delivered_ml            (immediate — unlike VWC)
//! water_g -= evapotranspiration(dt)
//! ```
//!
//! # The divergence is the point
//!
//! Weight responds **immediately** to delivered water while VWC lags behind it.
//! That gap is what makes weight useful for detecting manual watering and for
//! catching a pump that runs without delivering (failure-model §5.1), so the
//! simulator has to reproduce it rather than smoothing it away. A model where
//! both responded identically would make the weight-based no-delivery detection
//! of M6-017 untestable — the test would pass against a device that cannot
//! exhibit the failure.

use crate::rng::SplitMix64;

/// One millilitre of water weighs one gram, near enough for a pot.
const GRAMS_PER_ML: f64 = 1.0;
/// Milliseconds in an hour.
const MS_PER_HOUR: f64 = 3_600_000.0;

/// Tunable weight parameters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WeightParams {
    /// Mass of pot and dry soil.
    pub dry_weight_g: f64,
    /// Water lost per hour to evaporation and the plant.
    pub evapotranspiration_g_per_hour: f64,
    /// Gaussian noise sigma on the reading, in grams.
    pub noise_sigma_g: f64,
    /// How long a reading is unstable after the mass changes.
    pub settle_ms: u64,
}

impl Default for WeightParams {
    fn default() -> Self {
        Self {
            dry_weight_g: 1_800.0,
            evapotranspiration_g_per_hour: 6.0,
            noise_sigma_g: 2.0,
            settle_ms: 30_000,
        }
    }
}

/// The pot on its scale.
#[derive(Clone, Debug)]
pub struct WeightModel {
    params: WeightParams,
    water_g: f64,
    /// Remaining settling time after the last change.
    unsettled_ms: u64,
    /// Tare offset applied by `command.tare`.
    tare_offset_g: f64,
}

impl WeightModel {
    /// Creates a pot holding the water implied by an initial moisture level.
    #[must_use]
    pub fn new(params: WeightParams, initial_water_g: f64) -> Self {
        Self {
            params,
            water_g: initial_water_g.max(0.0),
            unsettled_ms: 0,
            tare_offset_g: 0.0,
        }
    }

    /// The parameters in force.
    #[must_use]
    pub const fn params(&self) -> &WeightParams {
        &self.params
    }

    /// Water mass currently in the pot.
    #[must_use]
    pub const fn water_g(&self) -> f64 {
        self.water_g
    }

    /// Adds delivered water **immediately**, and unsettles the scale.
    pub fn deliver(&mut self, ml: f64) {
        if !ml.is_finite() || ml <= 0.0 {
            return;
        }
        self.water_g += ml * GRAMS_PER_ML;
        self.unsettled_ms = self.params.settle_ms;
    }

    /// Advances evapotranspiration and settling.
    pub fn step(&mut self, dt_ms: u64) {
        if dt_ms == 0 {
            return;
        }
        let lost = self.params.evapotranspiration_g_per_hour * (dt_ms as f64) / MS_PER_HOUR;
        self.water_g = (self.water_g - lost).max(0.0);
        self.unsettled_ms = self.unsettled_ms.saturating_sub(dt_ms);
    }

    /// Whether the reading has settled since the last change.
    ///
    /// A load cell rings for a while after the mass on it changes. Reporting an
    /// unstable reading as stable would let a controller act on a transient.
    #[must_use]
    pub const fn is_stable(&self) -> bool {
        self.unsettled_ms == 0
    }

    /// Zeroes the scale, as `command.tare` does.
    pub fn tare(&mut self) {
        self.tare_offset_g = self.params.dry_weight_g + self.water_g;
        self.unsettled_ms = self.params.settle_ms;
    }

    /// Sets the water mass directly, for the control API.
    pub fn set_water_g(&mut self, grams: f64) {
        self.water_g = grams.max(0.0);
        self.unsettled_ms = self.params.settle_ms;
    }

    /// What the scale reports.
    pub fn sample_g(&self, rng: &mut SplitMix64, noise: bool) -> f64 {
        let sigma = if noise {
            self.params.noise_sigma_g
        } else {
            0.0
        };
        (self.params.dry_weight_g + self.water_g - self.tare_offset_g + rng.gaussian(sigma))
            .max(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{SoilModel, SoilParams};

    fn quiet(model: &WeightModel) -> f64 {
        let mut rng = SplitMix64::new(1);
        model.sample_g(&mut rng, false)
    }

    /// The key assertion of M2-005.
    #[test]
    fn weight_rises_immediately_while_moisture_still_lags() {
        let params = SoilParams {
            temperature_amplitude_c: 0.0,
            drying_rate_per_hour: 0.0,
            ..SoilParams::default()
        };
        let mut soil = SoilModel::new(params, 20.0);
        let mut scale = WeightModel::new(WeightParams::default(), 400.0);

        let vwc_before = soil.true_vwc();
        let weight_before = quiet(&scale);

        let delivery = soil.deliver(80.0);
        scale.deliver(delivery.retained_ml);

        assert_eq!(
            soil.true_vwc(),
            vwc_before,
            "moisture has not moved at all yet"
        );
        assert!(
            (quiet(&scale) - weight_before - 80.0).abs() < 1e-9,
            "but the scale already reads 80 g heavier"
        );

        // A full absorption time constant later, moisture has caught up only
        // part of the way — the divergence is sustained, not instantaneous.
        soil.step(params.absorption_tau_ms);
        scale.step(params.absorption_tau_ms);
        assert!(soil.pending_vwc() > 0.0, "absorption is still in flight");
    }

    #[test]
    fn weight_decreases_over_time_through_evapotranspiration() {
        let mut scale = WeightModel::new(WeightParams::default(), 400.0);
        let before = scale.water_g();
        scale.step(MS_PER_HOUR as u64);
        assert!((before - scale.water_g() - 6.0).abs() < 1e-9);
    }

    #[test]
    fn water_mass_never_goes_negative() {
        let mut scale = WeightModel::new(WeightParams::default(), 1.0);
        for _ in 0..1_000 {
            scale.step(MS_PER_HOUR as u64);
        }
        assert_eq!(scale.water_g(), 0.0);
        assert!(quiet(&scale) >= 0.0);
    }

    #[test]
    fn the_stable_flag_is_false_briefly_after_a_change() {
        let mut scale = WeightModel::new(WeightParams::default(), 400.0);
        assert!(scale.is_stable());
        scale.deliver(40.0);
        assert!(
            !scale.is_stable(),
            "a load cell rings after the mass changes"
        );
        scale.step(scale.params().settle_ms - 1);
        assert!(!scale.is_stable());
        scale.step(1);
        assert!(scale.is_stable());
    }

    #[test]
    fn taring_zeroes_the_scale_without_removing_water() {
        let mut scale = WeightModel::new(WeightParams::default(), 400.0);
        scale.tare();
        assert!(quiet(&scale).abs() < 1e-9);
        assert_eq!(scale.water_g(), 400.0, "the water is still in the pot");
        scale.deliver(50.0);
        assert!((quiet(&scale) - 50.0).abs() < 1e-9);
    }

    #[test]
    fn a_nonsense_delivery_changes_nothing() {
        let mut scale = WeightModel::new(WeightParams::default(), 400.0);
        for ml in [0.0, -1.0, f64::NAN] {
            scale.deliver(ml);
        }
        assert_eq!(scale.water_g(), 400.0);
        assert!(scale.is_stable());
    }

    #[test]
    fn readings_are_deterministic_from_the_seed() {
        let scale = WeightModel::new(WeightParams::default(), 400.0);
        let draw = |seed| {
            let mut rng = SplitMix64::new(seed);
            (0..8)
                .map(|_| scale.sample_g(&mut rng, true).to_bits())
                .collect::<Vec<_>>()
        };
        assert_eq!(draw(3), draw(3));
        assert_ne!(draw(3), draw(4));
    }
}
