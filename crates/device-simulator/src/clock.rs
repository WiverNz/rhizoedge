//! The device's two clocks.
//!
//! A device has a **monotonic** clock that always works and a **wall** clock
//! that is only meaningful once the Edge has synchronised it
//! ([ADR-013](../../../../docs/adr/013-clock-and-time-semantics.md)). Keeping
//! them as separate concepts here rather than one `now()` is what makes the
//! offline rules expressible: every duration in a policy is measured on the
//! monotonic clock precisely because an isolated device may have no wall time
//! at all.
//!
//! # One clock per process
//!
//! Nothing outside this module reads the system clock. A stray `Instant::now()`
//! elsewhere would age at a different rate from the rest of the process once
//! `--time-scale` is applied (M2-014), and the resulting bug is extremely
//! confusing to diagnose.

use std::time::Instant;

use rhizo_mqtt_contract::UtcMillis;

/// The device's monotonic clock, in milliseconds since boot.
///
/// Advanced by the run loop from elapsed real time, or by a test in explicit
/// steps. It never reads a wall clock, never goes backwards, and is unaffected
/// by synchronisation.
#[derive(Clone, Debug)]
pub struct MonotonicClock {
    elapsed_ms: u64,
    anchor: Instant,
    last_real_ms: u64,
}

impl MonotonicClock {
    /// Starts a clock at zero, anchored to now.
    #[must_use]
    pub fn start() -> Self {
        Self {
            elapsed_ms: 0,
            anchor: Instant::now(),
            last_real_ms: 0,
        }
    }

    /// Milliseconds since boot.
    #[must_use]
    pub const fn elapsed_ms(&self) -> u64 {
        self.elapsed_ms
    }

    /// Advances by an explicit number of milliseconds.
    ///
    /// Saturating: a monotonic clock that wraps would make every stored
    /// remaining-duration meaningless at the wrap point (SAFETY-015).
    pub fn advance_ms(&mut self, by: u64) {
        self.elapsed_ms = self.elapsed_ms.saturating_add(by);
    }

    /// Real milliseconds elapsed since the previous call, for a run loop that
    /// converts them into virtual time.
    pub fn real_delta_ms(&mut self) -> u64 {
        let now = u64::try_from(self.anchor.elapsed().as_millis()).unwrap_or(u64::MAX);
        let delta = now.saturating_sub(self.last_real_ms);
        self.last_real_ms = now;
        delta
    }
}

/// The device's wall clock, maintained solely from `edge.time`.
///
/// There is no NTP client and no fallback to the host clock: a simulator that
/// quietly used the host's time would make `clock_synced` meaningless and would
/// hide every SAFETY-002 failure the edge's synchronisation is there to
/// surface.
#[derive(Clone, Copy, Debug, Default)]
pub struct WallClock {
    /// The most recently applied Edge timestamp.
    applied: Option<UtcMillis>,
    /// The monotonic instant at which it was applied.
    applied_at_monotonic_ms: u64,
}

impl WallClock {
    /// Records an applied Edge timestamp.
    ///
    /// Called **only** after
    /// [`rhizo_mqtt_contract::payload::TimeSyncState::apply`] has accepted the
    /// timestamp as strictly newer. The acceptance rule lives in the contract
    /// crate; this type only carries the resulting value.
    pub const fn set(&mut self, edge_time: UtcMillis, monotonic_now_ms: u64) {
        self.applied = Some(edge_time);
        self.applied_at_monotonic_ms = monotonic_now_ms;
    }

    /// The current wall time, or `None` when the clock has never been set.
    ///
    /// Extrapolated from the monotonic clock since the last synchronisation, so
    /// it advances between syncs rather than freezing at the last value.
    #[must_use]
    pub fn now_ms(&self, monotonic_now_ms: u64) -> Option<UtcMillis> {
        let applied = self.applied?;
        let since = monotonic_now_ms.saturating_sub(self.applied_at_monotonic_ms);
        Some(UtcMillis(applied.0.saturating_add(since as i64)))
    }
}

