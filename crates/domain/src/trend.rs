//! Robust least-squares trends over a time window.
//!
//! PRD 050 F-050-10 and F-050-11. The requirement that shapes every line here
//! is the second one: **a trend is `None` rather than a fabricated slope**. A
//! confident wrong trend is worse than an absent one, because the operator acts
//! on it (SAFETY-012's reasoning applied to an advisory signal).
//!
//! Two independent reasons to answer `None`, and both matter:
//!
//! - **Too few samples.** Fewer than [`MIN_SAMPLES`] valid readings describe
//!   noise, not a direction.
//! - **Too sparse.** Five samples clustered in the last two minutes of a
//!   six-hour window describe two minutes. Coverage is checked separately from
//!   count, because a count alone cannot tell the two apart.
//!
//! The fit is least-squares rather than an endpoint difference: noise is real
//! (the simulator adds it by default), and two endpoints are exactly the two
//! readings most able to mislead.
use chrono::{DateTime, Duration, Utc};

use crate::profile::SoilSample;

/// The minimum number of valid samples a trend may be computed from.
pub const MIN_SAMPLES: usize = 5;

/// The fraction of the window the samples must actually span.
///
/// Half the window is the smallest span for which "over the last six hours" is
/// an honest description of a six-hour window.
pub const MIN_COVERAGE_FRACTION: f64 = 0.5;

/// The default trend window (PRD 050 F-050-10).
#[must_use]
pub fn default_window() -> Duration {
    Duration::hours(6)
}

/// One timestamped reading offered to a fit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrendSample {
    /// The reading in its own units.
    pub value: f64,
    /// **Edge** receipt time. Never a device timestamp (SAFETY-005).
    pub at: DateTime<Utc>,
    /// Whether the reading passed validation. Invalid readings are excluded
    /// before fitting rather than fitted and hoped about.
    pub valid: bool,
}

/// A fitted slope, in units of the measurement per hour.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Trend {
    /// Slope, measurement units per hour. Negative means falling.
    pub per_hour: f64,
    /// How many valid samples the fit used.
    pub sample_count: usize,
    /// The fraction of the window the used samples spanned, 0.0..=1.0.
    pub coverage: f64,
}

/// A moisture slope in %VWC per hour.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrendVwcPerHour(pub f64);

/// Fits a slope over `window`, or answers `None`.
///
/// Pure: it reads no clock. The window is anchored on the newest sample offered,
/// so the caller decides what "now" means and the function stays deterministic.
///
/// Returns `None` when fewer than [`MIN_SAMPLES`] valid samples fall inside the
/// window, when they span less than [`MIN_COVERAGE_FRACTION`] of it, or when
/// every sample shares one instant (a vertical fit has no slope).
#[must_use]
pub fn fit(samples: &[TrendSample], window: Duration) -> Option<Trend> {
    if window <= Duration::zero() {
        return None;
    }
    let anchor = samples
        .iter()
        .filter(|s| s.valid && s.value.is_finite())
        .map(|s| s.at)
        .max()?;
    let start = anchor.checked_sub_signed(window)?;
    let used: Vec<&TrendSample> = samples
        .iter()
        .filter(|s| s.valid && s.value.is_finite() && s.at >= start && s.at <= anchor)
        .collect();
    if used.len() < MIN_SAMPLES {
        return None;
    }
    let first = used.iter().map(|s| s.at).min()?;
    let last = used.iter().map(|s| s.at).max()?;
    let span_ms = last.signed_duration_since(first).num_milliseconds() as f64;
    let window_ms = window.num_milliseconds() as f64;
    let coverage = if window_ms > 0.0 {
        span_ms / window_ms
    } else {
        0.0
    };
    if coverage < MIN_COVERAGE_FRACTION {
        return None;
    }
    // Least squares on (hours since `first`, value). Referencing hours to the
    // first sample rather than to the epoch keeps the sums small enough that
    // f64 cancellation is not a factor.
    let n = used.len() as f64;
    let xs: Vec<f64> = used
        .iter()
        .map(|s| s.at.signed_duration_since(first).num_milliseconds() as f64 / 3_600_000.0)
        .collect();
    let mean_x = xs.iter().sum::<f64>() / n;
    let mean_y = used.iter().map(|s| s.value).sum::<f64>() / n;
    let mut sxx = 0.0;
    let mut sxy = 0.0;
    for (x, s) in xs.iter().zip(&used) {
        let dx = x - mean_x;
        sxx += dx * dx;
        sxy += dx * (s.value - mean_y);
    }
    if sxx <= 0.0 {
        return None;
    }
    let per_hour = sxy / sxx;
    if !per_hour.is_finite() {
        return None;
    }
    Some(Trend {
        per_hour,
        sample_count: used.len(),
        coverage,
    })
}

