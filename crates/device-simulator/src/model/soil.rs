//! Soil moisture and temperature.
//!
//! Specified by
//! [simulator-strategy.md](../../../../../docs/testing/simulator-strategy.md)
//! §3. It is a defensible approximation chosen to exercise control logic, not a
//! claim about real soil: nothing in the system's correctness depends on its
//! numerical accuracy.
//!
//! # The two behaviours that matter
//!
//! **Probe overshoot** and the **drainage cap** are here because they punish a
//! naive controller. A controller that doses again immediately after seeing an
//! overshoot-inflated reading, or that believes a dose beyond field capacity
//! raised moisture proportionally, will misbehave — and should, in tests, before
//! it does so with real water.
//!
//! # No clock
//!
//! Every method takes elapsed milliseconds. The model cannot read a clock, so a
//! six-hour drying curve is a single call in a unit test and the whole thing
//! runs identically at any `--time-scale`.

use crate::rng::SplitMix64;

/// Milliseconds in an hour, as a float for the decay arithmetic.
const MS_PER_HOUR: f64 = 3_600_000.0;
/// The virtual day used for the diurnal temperature component.
const MS_PER_DAY: f64 = 86_400_000.0;

/// Tunable soil parameters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SoilParams {
    /// Drying rate constant, per hour, at the reference temperature.
    pub drying_rate_per_hour: f64,
    /// Residual moisture the soil dries towards, never below.
    pub vwc_floor: f64,
    /// Moisture above which water drains away instead of being held.
    pub field_capacity_vwc: f64,
    /// Time constant of the pending-absorption pool.
    pub absorption_tau_ms: u64,
    /// Peak surface over-read, as a fraction of the change being absorbed.
    pub overshoot_fraction: f64,
    /// Time constant of the overshoot decay.
    pub overshoot_tau_ms: u64,
    /// Pot volume in millilitres.
    pub pot_volume_ml: f64,
    /// Fraction of the pot volume that holds water.
    pub soil_factor: f64,
    /// Mean ambient temperature.
    pub temperature_base_c: f64,
    /// Half-range of the diurnal swing.
    pub temperature_amplitude_c: f64,
    /// Gaussian noise sigma on moisture readings, in VWC points.
    pub noise_vwc_sigma: f64,
    /// Gaussian noise sigma on temperature readings.
    pub noise_temperature_sigma: f64,
}

impl SoilParams {
    /// The reference temperature at which drying runs at the base rate.
    pub const REFERENCE_TEMPERATURE_C: f64 = 21.0;
    /// Fractional change in drying rate per degree above the reference.
    pub const TEMPERATURE_COEFFICIENT: f64 = 0.03;
    /// Lower bound on the temperature factor: cold soil still dries a little.
    pub const MIN_TEMPERATURE_FACTOR: f64 = 0.5;
    /// Default absorption time constant (PRD 020 open question 2).
    pub const DEFAULT_ABSORPTION_TAU_MS: u64 = 6 * 60 * 1000;
    /// Default field capacity (PRD 020 open question 2).
    pub const DEFAULT_FIELD_CAPACITY_VWC: f64 = 45.0;
}

impl Default for SoilParams {
    fn default() -> Self {
        Self {
            drying_rate_per_hour: 0.06,
            vwc_floor: 8.0,
            field_capacity_vwc: Self::DEFAULT_FIELD_CAPACITY_VWC,
            absorption_tau_ms: Self::DEFAULT_ABSORPTION_TAU_MS,
            overshoot_fraction: 0.15,
            overshoot_tau_ms: 2 * 60 * 1000,
            pot_volume_ml: 2500.0,
            soil_factor: 1.0,
            temperature_base_c: Self::REFERENCE_TEMPERATURE_C,
            temperature_amplitude_c: 3.0,
            noise_vwc_sigma: 0.3,
            noise_temperature_sigma: 0.1,
        }
    }
}

/// How a delivery was split between the soil and the drain.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Delivery {
    /// Millilitres that entered the pending-absorption pool.
    pub retained_ml: f64,
    /// Millilitres that ran out of the bottom and will never be measured.
    pub drained_ml: f64,
}

