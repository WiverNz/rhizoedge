//! Electrical conductivity: recorded, trended, and warned about (F-050-18).
//!
//! # The non-goal is the important line
//!
//! **No nutrient value is ever derived from EC.** Cheap "NPK" probes compute
//! their outputs from a conductivity reading by an undisclosed formula;
//! presenting those as nutrient measurements would be a false claim. Rhizo
//! records the reading, trends it with the same robustness rules as moisture,
//! warns on an abnormal rise, and claims nothing further
//! ([PRD 100](../../../docs/prd/100-calibration-and-accuracy.md),
//! [PRD 140](../../../docs/prd/140-field-readiness.md)).
//!
//! # EC is a warning, never a lockout
//!
//! High salinity is a horticultural problem for a human to solve. It is not a
//! reason to refuse water — if anything it is a reason to flush the pot. Nothing
//! in this module produces a [`crate::state::LockoutReason`], and
//! [`crate::recommend::RecommendationInputs`] has no EC field at all, which is
//! the structural form of the same promise.
use chrono::Duration;

use crate::trend::{Trend, TrendSample, fit};

/// The default high-EC warning threshold, microsiemens per centimetre.
///
/// A starting point for a soil solution, not a measured constant. PRD 100 tunes
/// it; until then it is deliberately generous, because a warning nobody trusts
/// is worse than no warning.
pub const DEFAULT_WARNING_HIGH_US_CM: f64 = 3_000.0;

/// An EC slope, microsiemens per centimetre per hour.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrendUsCmPerHour(pub f64);

/// The EC trend, using exactly the moisture trend's robustness rules.
///
/// Sharing [`fit`] rather than reimplementing it means the `None`-on-sparse-data
/// guarantee cannot hold for one measurement and quietly not for the other.
#[must_use]
pub fn ec_trend(samples: &[TrendSample], window: Duration) -> Option<TrendUsCmPerHour> {
    fit(samples, window).map(|Trend { per_hour, .. }| TrendUsCmPerHour(per_hour))
}

/// A high-EC warning. Advisory: it raises an event and nothing else.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EcWarning {
    /// The reading that crossed.
    pub us_cm: f64,
    /// The configured threshold.
    pub warning_high_us_cm: f64,
}

/// Whether the latest reading warrants a warning event.
///
/// `None` for an absent, non-finite, or below-threshold reading. An absent
/// reading is not a warning — silence is not evidence of salt.
#[must_use]
pub fn ec_warning(latest_us_cm: Option<f64>, warning_high_us_cm: f64) -> Option<EcWarning> {
    let us_cm = latest_us_cm.filter(|v| v.is_finite())?;
    if !warning_high_us_cm.is_finite() || us_cm <= warning_high_us_cm {
        return None;
    }
    Some(EcWarning {
        us_cm,
        warning_high_us_cm,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, TimeZone, Utc};

    fn base() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap()
    }

    #[test]
    fn the_trend_uses_the_moisture_robustness_rules() {
        let rising: Vec<TrendSample> = (0..13)
            .map(|i| TrendSample {
                value: 900.0 + f64::from(i) * 25.0,
                at: base() + Duration::minutes(i64::from(i) * 30),
                valid: true,
            })
            .collect();
        let TrendUsCmPerHour(slope) = ec_trend(&rising, Duration::hours(6)).unwrap();
        assert!((slope - 50.0).abs() < 1e-9, "{slope}");
        // Fewer than five valid samples is `None`, exactly as for moisture.
        assert_eq!(ec_trend(&rising[..4], Duration::hours(6)), None);
    }

    #[test]
    fn a_reading_above_the_threshold_warrants_a_warning() {
        assert_eq!(
            ec_warning(Some(3_200.0), DEFAULT_WARNING_HIGH_US_CM),
            Some(EcWarning {
                us_cm: 3_200.0,
                warning_high_us_cm: DEFAULT_WARNING_HIGH_US_CM
            })
        );
        // The threshold itself is not a crossing.
        assert_eq!(
            ec_warning(Some(DEFAULT_WARNING_HIGH_US_CM), DEFAULT_WARNING_HIGH_US_CM),
            None
        );
        assert_eq!(ec_warning(None, DEFAULT_WARNING_HIGH_US_CM), None);
        assert_eq!(ec_warning(Some(f64::NAN), DEFAULT_WARNING_HIGH_US_CM), None);
        assert_eq!(ec_warning(Some(9_000.0), f64::NAN), None);
    }

    /// The explicit negative control: EC cannot reach a watering decision,
    /// because the recommendation engine has no way to be told about it.
    ///
    /// A structural check rather than a behavioural one — a behavioural test
    /// could only demonstrate that EC does not *currently* matter, whereas this
    /// fails the moment somebody wires the two modules together.
    #[test]
    fn ec_never_reaches_the_gate() {
        let engine = include_str!("recommend.rs");
        for forbidden in [
            "crate::ec",
            "EcWarning",
            "us_cm",
            "TrendUsCmPerHour",
            "SoilEc",
        ] {
            assert!(
                !engine.contains(forbidden),
                "recommend.rs mentions {forbidden}: EC is a warning, never a lockout"
            );
        }
        let lockouts = include_str!("state.rs");
        assert!(
            !lockouts.contains("Conductivity") && !lockouts.contains("Salinity"),
            "no lockout reason may be about conductivity"
        );
    }
}
