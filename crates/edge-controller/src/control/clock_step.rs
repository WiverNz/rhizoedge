//! Edge wall-clock step detection (M6-015, F-060-50…52).
//!
//! [ADR-013](../../../../docs/adr/013-clock-and-time-semantics.md) §"Edge clock
//! steps". The edge host is genuinely NTP-synced, unlike the devices, so its
//! clock can *jump*. A forward jump drops older `watering_events` out of the
//! rolling 24-hour window early, and a plant that had spent its allowance could
//! be handed a fresh one.
//!
//! # The monotonic reference is what makes detection possible
//!
//! Comparing the wall clock against itself cannot reveal a step. Each tick
//! samples `Instant` alongside the wall clock; if the two disagree by more than
//! the tolerance, the wall clock moved and the monotonic clock did not.
//!
//! # The asymmetry is deliberate
//!
//! **Forward** beyond ten minutes locks every plant `Uncertain` for one
//! cooldown. Heavy-handed, and correct: the alternative is accepting that the
//! daily cap can be bypassed by an NTP correction, which is exactly the class of
//! subtle hole SAFETY-012 exists to close.
//!
//! **Backward** is logged and nothing else. A backward step makes the rolling
//! window include *more* history, so the cap becomes more conservative on its
//! own.

use std::time::Instant;

use chrono::{DateTime, Duration, Utc};

/// A step beyond this is a step, not drift.
///
/// Ordinary NTP slew is milliseconds per tick; ten minutes is far outside
/// anything a healthy host produces between two thirty-second ticks.
pub const FORWARD_STEP_THRESHOLD: Duration = Duration::minutes(10);

/// How much wall/monotonic divergence is tolerated before it is called a step.
///
/// Generous enough that a loaded host, a slow tick, or a suspended process does
/// not produce a false positive.
pub const TOLERANCE: Duration = Duration::seconds(30);

/// Which way the clock moved.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Direction {
    /// The wall clock jumped ahead of the monotonic reference.
    Forward,
    /// The wall clock fell behind it.
    Backward,
}

impl Direction {
    /// The stable label used in the metric and the recorded event.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Forward => "forward",
            Self::Backward => "backward",
        }
    }
}

/// A detected step.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Step {
    /// Which way.
    pub direction: Direction,
    /// How far, always positive.
    pub magnitude: Duration,
}

impl Step {
    /// Whether this step must lock every plant out (F-060-51).
    ///
    /// Only a forward step past the threshold does. Backward is safe by
    /// construction and is recorded for diagnosis alone.
    #[must_use]
    pub fn locks_out(self) -> bool {
        match self.direction {
            Direction::Forward => self.magnitude > FORWARD_STEP_THRESHOLD,
            Direction::Backward => false,
        }
    }
}

/// Samples the wall clock against a monotonic reference.
///
/// The monotonic half is the process's own `Instant`, so the detector is
/// per-process and does not survive a restart — which is correct: a restart
/// cannot tell a clock step from an ordinary gap between runs, and assuming one
/// would lock every plant out on every boot.
#[derive(Clone, Copy, Debug)]
pub struct Detector {
    wall: DateTime<Utc>,
    monotonic: Instant,
}

impl Detector {
    /// Starts a detector from the current pair.
    #[must_use]
    pub fn new(wall: DateTime<Utc>, monotonic: Instant) -> Self {
        Self { wall, monotonic }
    }

    /// Folds in a new sample, returning a step when one occurred.
    ///
    /// The reference is advanced whether or not a step was found, so one jump is
    /// reported once rather than on every tick afterwards.
    pub fn observe(&mut self, wall: DateTime<Utc>, monotonic: Instant) -> Option<Step> {
        self.observe_at_rate(wall, monotonic, 1.0)
    }

