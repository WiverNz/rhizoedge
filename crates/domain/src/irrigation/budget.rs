//! The rolling 24-hour water cap (M6-007, SAFETY-006).
//!
//! **Derived from rows, never from a counter.** A counter would need its own
//! persistence, its own recovery, and its own bugs; a sum over
//! `watering_events` survives restarts, crashes, and clock steps for free, and
//! is the last line of defence against a logic bug anywhere else in the machine.
//!
//! **Rolling, not calendar** ([ADR-013](../../../../docs/adr/013-clock-and-time-semantics.md)).
//! A calendar-day cap permits two full daily allowances a few hours apart, one
//! either side of midnight.
//!
//! Everything here is pure arithmetic over values the caller read from SQLite,
//! so the flagship property test can generate ten thousand adversarial histories
//! for the price of a few milliseconds.

use chrono::{DateTime, Duration, Utc};
use rhizo_mqtt_contract::payload::CommandStatus;

/// The width of the rolling window.
pub const ROLLING_WINDOW_HOURS: i64 = 24;

/// The modes whose delivered volume counts against the automatic cap.
///
/// `manual` is excluded because a person took responsibility for it, and
/// `detected` because the system did not deliver it at all — both still reset
/// the **cooldown** through the plant's last-watering time, which is a different
/// question (PRD 060 §Data model). `automatic` covers offline-autonomous doses
/// too: there is one budget per plant, not one per control path (SAFETY-014).
pub const BUDGETED_MODES: [&str; 2] = ["automatic", "recommended"];

/// The instant the rolling window opens.
#[must_use]
pub fn window_start(now: DateTime<Utc>) -> DateTime<Utc> {
    now - Duration::hours(ROLLING_WINDOW_HOURS)
}

/// Whether one more dose of `dose_ml` fits inside the cap.
///
/// The **second** of SAFETY-006's two checks: the gate refuses once the total
/// has already reached the ceiling, and this refuses a dose that would cross it,
/// so a crossing dose is never issued at all. Non-finite inputs answer `false`:
/// an edge that cannot prove it is under budget assumes it is not, exactly as
/// protocol §5.8 step 11 requires of the device.
#[must_use]
pub fn dose_fits(delivered_last_24h_ml: f32, dose_ml: f32, max_daily_ml: f32) -> bool {
    if !delivered_last_24h_ml.is_finite() || !dose_ml.is_finite() || !max_daily_ml.is_finite() {
        return false;
    }
    if dose_ml <= 0.0 {
        return false;
    }
    delivered_last_24h_ml + dose_ml <= max_daily_ml
}

/// What remains of the cap, never negative and never `NaN`.
#[must_use]
pub fn remaining_ml(delivered_last_24h_ml: f32, max_daily_ml: f32) -> f32 {
    if !delivered_last_24h_ml.is_finite() || !max_daily_ml.is_finite() {
        return 0.0;
    }
    (max_daily_ml - delivered_last_24h_ml).max(0.0)
}

/// The volume a settled command charges to the rolling window.
///
/// Conservative by construction, and exhaustive over the wire status with no
/// catch-all arm:
///
/// - `completed` charges what was delivered — or, if the device could not say,
///   what was requested.
/// - `rejected` charges nothing and creates no watering event: the pump never
///   ran, and recording one would corrupt the budget in the permissive
///   direction.
/// - `interrupted` and `failed` charge the **full `requested_ml`**. That
///   over-counts when the interruption was early, which is the deliberate
///   direction: over-counting reduces the next dose, under-counting could permit
///   an extra one (PRD 060 §Open questions 2).
/// - a status this contract version does not recognise charges the full request,
///   for the same reason (SAFETY-012).
#[must_use]
pub fn credited_ml(status: CommandStatus, requested_ml: f32, delivered_ml: Option<f32>) -> f32 {
    let requested = if requested_ml.is_finite() && requested_ml > 0.0 {
        requested_ml
    } else {
        0.0
    };
    match status {
        CommandStatus::Completed => delivered_ml
            .filter(|v| v.is_finite() && *v >= 0.0)
            .unwrap_or(requested),
        CommandStatus::Rejected => 0.0,
        CommandStatus::Interrupted | CommandStatus::Failed | CommandStatus::Unknown => requested,
    }
}

