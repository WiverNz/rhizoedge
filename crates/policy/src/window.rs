//! The rolling budget window — **the one implementation** of the device-side
//! volume accounting both consumers share (ADR-008, ADR-015, SAFETY-014).
//!
//! # Why this is here and not in each device
//!
//! [`evaluate_offline`](crate::evaluate_offline) reads `budget_used_ml` and
//! refuses a dose that would cross `limits.max_volume_per_window_ml`, but it is
//! pure and clock-free, so it cannot be the thing that *releases* the budget
//! when a window elapses. That release therefore has to happen in the caller —
//! and for a while it happened in each caller separately, with two different
//! answers:
//!
//! ```text
//! a credit of 30 h against a 24 h window
//!   firmware   budget -> 0, window_elapsed -> 6 h   (the overshoot carried)
//!   simulator  budget -> 0, window_elapsed -> 0     (the overshoot dropped)
//! ```
//!
//! The firmware replenished eighteen hours later, the simulator twenty-four —
//! the firmware being the *more permissive* of the two, against the
//! implementation the project treats as canonical. That is the divergence
//! ADR-008 exists to prevent, arriving through the one piece of offline
//! arithmetic that had been left outside the shared crate. The correct answer
//! is the tumbling window the `%` produces: a window that ended six hours ago
//! ended six hours ago, and restarting the clock at the moment the device
//! happened to look would make the replenishment rate depend on how often it
//! looked.
//!
//! # The cooldown is not here, deliberately
//!
//! [`next_offline_state`](crate::next_offline_state) already counts the
//! cooldown down by the same `elapsed`. A caller that also decremented it would
//! halve every cooldown — which is exactly what one of them did. The rule is
//! one owner per quantity: **the evaluator owns the cooldown and the
//! confirmation, this owns the window**, and a device owns neither.

use crate::types::MonotonicMillis;

/// The rolling volume budget and the time accumulated inside it.
///
/// Both fields are *durations and totals*, never instants: a device that may
/// reboot at any moment cannot interpret a monotonic instant recorded before
/// the reboot, and an isolated device may have no wall clock to record one in.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BudgetWindow {
    /// Volume spent inside the current window.
    pub used_ml: f32,
    /// Monotonic milliseconds accumulated in the current window.
    pub elapsed_ms: u64,
}

impl BudgetWindow {
    /// Credits observed monotonic time, releasing the spent volume once a full
    /// window has passed.
    ///
    /// `elapsed` is a **delta the device actually observed** — what
    /// `credit_elapsed` produces from a timer wake with a valid RTC checksum,
    /// and zero for every other reset reason. Crediting zero advances nothing,
    /// so a device in a reboot loop never earns water (SAFETY-015).
    ///
    /// The remainder is taken with `%` rather than by subtracting in a loop. A
    /// loop is the obvious way to write it and is a real hazard: a corrupted
    /// RTC word can present a credit of `u64::MAX`, about 2e11 iterations of a
    /// day-long window, which on an ESP32 is a watchdog reset inside the
    /// accounting code — the last place to hang.
    ///
    /// A zero-length window is not a window. It releases nothing and advances
    /// nothing, rather than dividing by zero or replenishing on every call;
    /// `OfflinePolicy::validate` already refuses `window_ms == 0`, so reaching
    /// this with one means the policy was never validated, and inventing a
    /// replenishment for it would be the permissive reading of a fault.
    pub fn credit(&mut self, elapsed: MonotonicMillis, window_ms: u64) {
        if window_ms == 0 {
            return;
        }
        let total = self.elapsed_ms.saturating_add(elapsed.0);
        if total >= window_ms {
            self.used_ml = 0.0;
        }
        self.elapsed_ms = total % window_ms;
    }

    /// Charges a delivered volume against the window.
    ///
    /// A non-finite volume charges nothing rather than poisoning the total:
    /// `NaN + x` is `NaN`, and `NaN > max` is `false`, so a poisoned total
    /// would read as *under* budget for ever. The gate's own `is_finite` guard
    /// catches it too, and neither is redundant — this keeps the number usable,
    /// that one refuses to act on an unusable one.
    pub fn charge(&mut self, ml: f32) {
        if ml.is_finite() && ml > 0.0 {
            self.used_ml += ml;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: u64 = 86_400_000;

    /// The divergence that prompted the extraction, pinned as a value.
    #[test]
    fn a_credit_longer_than_the_window_carries_the_overshoot() {
        let mut window = BudgetWindow {
            used_ml: 300.0,
            elapsed_ms: 0,
        };
        window.credit(MonotonicMillis(30 * 3_600_000), DAY);
        assert_eq!(window.used_ml, 0.0, "a full window releases the budget");
        assert_eq!(
            window.elapsed_ms,
            6 * 3_600_000,
            "the overshoot is carried, not dropped: restarting the clock here \
             would make replenishment depend on how often the device looked"
        );
    }

    /// Many windows in one credit release once and terminate promptly.
    #[test]
    fn several_windows_in_one_credit_terminate_promptly() {
        let mut window = BudgetWindow {
            used_ml: 120.0,
            elapsed_ms: 0,
        };
        window.credit(MonotonicMillis(u64::MAX), DAY);
        assert_eq!(window.used_ml, 0.0);
        assert!(window.elapsed_ms < DAY);
    }

    /// Crediting zero — a reboot, or a failed RTC checksum — advances nothing.
    #[test]
    fn safety_015_zero_credit_advances_nothing() {
        let mut window = BudgetWindow {
            used_ml: 250.0,
            elapsed_ms: 1_000,
        };
        for _ in 0..1_000 {
            window.credit(MonotonicMillis(0), DAY);
        }
        assert_eq!(window.used_ml, 250.0);
        assert_eq!(window.elapsed_ms, 1_000);
    }

    /// Time short of a full window releases nothing at all.
    #[test]
    fn a_partial_window_releases_nothing() {
        let mut window = BudgetWindow {
            used_ml: 250.0,
            elapsed_ms: 0,
        };
        window.credit(MonotonicMillis(DAY - 1), DAY);
        assert_eq!(window.used_ml, 250.0);
        assert_eq!(window.elapsed_ms, DAY - 1);
        window.credit(MonotonicMillis(1), DAY);
        assert_eq!(window.used_ml, 0.0);
        assert_eq!(window.elapsed_ms, 0);
    }

    /// A zero-length window releases nothing rather than everything.
    #[test]
    fn a_zero_length_window_is_not_a_window() {
        let mut window = BudgetWindow {
            used_ml: 250.0,
            elapsed_ms: 0,
        };
        window.credit(MonotonicMillis(DAY), 0);
        assert_eq!(window.used_ml, 250.0);
        assert_eq!(window.elapsed_ms, 0);
    }

    /// A non-finite charge cannot poison the total into reading under budget.
    #[test]
    fn safety_012_a_non_finite_charge_is_ignored() {
        let mut window = BudgetWindow::default();
        window.charge(35.0);
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -1.0, 0.0] {
            window.charge(bad);
        }
        assert_eq!(window.used_ml, 35.0);
    }
}
