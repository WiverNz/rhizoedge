//! Continuous time below the moisture target (PRD 050 F-050-12).
//!
//! The `dry_confirm_minutes` debounce is what separates `Drying` from
//! `DryConfirmed`: it exists so a momentary dip does not become a dose.
//!
//! # The gap case is the whole difficulty
//!
//! If samples stop for two hours and resume dry, those two hours **must not**
//! count as confirmed dryness: nobody knows what happened in them, and
//! uncertainty is not evidence (SAFETY-012). Duration therefore accumulates
//! only across intervals the edge actually observed, and a gap longer than the
//! staleness threshold resets the accumulator rather than filling it in.
//!
//! An invalid sample neither accumulates nor resets. It is not evidence that the
//! soil recovered, and it is not evidence that it stayed dry.
//!
//! The state is persisted, so a restart mid-debounce neither loses progress nor
//! invents it.
use chrono::{DateTime, Duration, Utc};

/// The accumulator, persisted between ticks and across restarts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DryDuration {
    /// Observed continuous milliseconds below the target.
    pub dry_ms: i64,
    /// Edge receipt time of the last **valid** sample folded in.
    pub last_sample_at: Option<DateTime<Utc>>,
}

/// What one observation did to the accumulator.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DryOutcome {
    /// Continuous dryness grew by an observed interval.
    Accumulated,
    /// A valid reading at or above the target cleared it.
    Recovered,
    /// A gap longer than the staleness threshold cleared it: the missing time
    /// is unknown, not dry.
    ResetByGap,
    /// The sample was invalid, so it neither accumulated nor reset.
    Ignored,
}

/// Configuration for one plant.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DryConfig {
    /// The moisture target the plant is measured against.
    pub target_min: f64,
    /// The freshness threshold. A gap at or beyond this is a reset.
    pub stale_after: Duration,
}

impl DryDuration {
    /// Folds one observation in and reports what it did.
    ///
    /// `value` is `None` for a failed read and `Some` only for a reading that
    /// already passed validation.
    pub fn observe(
        &mut self,
        value: Option<f64>,
        at: DateTime<Utc>,
        config: &DryConfig,
    ) -> DryOutcome {
        let Some(value) = value.filter(|v| v.is_finite()) else {
            return DryOutcome::Ignored;
        };
        let gap = self
            .last_sample_at
            .map(|last| at.signed_duration_since(last));
        // A sample that arrives before the last one recorded is out of order.
        // Treated as a gap, because a negative interval is not observed time.
        let gapped = gap.is_none_or(|g| g >= config.stale_after || g < Duration::zero());
        if value >= config.target_min {
            self.dry_ms = 0;
            self.last_sample_at = Some(at);
            return DryOutcome::Recovered;
        }
        if gapped {
            self.dry_ms = 0;
            self.last_sample_at = Some(at);
            return DryOutcome::ResetByGap;
        }
        let observed = gap.unwrap_or_else(Duration::zero).num_milliseconds();
        self.dry_ms = self.dry_ms.saturating_add(observed);
        self.last_sample_at = Some(at);
        DryOutcome::Accumulated
    }

    /// Whether the debounce has elapsed.
    #[must_use]
    pub fn is_confirmed(&self, confirm: Duration) -> bool {
        self.dry_ms >= confirm.num_milliseconds()
    }

    /// Continuous dryness as a duration.
    #[must_use]
    pub fn duration(&self) -> Duration {
        Duration::milliseconds(self.dry_ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn config() -> DryConfig {
        DryConfig {
            target_min: 28.0,
            stale_after: Duration::minutes(15),
        }
    }
    fn base() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap()
    }

    #[test]
    fn continuous_dryness_accumulates_from_observed_intervals() {
        let mut state = DryDuration::default();
        let mut outcomes = Vec::new();
        for i in 0..7 {
            outcomes.push(state.observe(Some(24.0), base() + Duration::minutes(i * 5), &config()));
        }
        // The first sample has no predecessor, so it starts the clock rather
        // than counting time nobody saw.
        assert_eq!(outcomes[0], DryOutcome::ResetByGap);
        assert!(outcomes[1..].iter().all(|o| *o == DryOutcome::Accumulated));
        assert_eq!(state.duration(), Duration::minutes(30));
        assert!(state.is_confirmed(Duration::minutes(30)));
        assert!(!state.is_confirmed(Duration::minutes(31)));
    }

    #[test]
    fn one_sample_at_or_above_the_target_resets_it() {
        let mut state = DryDuration::default();
        for i in 0..7 {
            state.observe(Some(24.0), base() + Duration::minutes(i * 5), &config());
        }
        assert_eq!(
            state.observe(Some(28.0), base() + Duration::minutes(35), &config()),
            DryOutcome::Recovered,
            "the target itself is not dry"
        );
        assert_eq!(state.dry_ms, 0);
    }

    /// The subtle one: unobserved time is unknown, not dry.
    #[test]
    fn a_gap_longer_than_the_staleness_threshold_does_not_accumulate() {
        let mut state = DryDuration::default();
        for i in 0..7 {
            state.observe(Some(24.0), base() + Duration::minutes(i * 5), &config());
        }
        assert_eq!(state.duration(), Duration::minutes(30));
        assert_eq!(
            state.observe(Some(24.0), base() + Duration::hours(3), &config()),
            DryOutcome::ResetByGap
        );
        assert_eq!(
            state.dry_ms, 0,
            "two silent hours are not two hours of confirmed dryness"
        );
        // Accumulation restarts from the resumed observation.
        state.observe(
            Some(24.0),
            base() + Duration::hours(3) + Duration::minutes(5),
            &config(),
        );
        assert_eq!(state.duration(), Duration::minutes(5));
    }

    #[test]
    fn an_invalid_sample_neither_accumulates_nor_resets() {
        let mut state = DryDuration::default();
        for i in 0..3 {
            state.observe(Some(24.0), base() + Duration::minutes(i * 5), &config());
        }
        let before = state;
        assert_eq!(
            state.observe(None, base() + Duration::minutes(20), &config()),
            DryOutcome::Ignored
        );
        assert_eq!(state, before);
        assert_eq!(
            state.observe(Some(f64::NAN), base() + Duration::minutes(25), &config()),
            DryOutcome::Ignored
        );
        assert_eq!(state, before);
    }

    /// A reading timestamped before the last one is not observed time.
    #[test]
    fn an_out_of_order_sample_is_treated_as_a_gap() {
        let mut state = DryDuration::default();
        state.observe(Some(24.0), base() + Duration::minutes(10), &config());
        state.observe(Some(24.0), base() + Duration::minutes(15), &config());
        assert_eq!(state.duration(), Duration::minutes(5));
        assert_eq!(
            state.observe(Some(24.0), base() + Duration::minutes(5), &config()),
            DryOutcome::ResetByGap
        );
        assert_eq!(state.dry_ms, 0);
    }

    /// The persisted shape round-trips: a restart resumes mid-debounce with the
    /// same accumulator, and the next observed interval continues it.
    #[test]
    fn the_accumulator_survives_a_restart_as_plain_data() {
        let mut state = DryDuration {
            dry_ms: Duration::minutes(20).num_milliseconds(),
            last_sample_at: Some(base()),
        };
        let restored = DryDuration {
            dry_ms: state.dry_ms,
            last_sample_at: state.last_sample_at,
        };
        assert_eq!(state, restored);
        state.observe(Some(24.0), base() + Duration::minutes(5), &config());
        assert_eq!(state.duration(), Duration::minutes(25));
    }
}
