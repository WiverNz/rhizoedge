//! Warning and critical threshold evaluation (M5-015).
//!
//! A warning and a control condition are different things. A critical ambient
//! temperature is real and worth alerting on, and it is **not** a reason to pump
//! water — wiring thresholds into actuation would be the category error the role
//! model (M5-013) exists to prevent. Nothing in this module produces a decision,
//! a command, or a [`crate::state::LockoutReason`], and
//! [`tests::thresholds_never_actuate`] fails if that changes.
//!
//! Alerts are raised whether or not the plant has an actuator: a monitoring-only
//! plant is the common case, and its critical readings matter just as much.
//!
//! # Hysteresis and confirmation, for the same reason as irrigation
//!
//! A value hovering on a threshold otherwise produces an alert per tick, and an
//! operator who is alerted constantly stops reading alerts (ADR-010). A crossing
//! is therefore recorded **once per transition**: a candidate severity must hold
//! for `confirm_duration_ms` before it becomes current, and leaving a severity
//! requires the reading to come back inside the band by `hysteresis`.
use chrono::{DateTime, Duration, Utc};

use crate::plant::MeasurementPolicy;

/// How bad a reading is.
#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub enum Severity {
    /// Inside every band.
    #[default]
    Normal,
    /// Outside the warning band.
    Warning,
    /// Outside the critical band.
    Critical,
}

impl Severity {
    /// The stable wire name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Warning => "warning",
            Self::Critical => "critical",
        }
    }
}

/// Per (plant, kind) crossing state, persisted so a restart does not re-alert.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ThresholdState {
    /// The severity currently in force.
    pub current: Severity,
    /// A severity waiting out its confirmation window.
    pub candidate: Option<Severity>,
    /// When the candidate first appeared, on the **edge** clock.
    pub candidate_since: Option<DateTime<Utc>>,
}

/// A crossing worth recording. One per transition, never one per tick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Crossing {
    /// The severity left behind.
    pub from: Severity,
    /// The severity entered.
    pub to: Severity,
    /// The reading that confirmed it.
    pub value: f64,
    /// Edge receipt time of that reading.
    pub at: DateTime<Utc>,
}

/// The severity a reading has on its own, ignoring history.
///
/// `hysteresis` widens the band the reading must re-enter to *leave* the current
/// severity, and is applied only in the improving direction — a deteriorating
/// reading is never held back.
#[must_use]
pub fn raw_severity(value: f64, policy: &MeasurementPolicy, current: Severity) -> Severity {
    if !value.is_finite() {
        return current;
    }
    let margin = |level: Severity| -> f64 {
        // Leaving a level is harder than entering it, by exactly the configured
        // hysteresis. Entering is never made easier.
        if current >= level {
            policy.hysteresis.filter(|h| h.is_finite()).unwrap_or(0.0)
        } else {
            0.0
        }
    };
    let critical_margin = margin(Severity::Critical);
    let below_critical = policy
        .critical_low
        .is_some_and(|low| value <= low + critical_margin);
    let above_critical = policy
        .critical_high
        .is_some_and(|high| value >= high - critical_margin);
    if below_critical || above_critical {
        return Severity::Critical;
    }
    let warning_margin = margin(Severity::Warning);
    let below_warning = policy
        .warning_low
        .is_some_and(|low| value <= low + warning_margin);
    let above_warning = policy
        .warning_high
        .is_some_and(|high| value >= high - warning_margin);
    if below_warning || above_warning {
        return Severity::Warning;
    }
    Severity::Normal
}