/// The soil model.
#[derive(Clone, Debug)]
pub struct SoilModel {
    params: SoilParams,
    /// Moisture the soil actually holds, before any measurement artefact.
    true_vwc: f64,
    /// Moisture delivered but not yet absorbed, in VWC points.
    pending_vwc: f64,
    /// Current surface over-read, in VWC points.
    overshoot_vwc: f64,
    /// Elapsed virtual time, for the diurnal temperature term.
    elapsed_ms: u64,
    /// Cumulative drained volume, exposed for diagnostics.
    drained_ml: f64,
}

impl SoilModel {
    /// Creates a model at an initial moisture.
    #[must_use]
    pub fn new(params: SoilParams, initial_vwc: f64) -> Self {
        Self {
            true_vwc: initial_vwc.clamp(params.vwc_floor, 100.0),
            params,
            pending_vwc: 0.0,
            overshoot_vwc: 0.0,
            elapsed_ms: 0,
            drained_ml: 0.0,
        }
    }

    /// The parameters in force.
    #[must_use]
    pub const fn params(&self) -> &SoilParams {
        &self.params
    }

    /// Moisture the soil actually holds, with no measurement artefact.
    #[must_use]
    pub const fn true_vwc(&self) -> f64 {
        self.true_vwc
    }

    /// Moisture delivered but not yet absorbed.
    #[must_use]
    pub const fn pending_vwc(&self) -> f64 {
        self.pending_vwc
    }

    /// Cumulative volume lost to drainage.
    #[must_use]
    pub const fn drained_ml(&self) -> f64 {
        self.drained_ml
    }

    /// Sets moisture directly, for the control API's `POST /sim/state`.
    ///
    /// Clears the pending pool and the overshoot with it: a test setting a
    /// moisture level means "the soil is at this value", not "it is at this
    /// value plus whatever was in flight".
    pub fn set_vwc(&mut self, vwc: f64) {
        self.true_vwc = vwc.clamp(0.0, 100.0);
        self.pending_vwc = 0.0;
        self.overshoot_vwc = 0.0;
    }

    /// Delivers water to the pot.
    ///
    /// Volume beyond field capacity drains immediately and is never measured.
    /// Splitting it here rather than during absorption keeps the drained water
    /// out of the pot's weight too, which is what really happens.
    pub fn deliver(&mut self, ml: f64) -> Delivery {
        if !ml.is_finite() || ml <= 0.0 {
            return Delivery {
                retained_ml: 0.0,
                drained_ml: 0.0,
            };
        }
        let vwc_per_ml = self.vwc_per_ml();
        let offered_vwc = ml * vwc_per_ml;
        let headroom = (self.params.field_capacity_vwc - self.true_vwc - self.pending_vwc).max(0.0);
        let absorbed_vwc = offered_vwc.min(headroom);

        self.pending_vwc += absorbed_vwc;
        // The overshoot is a *measurement* artefact proportional to the change
        // being absorbed, not extra water.
        self.overshoot_vwc += absorbed_vwc * self.params.overshoot_fraction;

        let retained_ml = if vwc_per_ml > 0.0 {
            absorbed_vwc / vwc_per_ml
        } else {
            0.0
        };
        let drained_ml = ml - retained_ml;
        self.drained_ml += drained_ml;
        Delivery {
            retained_ml,
            drained_ml: drained_ml.max(0.0),
        }
    }

    /// Advances the model by elapsed virtual time.
    pub fn step(&mut self, dt_ms: u64) {
        if dt_ms == 0 {
            return;
        }
        self.elapsed_ms = self.elapsed_ms.saturating_add(dt_ms);
        let dt = dt_ms as f64;

        // Absorption: 63 % of what is pending transfers within `absorption_tau`.
        if self.pending_vwc > 0.0 {
            let transferred =
                self.pending_vwc * transfer_fraction(dt, self.params.absorption_tau_ms);
            self.pending_vwc -= transferred;
            self.true_vwc += transferred;
        }

        // Drying: exponential towards the floor, scaled by temperature. Wet
        // soil loses water faster than dry soil, so a plant approaches the dry
        // threshold gradually rather than falling off a cliff — which is what
        // exercises the Drying -> DryConfirmed debounce.
        let factor = temperature_factor(self.true_temperature_c());
        let k = self.params.drying_rate_per_hour * factor;
        let decay = (-k * dt / MS_PER_HOUR).exp();
        self.true_vwc = self.params.vwc_floor + (self.true_vwc - self.params.vwc_floor) * decay;

        // The overshoot decays away over roughly two minutes.
        if self.overshoot_vwc > 0.0 {
            self.overshoot_vwc *= 1.0 - transfer_fraction(dt, self.params.overshoot_tau_ms);
            if self.overshoot_vwc < 1e-9 {
                self.overshoot_vwc = 0.0;
            }
        }
    }