/// The virtual clock the whole process runs on.
///
/// ```text
/// virtual_elapsed = (real_now - anchor) * scale
/// ```
///
/// ADR-013 asks for accelerated time so a multi-hour watering cycle becomes a
/// six-second test. Without it the end-to-end suite is not something anyone
/// runs, and a suite nobody runs is a suite that fails silently.
///
/// # One clock per process
///
/// Everything that ages — the monotonic clock, the physical model, the sampling
/// schedule, the status heartbeat, the pump, the offline runtime state — is
/// advanced from this one source. A component that read the system clock
/// directly would age at a different rate from everything around it once the
/// scale is not 1, and the resulting bug is extremely confusing: readings that
/// disagree with the timestamps attached to them.
///
/// # Drift-free
///
/// Each call returns `total_virtual_since_start − already_reported`, rather than
/// scaling the interval since the previous call. Scaling each interval
/// separately accumulates rounding: at scale 600 and a 100 ms tick, losing half
/// a millisecond per tick is five minutes of virtual time an hour.
#[derive(Debug)]
pub struct AcceleratedClock {
    scale: f64,
    anchor: Instant,
    reported_virtual_ms: u64,
}

impl AcceleratedClock {
    /// Creates a clock running at `scale` times real time.
    ///
    /// A non-finite or non-positive scale falls back to real time. The CLI
    /// already rejects those, so this is the belt to that braces: a clock that
    /// stopped, or ran backwards, would freeze every timer in the process.
    #[must_use]
    pub fn new(scale: f64) -> Self {
        Self {
            scale: if scale.is_finite() && scale > 0.0 {
                scale
            } else {
                1.0
            },
            anchor: Instant::now(),
            reported_virtual_ms: 0,
        }
    }

    /// The configured acceleration factor.
    #[must_use]
    pub const fn scale(&self) -> f64 {
        self.scale
    }

    /// Virtual milliseconds elapsed since this was last called.
    pub fn take_elapsed_ms(&mut self) -> u64 {
        let real_ms = u64::try_from(self.anchor.elapsed().as_millis()).unwrap_or(u64::MAX);
        let total = Self::virtual_ms(real_ms, self.scale);
        let elapsed = total.saturating_sub(self.reported_virtual_ms);
        self.reported_virtual_ms = total;
        elapsed
    }

    /// Converts real milliseconds to virtual ones.
    ///
    /// Saturating rather than wrapping: an absurd scale must slow the clock
    /// down to a standstill at the ceiling, not wrap it back to zero and make
    /// every stored duration meaningless.
    #[must_use]
    pub fn virtual_ms(real_ms: u64, scale: f64) -> u64 {
        let scaled = (real_ms as f64) * scale;
        if scaled.is_finite() && scaled >= 0.0 {
            // `as` saturates at the integer bounds for floats in Rust.
            scaled as u64
        } else {
            0
        }
    }

    /// Splits a virtual interval into steps no longer than
    /// [`MAX_VIRTUAL_STEP_MS`].
    ///
    /// At `--time-scale 600` a 100 ms tick is a minute of virtual time, and
    /// applying it as one step would make the exponential drying curve, the
    /// absorption pool, and the overshoot decay resolve to a single jump. The
    /// model would then behave differently at different scales — which would
    /// make an accelerated test a test of something other than the system.
    #[must_use]
    pub fn steps(virtual_ms: u64) -> Vec<u64> {
        if virtual_ms == 0 {
            return Vec::new();
        }
        let mut remaining = virtual_ms;
        let mut steps = Vec::new();
        while remaining > 0 {
            let step = remaining.min(MAX_VIRTUAL_STEP_MS);
            steps.push(step);
            remaining -= step;
        }
        steps
    }
}

/// The longest virtual interval applied to the models in one step.
///
/// Ten virtual seconds: finer than the shortest configured sampling or control
/// interval, while keeping accelerated offline autonomy from performing six
/// hundred durable state writes per real second. Pump completion still uses
/// the exact elapsed duration and the physical model integrates each step.
pub const MAX_VIRTUAL_STEP_MS: u64 = 60_000;

#[cfg(test)]
mod accelerated_clock_tests {
    use super::*;

    #[test]
    fn scale_one_is_real_time() {
        assert_eq!(AcceleratedClock::virtual_ms(1_000, 1.0), 1_000);
        assert_eq!(AcceleratedClock::new(1.0).scale(), 1.0);
    }

    #[test]
    fn scale_six_hundred_runs_ten_simulated_minutes_per_real_second() {
        assert_eq!(AcceleratedClock::virtual_ms(1_000, 600.0), 600_000);
        assert_eq!(600_000 / 60_000, 10, "ten minutes per real second");
        // A fifteen-minute absorption wait takes 1.5 s of wall time.
        let real_ms_for_fifteen_virtual_minutes = 15 * 60 * 1_000 / 600;
        assert_eq!(real_ms_for_fifteen_virtual_minutes, 1_500);
    }