/// Folds one reading in, returning a crossing only on an actual transition.
///
/// `value` is `None` for a missing or invalid reading. A missing reading changes
/// nothing: it is not evidence that the condition cleared, and it is not
/// evidence that it worsened (SAFETY-012). Freshness is a separate question,
/// answered by `stale_after_ms` at the call site.
pub fn evaluate(
    state: &mut ThresholdState,
    value: Option<f64>,
    at: DateTime<Utc>,
    policy: &MeasurementPolicy,
) -> Option<Crossing> {
    let value = value.filter(|v| v.is_finite())?;
    let observed = raw_severity(value, policy, state.current);
    if observed == state.current {
        state.candidate = None;
        state.candidate_since = None;
        return None;
    }
    let confirm = Duration::milliseconds(i64::from(policy.confirm_duration_ms.unwrap_or(0)));
    if state.candidate != Some(observed) {
        state.candidate = Some(observed);
        state.candidate_since = Some(at);
    }
    let since = state.candidate_since.unwrap_or(at);
    if at.signed_duration_since(since) < confirm {
        return None;
    }
    let from = state.current;
    state.current = observed;
    state.candidate = None;
    state.candidate_since = None;
    Some(Crossing {
        from,
        to: observed,
        value,
        at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use rhizo_mqtt_contract::payload::MeasurementKind;

    fn base() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap()
    }
    fn policy() -> MeasurementPolicy {
        MeasurementPolicy {
            kind: MeasurementKind::AmbientTemperature,
            target_min: Some(18.0),
            target_max: Some(27.0),
            warning_low: Some(12.0),
            warning_high: Some(30.0),
            critical_low: Some(5.0),
            critical_high: Some(35.0),
            stale_after_ms: 900_000,
            hysteresis: Some(1.0),
            confirm_duration_ms: None,
        }
    }

    #[test]
    fn crossings_raise_events_with_the_right_severity() {
        let mut state = ThresholdState::default();
        assert_eq!(evaluate(&mut state, Some(21.0), base(), &policy()), None);

        let warning = evaluate(&mut state, Some(31.0), base(), &policy()).unwrap();
        assert_eq!(warning.from, Severity::Normal);
        assert_eq!(warning.to, Severity::Warning);

        let critical = evaluate(&mut state, Some(36.0), base(), &policy()).unwrap();
        assert_eq!(critical.from, Severity::Warning);
        assert_eq!(critical.to, Severity::Critical);
    }

    /// One event per transition, not one per tick. This is the difference
    /// between an alert an operator reads and a stream they mute.
    #[test]
    fn a_crossing_raises_one_event_per_transition() {
        let mut state = ThresholdState::default();
        assert!(evaluate(&mut state, Some(31.0), base(), &policy()).is_some());
        for tick in 1..40 {
            assert_eq!(
                evaluate(
                    &mut state,
                    Some(31.0 + f64::from(tick) * 0.01),
                    base() + Duration::seconds(i64::from(tick) * 30),
                    &policy()
                ),
                None,
                "tick {tick}"
            );
        }
        assert_eq!(state.current, Severity::Warning);
    }

    /// Hysteresis prevents oscillation at the boundary: a reading that dips a
    /// hair back inside the band does not clear the warning.
    #[test]
    fn hysteresis_prevents_oscillation_at_the_boundary() {
        let mut state = ThresholdState::default();
        evaluate(&mut state, Some(30.5), base(), &policy()).unwrap();
        assert_eq!(state.current, Severity::Warning);
        for value in [29.9, 29.5, 29.1] {
            assert_eq!(
                evaluate(&mut state, Some(value), base(), &policy()),
                None,
                "{value} is inside the 1.0 hysteresis band"
            );
            assert_eq!(state.current, Severity::Warning);
        }
        let cleared = evaluate(&mut state, Some(28.5), base(), &policy()).unwrap();
        assert_eq!(cleared.to, Severity::Normal);
    }

    #[test]
    fn a_candidate_must_hold_for_the_confirmation_window() {
        let policy = MeasurementPolicy {
            confirm_duration_ms: Some(600_000),
            ..policy()
        };
        let mut state = ThresholdState::default();
        assert_eq!(evaluate(&mut state, Some(31.0), base(), &policy), None);
        assert_eq!(
            evaluate(
                &mut state,
                Some(31.0),
                base() + Duration::minutes(9),
                &policy
            ),
            None
        );
        let crossing = evaluate(
            &mut state,
            Some(31.0),
            base() + Duration::minutes(10),
            &policy,
        )
        .unwrap();
        assert_eq!(crossing.to, Severity::Warning);

        // A candidate that goes away resets the window rather than banking it.
        let mut state = ThresholdState::default();
        evaluate(&mut state, Some(31.0), base(), &policy);
        evaluate(
            &mut state,
            Some(21.0),
            base() + Duration::minutes(5),
            &policy,
        );
        assert_eq!(state.candidate, None);
        assert_eq!(
            evaluate(
                &mut state,
                Some(31.0),
                base() + Duration::minutes(11),
                &policy
            ),
            None,
            "the window restarts from the new candidate"
        );
    }

    #[test]
    fn a_missing_or_invalid_reading_neither_raises_nor_clears() {
        let mut state = ThresholdState::default();
        evaluate(&mut state, Some(36.0), base(), &policy()).unwrap();
        let before = state;
        assert_eq!(evaluate(&mut state, None, base(), &policy()), None);
        assert_eq!(
            evaluate(&mut state, Some(f64::NAN), base(), &policy()),
            None
        );
        assert_eq!(state, before, "silence does not clear a critical alert");
    }

    /// A policy that configures no band for a kind evaluates cleanly and alerts
    /// about nothing, rather than inventing a threshold.
    #[test]
    fn a_policy_with_no_bands_alerts_about_nothing() {
        let bare = MeasurementPolicy {
            warning_low: None,
            warning_high: None,
            critical_low: None,
            critical_high: None,
            ..policy()
        };
        let mut state = ThresholdState::default();
        for value in [-100.0, 0.0, 1_000.0] {
            assert_eq!(evaluate(&mut state, Some(value), base(), &bare), None);
        }
        assert_eq!(state.current, Severity::Normal);
    }

    /// The explicit negative control: no threshold crossing of any kind can
    /// reach actuation. Structural, because a behavioural test could only show
    /// that it does not happen today.
    #[test]
    fn thresholds_never_actuate() {
        // The needles are assembled from fragments so that this test's own
        // source does not contain the strings it forbids. Scanning a file that
        // names the thing it bans is a test that can only fail.
        let source = include_str!("threshold.rs");
        for forbidden in [
            concat!("validate_water", "_command"),
            concat!("Water", "Command"),
            concat!("Decision", "::Water"),
            concat!("Offline", "Decision"),
            concat!("Command", "Verdict"),
        ] {
            assert!(
                !source.contains(forbidden),
                "threshold.rs mentions {forbidden}: thresholds inform, they never water"
            );
        }
        // A lockout may be *named* in prose but never constructed here.
        assert!(
            !source.contains(concat!("Lockout", "Reason::")),
            "threshold.rs constructs a lockout: thresholds are not a safety gate"
        );
    }
}