    /// The temperature the soil actually is, before measurement noise.
    #[must_use]
    pub fn true_temperature_c(&self) -> f64 {
        let phase = (self.elapsed_ms as f64 / MS_PER_DAY) * core::f64::consts::TAU;
        self.params.temperature_base_c + self.params.temperature_amplitude_c * phase.sin()
    }

    /// What a surface probe reports: the true value plus the decaying
    /// overshoot, plus Gaussian noise.
    ///
    /// Draws from the generator, so it advances the deterministic stream and
    /// must be called exactly once per sampling cycle.
    pub fn sample_vwc(&self, rng: &mut SplitMix64, noise: bool) -> f64 {
        let sigma = if noise {
            self.params.noise_vwc_sigma
        } else {
            0.0
        };
        (self.true_vwc + self.overshoot_vwc + rng.gaussian(sigma)).clamp(0.0, 100.0)
    }

    /// What the temperature probe reports.
    pub fn sample_temperature_c(&self, rng: &mut SplitMix64, noise: bool) -> f64 {
        let sigma = if noise {
            self.params.noise_temperature_sigma
        } else {
            0.0
        };
        self.true_temperature_c() + rng.gaussian(sigma)
    }

    /// VWC points added per millilitre delivered.
    fn vwc_per_ml(&self) -> f64 {
        let denominator = self.params.pot_volume_ml * self.params.soil_factor;
        if denominator > 0.0 {
            100.0 / denominator
        } else {
            0.0
        }
    }
}

/// The fraction of a first-order pool that transfers in `dt`.
///
/// `1 - exp(-dt/tau)`, so exactly 63 % has transferred when `dt == tau` — the
/// definition of a time constant, and what the strategy document specifies.
fn transfer_fraction(dt_ms: f64, tau_ms: u64) -> f64 {
    if tau_ms == 0 {
        return 1.0;
    }
    1.0 - (-dt_ms / tau_ms as f64).exp()
}