    /// Observes a clock whose intentional wall-time rate may be accelerated.
    /// Test acceleration is not a wall-clock step and must not trigger the
    /// conservative anomaly response on every control tick.
    pub fn observe_at_rate(
        &mut self,
        wall: DateTime<Utc>,
        monotonic: Instant,
        rate: f64,
    ) -> Option<Step> {
        let elapsed_monotonic =
            Duration::from_std(monotonic.saturating_duration_since(self.monotonic))
                .unwrap_or_else(|_| Duration::zero());
        let elapsed_monotonic = Duration::milliseconds(
            (elapsed_monotonic.num_milliseconds() as f64 * rate).round() as i64,
        );
        let elapsed_wall = wall.signed_duration_since(self.wall);
        self.wall = wall;
        self.monotonic = monotonic;
        let divergence = elapsed_wall - elapsed_monotonic;
        if divergence > TOLERANCE {
            Some(Step {
                direction: Direction::Forward,
                magnitude: divergence,
            })
        } else if divergence < -TOLERANCE {
            Some(Step {
                direction: Direction::Backward,
                magnitude: -divergence,
            })
        } else {
            None
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
#[allow(
    clippy::module_inception,
    reason = "the module name is the verification filter the issue quotes literally"
)]
mod clock_step {
    use super::*;
    use chrono::TimeZone;
    use std::time::Duration as StdDuration;

    fn base() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, 30, 12, 0, 0)
            .single()
            .unwrap()
    }

    #[test]
    fn ordinary_drift_is_not_a_step() {
        let start = Instant::now();
        let mut detector = Detector::new(base(), start);
        // Thirty seconds of monotonic time and thirty seconds and 200 ms of
        // wall time: a slewing clock, not a jumping one.
        let step = detector.observe(
            base() + Duration::milliseconds(30_200),
            start + StdDuration::from_secs(30),
        );
        assert_eq!(step, None);
    }

    /// SCEN-071.
    #[test]
    fn a_forward_step_is_detected_and_locks_out() {
        let start = Instant::now();
        let mut detector = Detector::new(base(), start);
        let step = detector
            .observe(
                base() + Duration::hours(2),
                start + StdDuration::from_secs(30),
            )
            .expect("two hours of wall time in thirty seconds is a step");
        assert_eq!(step.direction, Direction::Forward);
        assert!(step.magnitude > Duration::minutes(100));
        assert!(step.locks_out());
    }

    /// SCEN-072: logged, and nothing else.
    #[test]
    fn a_backward_step_is_detected_and_causes_no_lockout() {
        let start = Instant::now();
        let mut detector = Detector::new(base(), start);
        let step = detector
            .observe(
                base() - Duration::hours(1),
                start + StdDuration::from_secs(30),
            )
            .expect("an hour backwards is a step");
        assert_eq!(step.direction, Direction::Backward);
        assert!(!step.locks_out(), "a backward step is conservative already");
        assert_eq!(step.direction.as_str(), "backward");
    }

    /// A forward step smaller than the threshold is recorded but does not lock
    /// anything out — the threshold is ten minutes, not thirty seconds.
    #[test]
    fn a_small_forward_step_is_recorded_without_locking_out() {
        let start = Instant::now();
        let mut detector = Detector::new(base(), start);
        let step = detector
            .observe(
                base() + Duration::minutes(5),
                start + StdDuration::from_secs(30),
            )
            .expect("five minutes in thirty seconds is a step");
        assert_eq!(step.direction, Direction::Forward);
        assert!(!step.locks_out());
    }

    /// One jump is reported once, not on every tick afterwards.
    #[test]
    fn a_step_is_reported_once() {
        let start = Instant::now();
        let mut detector = Detector::new(base(), start);
        let jumped = base() + Duration::hours(2);
        assert!(
            detector
                .observe(jumped, start + StdDuration::from_secs(30))
                .is_some()
        );
        assert_eq!(
            detector.observe(
                jumped + Duration::seconds(30),
                start + StdDuration::from_secs(60)
            ),
            None,
            "the reference advanced, so the same jump is not re-reported"
        );
    }
}