/// Whether a settled command should create a `watering_event`.
///
/// Only `completed` asserts that water reached the plant. Everything else is
/// credited to the budget through the command row instead, because a watering
/// event is a claim about the plant and inventing one for a refused command
/// corrupts both the daily total and the cooldown.
#[must_use]
pub const fn creates_watering_event(status: CommandStatus) -> bool {
    match status {
        CommandStatus::Completed => true,
        CommandStatus::Rejected
        | CommandStatus::Interrupted
        | CommandStatus::Failed
        | CommandStatus::Unknown => false,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod daily_cap {
    use super::*;
    use chrono::TimeZone;
    use proptest::prelude::*;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, 30, 12, 0, 0)
            .single()
            .unwrap()
    }

    #[test]
    fn the_window_is_rolling_and_exactly_a_day_wide() {
        assert_eq!(window_start(now()), now() - Duration::hours(24));
    }

    #[test]
    fn only_automatic_and_recommended_count_against_the_cap() {
        assert_eq!(BUDGETED_MODES, ["automatic", "recommended"]);
        assert!(!BUDGETED_MODES.contains(&"manual"));
        assert!(!BUDGETED_MODES.contains(&"detected"));
    }

    #[test]
    fn a_dose_that_would_cross_the_cap_does_not_fit() {
        assert!(dose_fits(260.0, 40.0, 300.0), "exactly reaching it is fine");
        assert!(!dose_fits(261.0, 40.0, 300.0));
        assert!(!dose_fits(0.0, 0.0, 300.0), "a zero dose is not a dose");
    }

    #[test]
    fn an_unreadable_total_never_fits() {
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert!(!dose_fits(bad, 40.0, 300.0), "delivered {bad}");
            assert!(!dose_fits(0.0, bad, 300.0), "dose {bad}");
            assert!(!dose_fits(0.0, 40.0, bad), "max {bad}");
            assert_eq!(remaining_ml(bad, 300.0), 0.0);
            assert_eq!(remaining_ml(0.0, bad), 0.0);
        }
    }

    #[test]
    fn remaining_is_never_negative() {
        assert_eq!(remaining_ml(400.0, 300.0), 0.0);
        assert!((remaining_ml(100.0, 300.0) - 200.0).abs() < f32::EPSILON);
    }

    /// PRD 060 F-060-26 and its open question: the conservative direction.
    #[test]
    fn crediting_is_conservative_for_every_status() {
        assert!((credited_ml(CommandStatus::Completed, 40.0, Some(38.5)) - 38.5).abs() < 1e-6);
        assert!(
            (credited_ml(CommandStatus::Completed, 40.0, None) - 40.0).abs() < 1e-6,
            "a device that cannot say how much it delivered is charged the request"
        );
        assert_eq!(credited_ml(CommandStatus::Rejected, 40.0, None), 0.0);
        assert!((credited_ml(CommandStatus::Interrupted, 40.0, None) - 40.0).abs() < 1e-6);
        assert!((credited_ml(CommandStatus::Failed, 40.0, None) - 40.0).abs() < 1e-6);
        assert!(
            (credited_ml(CommandStatus::Unknown, 40.0, None) - 40.0).abs() < 1e-6,
            "an unrecognised outcome is charged, never forgiven"
        );
        assert!(
            (credited_ml(CommandStatus::Completed, 40.0, Some(f32::NAN)) - 40.0).abs() < 1e-6,
            "an unreadable delivered volume falls back to the request"
        );
    }

    #[test]
    fn only_a_completed_command_creates_a_watering_event() {
        assert!(creates_watering_event(CommandStatus::Completed));
        for status in [
            CommandStatus::Rejected,
            CommandStatus::Interrupted,
            CommandStatus::Failed,
            CommandStatus::Unknown,
        ] {
            assert!(!creates_watering_event(status), "{status:?}");
        }
    }

    proptest! {
        /// A dose that fits never takes the total past the ceiling, whatever the
        /// numbers.
        #[test]
        fn a_fitting_dose_never_crosses_the_ceiling(
            delivered in 0.0f32..1_000.0,
            dose in 0.1f32..1_000.0,
            max in 0.0f32..1_000.0,
        ) {
            if dose_fits(delivered, dose, max) {
                prop_assert!(delivered + dose <= max + f32::EPSILON);
            }
        }

        /// Crediting never returns less than nothing and never returns `NaN`.
        #[test]
        fn crediting_is_always_a_usable_number(
            requested in -10.0f32..1_000.0,
            delivered in proptest::option::of(-10.0f32..1_000.0),
        ) {
            for status in [
                CommandStatus::Completed,
                CommandStatus::Rejected,
                CommandStatus::Interrupted,
                CommandStatus::Failed,
                CommandStatus::Unknown,
            ] {
                let credited = credited_ml(status, requested, delivered);
                prop_assert!(credited.is_finite());
                prop_assert!(credited >= 0.0);
            }
        }
    }
}