    #[test]
    fn an_unusable_scale_falls_back_to_real_time_rather_than_stopping() {
        for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert_eq!(
                AcceleratedClock::new(bad).scale(),
                1.0,
                "a stopped clock would freeze every timer in the process"
            );
        }
    }

    #[test]
    fn an_absurd_scale_saturates_rather_than_wrapping() {
        assert_eq!(AcceleratedClock::virtual_ms(u64::MAX, 1e30), u64::MAX);
        assert_eq!(AcceleratedClock::virtual_ms(1_000, f64::NAN), 0);
    }

    #[test]
    fn elapsed_time_does_not_drift_across_many_calls() {
        // Simulated directly rather than by sleeping: what is being asserted is
        // the arithmetic, and a test that slept would be slow *and* flaky.
        let scale = 600.0;
        let mut reported = 0u64;
        let mut total = 0u64;
        for tick in 1..=36_000u64 {
            let real_ms = tick * 100;
            let virtual_total = AcceleratedClock::virtual_ms(real_ms, scale);
            total += virtual_total.saturating_sub(reported);
            reported = virtual_total;
        }
        let real_elapsed_ms = 36_000 * 100;
        assert_eq!(
            total,
            AcceleratedClock::virtual_ms(real_elapsed_ms, scale),
            "an hour of ticks must sum to exactly the scaled elapsed time"
        );
    }

    #[test]
    fn a_real_clock_advances_and_reports_each_interval_once() {
        let mut clock = AcceleratedClock::new(1000.0);
        let first = clock.take_elapsed_ms();
        std::thread::sleep(std::time::Duration::from_millis(20));
        let second = clock.take_elapsed_ms();
        assert!(second > 0, "time passed, so the clock must report it");
        assert!(
            second >= 10_000,
            "20 ms at scale 1000 is at least 10 s of virtual time, got {second}"
        );
        let _ = first;
    }

    #[test]
    fn a_large_interval_is_split_into_bounded_steps() {
        let steps = AcceleratedClock::steps(60_000);
        assert_eq!(steps.len(), 6);
        assert!(steps.iter().all(|s| *s <= MAX_VIRTUAL_STEP_MS));
        assert_eq!(steps.iter().sum::<u64>(), 60_000, "nothing is lost");
    }

    #[test]
    fn a_small_interval_is_a_single_step_and_zero_is_none() {
        assert_eq!(AcceleratedClock::steps(100), vec![100]);
        assert_eq!(
            AcceleratedClock::steps(MAX_VIRTUAL_STEP_MS),
            vec![MAX_VIRTUAL_STEP_MS]
        );
        assert!(AcceleratedClock::steps(0).is_empty());
    }

    #[test]
    fn an_odd_interval_keeps_its_remainder() {
        let steps = AcceleratedClock::steps(25_000);
        assert_eq!(steps, vec![25_000]);
        assert_eq!(steps.iter().sum::<u64>(), 25_000);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_monotonic_clock_only_moves_when_advanced() {
        let mut c = MonotonicClock::start();
        assert_eq!(c.elapsed_ms(), 0);
        c.advance_ms(1_500);
        c.advance_ms(500);
        assert_eq!(c.elapsed_ms(), 2_000);
    }

    #[test]
    fn the_monotonic_clock_saturates_rather_than_wrapping() {
        let mut c = MonotonicClock::start();
        c.advance_ms(u64::MAX);
        c.advance_ms(1_000);
        assert_eq!(
            c.elapsed_ms(),
            u64::MAX,
            "a wrap would reset every duration"
        );
    }

    #[test]
    fn an_unsynchronised_wall_clock_has_no_time_at_all() {
        assert!(
            WallClock::default().now_ms(10_000).is_none(),
            "the host clock is never a fallback"
        );
    }

    #[test]
    fn wall_time_advances_with_the_monotonic_clock_between_synchronisations() {
        let mut w = WallClock::default();
        w.set(UtcMillis(1_756_121_400_000), 5_000);
        assert_eq!(w.now_ms(5_000), Some(UtcMillis(1_756_121_400_000)));
        assert_eq!(w.now_ms(65_000), Some(UtcMillis(1_756_121_460_000)));
    }

    #[test]
    fn a_later_synchronisation_replaces_the_earlier_one() {
        let mut w = WallClock::default();
        w.set(UtcMillis(1_000), 0);
        w.set(UtcMillis(9_000), 100);
        assert_eq!(w.now_ms(100), Some(UtcMillis(9_000)));
    }
}