/// The moisture trend, in %VWC per hour (PRD 050 §Interfaces).
#[must_use]
pub fn moisture_trend(samples: &[SoilSample], window: Duration) -> Option<TrendVwcPerHour> {
    let converted: Vec<TrendSample> = samples
        .iter()
        .map(|s| TrendSample {
            value: s.moisture_vwc.unwrap_or(f64::NAN),
            at: s.received_at,
            valid: s.is_valid(),
        })
        .collect();
    fit(&converted, window).map(|t| TrendVwcPerHour(t.per_hour))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn base() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap()
    }

    /// A series falling one point per hour over six hours.
    fn falling(count: i64, step_minutes: i64) -> Vec<TrendSample> {
        (0..count)
            .map(|i| TrendSample {
                value: 40.0 - i as f64 * (step_minutes as f64 / 60.0),
                at: base() + Duration::minutes(i * step_minutes),
                valid: true,
            })
            .collect()
    }

    #[test]
    fn a_known_falling_series_produces_the_expected_slope() {
        let t = fit(&falling(13, 30), default_window()).unwrap();
        assert!(
            (t.per_hour + 1.0).abs() < 1e-9,
            "expected -1 %VWC/h, got {}",
            t.per_hour
        );
        assert_eq!(t.sample_count, 13);
        assert!(t.coverage > 0.99);
    }

    #[test]
    fn fewer_than_five_valid_samples_is_none() {
        assert_eq!(fit(&falling(4, 90), default_window()), None);
        assert!(fit(&falling(5, 90), default_window()).is_some());
    }

    /// The sparsity rule. Five samples two minutes apart describe two minutes,
    /// however confidently the arithmetic would answer.
    #[test]
    fn five_clustered_samples_are_refused_for_sparsity() {
        let clustered: Vec<TrendSample> = (0..5)
            .map(|i| TrendSample {
                value: 40.0 - f64::from(i),
                at: base() + Duration::minutes(i64::from(i) / 2),
                valid: true,
            })
            .collect();
        assert_eq!(fit(&clustered, default_window()), None);
    }

    #[test]
    fn invalid_and_non_finite_samples_are_excluded_before_fitting() {
        let mut samples = falling(13, 30);
        // Four readings that would wreck the fit if they reached it.
        samples[2].valid = false;
        samples[2].value = 1e9;
        samples[5].value = f64::NAN;
        samples[7].valid = false;
        samples[9].value = f64::INFINITY;
        let t = fit(&samples, default_window()).unwrap();
        assert_eq!(t.sample_count, 9);
        assert!((t.per_hour + 1.0).abs() < 1e-9, "{}", t.per_hour);
    }

    #[test]
    fn samples_outside_the_window_are_ignored() {
        let mut samples = falling(13, 30);
        samples.insert(
            0,
            TrendSample {
                value: 90.0,
                at: base() - Duration::hours(12),
                valid: true,
            },
        );
        let t = fit(&samples, default_window()).unwrap();
        assert_eq!(t.sample_count, 13);
        assert!((t.per_hour + 1.0).abs() < 1e-9);
    }

    #[test]
    fn one_instant_has_no_slope() {
        let same: Vec<TrendSample> = (0..8)
            .map(|i| TrendSample {
                value: f64::from(i),
                at: base(),
                valid: true,
            })
            .collect();
        assert_eq!(fit(&same, default_window()), None);
        assert_eq!(fit(&falling(13, 30), Duration::zero()), None);
        assert_eq!(fit(&[], default_window()), None);
    }

    #[test]
    fn the_moisture_wrapper_reads_the_edge_receipt_time() {
        let samples: Vec<SoilSample> = (0..13)
            .map(|i| SoilSample {
                moisture_vwc: Some(40.0 - f64::from(i) * 0.5),
                received_at: base() + Duration::minutes(i64::from(i) * 30),
            })
            .collect();
        let TrendVwcPerHour(slope) = moisture_trend(&samples, default_window()).unwrap();
        assert!((slope + 1.0).abs() < 1e-9, "{slope}");
        assert_eq!(moisture_trend(&samples[..4], default_window()), None);
    }

    proptest::proptest! {
        /// Noise must not flip the sign of a clear trend. The fit exists
        /// because endpoint differences do exactly that.
        #[test]
        fn a_clear_trend_keeps_its_sign_under_noise(
            noise in proptest::collection::vec(-1.5f64..1.5, 13),
            direction in proptest::bool::ANY,
        ) {
            let sign = if direction { 1.0 } else { -1.0 };
            let samples: Vec<TrendSample> = noise
                .iter()
                .enumerate()
                .map(|(i, n)| TrendSample {
                    value: 40.0 + sign * i as f64 * 1.0 + n,
                    at: base() + Duration::minutes(i as i64 * 30),
                    valid: true,
                })
                .collect();
            let t = fit(&samples, default_window()).unwrap();
            proptest::prop_assert!(
                t.per_hour * sign > 0.0,
                "noise flipped the sign: {} with direction {sign}",
                t.per_hour
            );
        }
    }
}