/// Drying rate multiplier for a temperature.
///
/// 1.0 at 21 °C, +3 % per degree above, floored at 0.5 so cold soil still dries.
#[must_use]
pub fn temperature_factor(celsius: f64) -> f64 {
    if !celsius.is_finite() {
        return 1.0;
    }
    let raw =
        1.0 + SoilParams::TEMPERATURE_COEFFICIENT * (celsius - SoilParams::REFERENCE_TEMPERATURE_C);
    raw.max(SoilParams::MIN_TEMPERATURE_FACTOR)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A model with no diurnal swing, so temperature tests set it explicitly.
    fn still(initial: f64) -> SoilModel {
        SoilModel::new(
            SoilParams {
                temperature_amplitude_c: 0.0,
                ..SoilParams::default()
            },
            initial,
        )
    }

    fn quiet_sample(model: &SoilModel) -> f64 {
        let mut rng = SplitMix64::new(1);
        model.sample_vwc(&mut rng, false)
    }

    #[test]
    fn moisture_decreases_monotonically_with_no_water_added() {
        let mut model = still(42.0);
        let mut last = model.true_vwc();
        for _ in 0..500 {
            model.step(60_000);
            assert!(
                model.true_vwc() < last,
                "{} did not fall below {last}",
                model.true_vwc()
            );
            last = model.true_vwc();
        }
    }

    #[test]
    fn drying_is_faster_at_higher_temperature() {
        let dry_at = |temperature| {
            let mut model = SoilModel::new(
                SoilParams {
                    temperature_amplitude_c: 0.0,
                    temperature_base_c: temperature,
                    ..SoilParams::default()
                },
                42.0,
            );
            for _ in 0..24 {
                model.step(MS_PER_HOUR as u64);
            }
            model.true_vwc()
        };
        assert!(
            dry_at(30.0) < dry_at(21.0),
            "warm soil must dry faster than the reference"
        );
        assert!(
            dry_at(21.0) < dry_at(10.0),
            "cool soil must dry slower than the reference"
        );
    }

    #[test]
    fn the_temperature_factor_matches_the_specified_curve() {
        assert!((temperature_factor(21.0) - 1.0).abs() < 1e-12);
        assert!((temperature_factor(31.0) - 1.3).abs() < 1e-12);
        assert!((temperature_factor(11.0) - 0.7).abs() < 1e-12);
        assert!(
            (temperature_factor(-40.0) - SoilParams::MIN_TEMPERATURE_FACTOR).abs() < 1e-12,
            "the factor is floored, so cold soil still dries"
        );
        assert!((temperature_factor(f64::NAN) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn moisture_never_falls_below_the_floor() {
        let mut model = still(42.0);
        for _ in 0..(24 * 365) {
            model.step(MS_PER_HOUR as u64);
        }
        assert!(model.true_vwc() >= model.params().vwc_floor);
        assert!(
            model.true_vwc() - model.params().vwc_floor < 0.001,
            "a year of drying should approach the floor"
        );
    }

    #[test]
    fn a_dose_raises_moisture_gradually_rather_than_instantly() {
        let mut model = still(20.0);
        let before = model.true_vwc();
        let delivery = model.deliver(80.0);
        assert_eq!(delivery.drained_ml, 0.0);
        assert_eq!(
            model.true_vwc(),
            before,
            "delivered water is pending, not yet measured"
        );
        assert!(model.pending_vwc() > 0.0);

        model.step(1_000);
        assert!(model.true_vwc() > before, "some of it has been absorbed");
        assert!(model.pending_vwc() > 0.0, "and most of it has not");
    }

    #[test]
    fn absorption_reaches_63_percent_within_the_time_constant() {
        let params = SoilParams {
            temperature_amplitude_c: 0.0,
            // Isolate absorption from drying, which otherwise removes a little
            // of what was just absorbed and blurs the measurement.
            drying_rate_per_hour: 0.0,
            ..SoilParams::default()
        };
        let mut model = SoilModel::new(params, 20.0);
        let total = 80.0 * 100.0 / (params.pot_volume_ml * params.soil_factor);
        model.deliver(80.0);
        model.step(params.absorption_tau_ms);
        let absorbed = model.true_vwc() - 20.0;
        assert!(
            (absorbed / total - 0.632).abs() < 0.01,
            "absorbed {absorbed} of {total} after one tau"
        );
    }

    #[test]
    fn the_probe_overshoots_and_the_overshoot_decays() {
        let params = SoilParams {
            temperature_amplitude_c: 0.0,
            drying_rate_per_hour: 0.0,
            ..SoilParams::default()
        };
        let mut model = SoilModel::new(params, 20.0);
        let total = 80.0 * 100.0 / (params.pot_volume_ml * params.soil_factor);
        model.deliver(80.0);
        let peak = quiet_sample(&model) - model.true_vwc();
        assert!(
            (peak - total * params.overshoot_fraction).abs() < 1e-9,
            "the peak over-read is 15 % of the change being absorbed"
        );
        assert!(
            peak <= total * 0.15 + 1e-9,
            "the specification says *up to* 15 %"
        );

        model.step(params.overshoot_tau_ms);
        let after_one_tau = quiet_sample(&model) - model.true_vwc();
        assert!(
            after_one_tau < peak * 0.4,
            "it decays over about two minutes"
        );
        model.step(params.overshoot_tau_ms * 8);
        assert!(
            (quiet_sample(&model) - model.true_vwc()).abs() < 0.001,
            "and is unmeasurable soon after"
        );
    }

    #[test]
    fn water_beyond_field_capacity_drains_and_is_never_measured() {
        let params = SoilParams {
            temperature_amplitude_c: 0.0,
            drying_rate_per_hour: 0.0,
            field_capacity_vwc: 45.0,
            ..SoilParams::default()
        };
        let mut model = SoilModel::new(params, 44.0);
        // One VWC point of headroom = 25 ml at the default pot volume.
        let delivery = model.deliver(80.0);
        assert!((delivery.retained_ml - 25.0).abs() < 1e-9);
        assert!((delivery.drained_ml - 55.0).abs() < 1e-9);

        // Let everything absorb, then confirm the cap really held.
        for _ in 0..100 {
            model.step(60_000);
        }
        assert!(
            model.true_vwc() <= params.field_capacity_vwc + 1e-9,
            "moisture reached {} above a field capacity of {}",
            model.true_vwc(),
            params.field_capacity_vwc
        );
    }

    #[test]
    fn a_dose_into_saturated_soil_raises_nothing_at_all() {
        let params = SoilParams {
            temperature_amplitude_c: 0.0,
            drying_rate_per_hour: 0.0,
            ..SoilParams::default()
        };
        let mut model = SoilModel::new(params, params.field_capacity_vwc);
        let delivery = model.deliver(80.0);
        assert_eq!(delivery.retained_ml, 0.0);
        assert!((delivery.drained_ml - 80.0).abs() < 1e-9);
        for _ in 0..50 {
            model.step(60_000);
        }
        assert!((model.true_vwc() - params.field_capacity_vwc).abs() < 1e-9);
    }

    #[test]
    fn a_nonsense_delivery_changes_nothing() {
        let mut model = still(20.0);
        for ml in [0.0, -5.0, f64::NAN, f64::INFINITY] {
            let delivery = model.deliver(ml);
            assert_eq!(delivery.retained_ml, 0.0);
            assert_eq!(delivery.drained_ml, 0.0);
        }
        assert_eq!(model.true_vwc(), 20.0);
        assert_eq!(model.pending_vwc(), 0.0);
    }

    #[test]
    fn the_model_is_deterministic_for_a_given_dt_sequence() {
        let run = || {
            let mut model = SoilModel::new(SoilParams::default(), 42.0);
            let mut rng = SplitMix64::new(2026);
            let mut readings = Vec::new();
            for i in 0..200 {
                if i == 50 {
                    model.deliver(60.0);
                }
                model.step(30_000);
                readings.push(model.sample_vwc(&mut rng, false).to_bits());
            }
            readings
        };
        assert_eq!(run(), run(), "identical inputs must give identical results");
    }

    #[test]
    fn noise_is_reproducible_from_the_seed_and_absent_when_disabled() {
        let model = still(30.0);
        let with_noise = |seed| {
            let mut rng = SplitMix64::new(seed);
            (0..8)
                .map(|_| model.sample_vwc(&mut rng, true).to_bits())
                .collect::<Vec<_>>()
        };
        assert_eq!(with_noise(7), with_noise(7));
        assert_ne!(with_noise(7), with_noise(8));

        let mut rng = SplitMix64::new(7);
        assert_eq!(model.sample_vwc(&mut rng, false), 30.0);
    }

    #[test]
    fn temperature_drifts_diurnally_around_its_base() {
        let mut model = SoilModel::new(SoilParams::default(), 30.0);
        let mut lowest = f64::MAX;
        let mut highest = f64::MIN;
        for _ in 0..(24 * 4) {
            model.step((MS_PER_HOUR / 4.0) as u64);
            lowest = lowest.min(model.true_temperature_c());
            highest = highest.max(model.true_temperature_c());
        }
        assert!(highest - lowest > 5.0, "a day should show a real swing");
        assert!(lowest > 21.0 - 3.1 && highest < 21.0 + 3.1);
    }

    #[test]
    fn setting_moisture_directly_discards_anything_in_flight() {
        let mut model = still(20.0);
        model.deliver(80.0);
        model.set_vwc(35.0);
        assert_eq!(model.true_vwc(), 35.0);
        assert_eq!(model.pending_vwc(), 0.0);
        assert_eq!(quiet_sample(&model), 35.0, "and the overshoot with it");
    }

    #[test]
    fn a_zero_length_step_is_a_no_op() {
        let mut model = still(30.0);
        model.deliver(40.0);
        let before = (model.true_vwc(), model.pending_vwc());
        model.step(0);
        assert_eq!((model.true_vwc(), model.pending_vwc()), before);
    }
}
