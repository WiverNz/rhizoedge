//! Soil electrical conductivity.
//!
//! ```text
//! ec_us_cm = base_ec * (reference_vwc / current_vwc) ± noise
//!            + fertilisation events (step increase, then slow decay)
//! ```
//!
//! EC rising as soil dries is the real relationship — the dissolved salts stay
//! put while the water leaves, so concentration goes up — and reproducing it is
//! what keeps the EC trend logic honest. A model where EC ignored moisture would
//! let a controller "detect" fertiliser in what is really just a dry pot.
//!
//! There is deliberately **no nitrogen, phosphorus, or potassium here**, for the
//! same reason protocol §5.1 has no kind for them: cheap probes derive those
//! from EC by an undisclosed formula, and publishing them would be a false claim
//! about a real plant. EC is EC.

use crate::rng::SplitMix64;

/// Milliseconds in an hour.
const MS_PER_HOUR: f64 = 3_600_000.0;

/// Tunable conductivity parameters.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EcParams {
    /// Conductivity at the reference moisture, with no fertilisation.
    pub base_us_cm: f64,
    /// The moisture level at which conductivity equals `base_us_cm`.
    pub reference_vwc: f64,
    /// Fraction of a fertilisation bump that decays per hour.
    pub fertilisation_decay_per_hour: f64,
    /// Gaussian noise sigma on the reading.
    pub noise_sigma_us_cm: f64,
    /// Whether the probe is factory-calibrated.
    ///
    /// `false` is the honest default for a cheap probe, and it makes the device
    /// publish `quality: "uncalibrated"` — which the edge stores but must not
    /// use for control (SAFETY-017).
    pub calibrated: bool,
}

impl Default for EcParams {
    fn default() -> Self {
        Self {
            base_us_cm: 1_200.0,
            reference_vwc: 35.0,
            fertilisation_decay_per_hour: 0.05,
            noise_sigma_us_cm: 15.0,
            calibrated: false,
        }
    }
}

/// The conductivity probe.
#[derive(Clone, Debug)]
pub struct EcModel {
    params: EcParams,
    /// Conductivity added by fertilisation, decaying over time.
    fertilisation_us_cm: f64,
}

impl EcModel {
    /// Creates a probe with no fertilisation applied.
    #[must_use]
    pub const fn new(params: EcParams) -> Self {
        Self {
            params,
            fertilisation_us_cm: 0.0,
        }
    }

    /// The parameters in force.
    #[must_use]
    pub const fn params(&self) -> &EcParams {
        &self.params
    }

    /// The current fertilisation contribution.
    #[must_use]
    pub const fn fertilisation_us_cm(&self) -> f64 {
        self.fertilisation_us_cm
    }

    /// Applies a fertilisation event: a step up, which then decays.
    pub fn fertilise(&mut self, us_cm: f64) {
        if us_cm.is_finite() && us_cm > 0.0 {
            self.fertilisation_us_cm += us_cm;
        }
    }

    /// Decays the fertilisation contribution.
    pub fn step(&mut self, dt_ms: u64) {
        if dt_ms == 0 || self.fertilisation_us_cm <= 0.0 {
            return;
        }
        let hours = dt_ms as f64 / MS_PER_HOUR;
        self.fertilisation_us_cm *= (-self.params.fertilisation_decay_per_hour * hours).exp();
        if self.fertilisation_us_cm < 1e-6 {
            self.fertilisation_us_cm = 0.0;
        }
    }

    /// The true conductivity at a given moisture, before noise.
    #[must_use]
    pub fn true_us_cm(&self, vwc: f64) -> f64 {
        // A pot at zero moisture has no continuous water path, so the ratio is
        // meaningless there; clamping the divisor keeps the value inside the
        // kind's declared range instead of producing an infinity, which §4
        // forbids emitting.
        let vwc = vwc.max(1.0);
        let concentration = self.params.reference_vwc / vwc;
        (self.params.base_us_cm * concentration + self.fertilisation_us_cm).clamp(0.0, 20_000.0)
    }

    /// What the probe reports.
    pub fn sample_us_cm(&self, vwc: f64, rng: &mut SplitMix64, noise: bool) -> f64 {
        let sigma = if noise {
            self.params.noise_sigma_us_cm
        } else {
            0.0
        };
        (self.true_us_cm(vwc) + rng.gaussian(sigma)).clamp(0.0, 20_000.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conductivity_rises_as_moisture_falls() {
        let ec = EcModel::new(EcParams::default());
        let wet = ec.true_us_cm(45.0);
        let reference = ec.true_us_cm(35.0);
        let dry = ec.true_us_cm(15.0);
        assert!(wet < reference, "{wet} should be below {reference}");
        assert!(reference < dry, "{reference} should be below {dry}");
        assert!(
            (reference - ec.params().base_us_cm).abs() < 1e-9,
            "the base value is by definition the reading at the reference moisture"
        );
    }

    #[test]
    fn a_fertilisation_event_steps_conductivity_up_and_then_decays() {
        let mut ec = EcModel::new(EcParams::default());
        let before = ec.true_us_cm(35.0);
        ec.fertilise(400.0);
        let after = ec.true_us_cm(35.0);
        assert!((after - before - 400.0).abs() < 1e-9, "a step, not a ramp");

        ec.step(MS_PER_HOUR as u64);
        let an_hour_later = ec.true_us_cm(35.0);
        assert!(an_hour_later < after, "and it decays");
        assert!(
            an_hour_later > before,
            "slowly — an hour must not erase it entirely"
        );

        for _ in 0..(24 * 30) {
            ec.step(MS_PER_HOUR as u64);
        }
        assert!(
            (ec.true_us_cm(35.0) - before).abs() < 1e-3,
            "gone after a month"
        );
    }

    #[test]
    fn a_nonsense_fertilisation_changes_nothing() {
        let mut ec = EcModel::new(EcParams::default());
        for us_cm in [0.0, -100.0, f64::NAN] {
            ec.fertilise(us_cm);
        }
        assert_eq!(ec.fertilisation_us_cm(), 0.0);
    }

    #[test]
    fn a_reading_stays_inside_the_kinds_declared_range() {
        let mut ec = EcModel::new(EcParams::default());
        ec.fertilise(50_000.0);
        let mut rng = SplitMix64::new(5);
        for vwc in [0.0, 0.5, 1.0, 35.0, 100.0] {
            let reading = ec.sample_us_cm(vwc, &mut rng, true);
            assert!(
                (0.0..=20_000.0).contains(&reading),
                "{reading} at vwc {vwc} is outside the soil_ec range"
            );
            assert!(reading.is_finite(), "protocol §4 forbids emitting infinity");
        }
    }

    #[test]
    fn a_cheap_probe_admits_it_is_uncalibrated_by_default() {
        assert!(
            !EcParams::default().calibrated,
            "quality: uncalibrated is the honest default"
        );
    }
}
