//! Monotonic offline budget and cooldown accounting (M9-018, SAFETY-015).
//!
//! # The rule
//!
//! A reboot must never replenish the water budget or shorten a cooldown. An
//! isolated device has no trustworthy wall clock, so both are tracked against
//! the monotonic timer and persisted conservatively:
//!
//! * the cooldown is stored as a **remaining duration**, never as a deadline. A
//!   deadline is meaningless to a device that cannot interpret absolute time
//!   after a reboot; a remainder is always interpretable and always
//!   conservative. It is also what `rhizo_policy::evaluate_offline` expects,
//!   because `elapsed` is a delta and there is no instant to subtract from.
//! * the budget window advances only from elapsed time the device actually
//!   **observed**. "Assume no time passed" is deliberately pessimistic: a device
//!   power-cycling every few minutes never earns budget, which is exactly right,
//!   because a reboot loop is not evidence that a day went by.
//!
//! # Deep sleep is the one exception, and only when it can prove itself
//!
//! [`credit_elapsed`] is the whole of it (ADR-018 §6, offline-autonomy §5b): a
//! **timer** wake with a **valid RTC checksum** credits the measured RTC
//! interval; every other reset reason and any checksum failure credits zero.
//! There is no third branch. Get this backwards and a corrupted RTC word
//! becomes free watering budget.

use serde::{Deserialize, Serialize};

use rhizo_policy::{MonotonicMillis, OfflineState};

use crate::power::{RtcSleepState, WakeReason};

/// The persisted conservative offline runtime state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct OfflineRuntime {
    /// Volume spent in the current rolling window.
    #[serde(default)]
    pub budget_used_ml: f32,
    /// Monotonic milliseconds accumulated in the current window.
    ///
    /// A *duration*, not an instant, for the same reason the cooldown is: the
    /// monotonic clock restarts at zero on every boot, so an instant recorded
    /// before a reboot means nothing afterwards.
    #[serde(default)]
    pub window_elapsed_ms: u64,
    /// Remaining cooldown, never a deadline.
    #[serde(default)]
    pub cooldown_remaining_ms: u64,
    /// Confirmation time accumulated toward the policy's confirm duration.
    #[serde(default)]
    pub confirm_elapsed_ms: u64,
    /// Doses delivered in the current cycle.
    #[serde(default)]
    pub dose_count: u16,
    /// The evaluator cycle phase, stored as its wire-stable name.
    #[serde(default)]
    pub cycle: PersistedCycle,
}

/// The evaluator cycle phase, in a form that survives NVS.
///
/// A local mirror of `rhizo_policy::OfflineCycle` rather than a `serde` impl on
/// the shared type: `rhizo-policy` is `no_std` and deliberately carries no
/// serialisation, and adding one for the firmware's storage format would put a
/// persistence concern in a crate that exists to be pure.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PersistedCycle {
    /// Nothing in progress.
    #[default]
    Idle,
    /// Accumulating confirmation time.
    Confirming,
    /// Waiting for a dose to be absorbed.
    WaitAbsorption,
    /// Cooling down after a cycle.
    Cooldown,
}

impl From<rhizo_policy::OfflineCycle> for PersistedCycle {
    fn from(value: rhizo_policy::OfflineCycle) -> Self {
        match value {
            rhizo_policy::OfflineCycle::Idle => Self::Idle,
            rhizo_policy::OfflineCycle::Confirming => Self::Confirming,
            rhizo_policy::OfflineCycle::WaitAbsorption => Self::WaitAbsorption,
            rhizo_policy::OfflineCycle::Cooldown => Self::Cooldown,
        }
    }
}

impl From<PersistedCycle> for rhizo_policy::OfflineCycle {
    fn from(value: PersistedCycle) -> Self {
        match value {
            PersistedCycle::Idle => Self::Idle,
            PersistedCycle::Confirming => Self::Confirming,
            PersistedCycle::WaitAbsorption => Self::WaitAbsorption,
            PersistedCycle::Cooldown => Self::Cooldown,
        }
    }
}

impl OfflineRuntime {
    /// The evaluator state this runtime represents.
    #[must_use]
    pub fn to_offline_state(self) -> OfflineState {
        OfflineState {
            cycle: self.cycle.into(),
            dose_count: self.dose_count,
            budget_used_ml: self.budget_used_ml,
            cooldown_remaining: MonotonicMillis(self.cooldown_remaining_ms),
            confirm_elapsed: MonotonicMillis(self.confirm_elapsed_ms),
        }
    }

    /// Absorbs the evaluator's next state, keeping the fields it does not own.
    pub fn apply_offline_state(&mut self, state: OfflineState) {
        self.cycle = state.cycle.into();
        self.dose_count = state.dose_count;
        self.budget_used_ml = state.budget_used_ml;
        self.cooldown_remaining_ms = state.cooldown_remaining.0;
        self.confirm_elapsed_ms = state.confirm_elapsed.0;
    }

    /// Advances the rolling budget window by observed elapsed time.
    ///
    /// The window is only ever advanced by time the device measured, and when
    /// a full window has passed the spent volume is released. Crediting zero —
    /// what a reboot and a failed RTC checksum both produce — advances nothing,
    /// which is the whole of SAFETY-015's guarantee.
    ///
    /// The remainder is computed with `%` rather than by subtracting in a loop.
    /// A loop is the obvious way to write it and is a real hazard here: a
    /// corrupted RTC word can offer a credit of `u64::MAX`, which is about
    /// 2e11 iterations of a day-long window — on an ESP32 that is a watchdog
    /// reset inside the accounting code, which is the last place to hang.
    pub fn credit_window(&mut self, elapsed: MonotonicMillis, window_ms: u64) {
        if window_ms == 0 {
            return;
        }
        let total = self.window_elapsed_ms.saturating_add(elapsed.0);
        if total >= window_ms {
            self.budget_used_ml = 0.0;
        }
        self.window_elapsed_ms = total % window_ms;
    }

    /// Accepts the edge's authoritative post-reconciliation baseline.
    ///
    /// The edge's figure is derived from committed rows and is authoritative
    /// once replay has completed; the local accumulator is reset to match
    /// rather than being merged, because two independent estimates of the same
    /// quantity is how double-counting starts.
    pub fn accept_edge_baseline(&mut self, budget_used_ml: f32) {
        if budget_used_ml.is_finite() && budget_used_ml >= 0.0 {
            self.budget_used_ml = budget_used_ml;
        }
    }
}

/// Elapsed monotonic time the device may credit for the interval it was away.
///
/// The single narrow rule of ADR-018 §6, written once so nobody re-derives it:
///
/// ```text
/// Timer  if checksum_valid(rtc_state)  -> rtc_counter_now - rtc_state.slept_at
/// _                                    -> ZERO
/// ```
///
/// There is no `_ =>` arm returning anything but zero, and there is no third
/// branch. A cold boot, a brownout, a watchdog reset, an external wake, and any
/// checksum failure all credit nothing.
#[must_use]
pub fn credit_elapsed(
    wake_reason: WakeReason,
    rtc: Option<&RtcSleepState>,
    rtc_counter_now_ms: u64,
) -> MonotonicMillis {
    match wake_reason {
        WakeReason::Timer => match rtc {
            Some(state) if state.checksum_valid() => {
                MonotonicMillis(rtc_counter_now_ms.saturating_sub(state.slept_at_ms))
            }
            // Spelled out rather than written `_`: a failed checksum and an
            // absent RTC block are different facts that happen to have the
            // same answer, and a catch-all here is how a *third* fact would
            // one day acquire that answer without anyone deciding it should.
            Some(_) | None => MonotonicMillis(0),
        },
        WakeReason::ColdBoot
        | WakeReason::External
        | WakeReason::Watchdog
        | WakeReason::Unknown => MonotonicMillis(0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_rtc(slept_at_ms: u64) -> RtcSleepState {
        RtcSleepState::seal(slept_at_ms, 3, 0)
    }

    #[test]
    fn safety_015_a_timer_wake_with_a_valid_checksum_credits_measured_time() {
        let rtc = valid_rtc(1_000);
        assert_eq!(
            credit_elapsed(WakeReason::Timer, Some(&rtc), 901_000),
            MonotonicMillis(900_000)
        );
    }

    #[test]
    fn safety_015_every_other_wake_reason_credits_zero() {
        let rtc = valid_rtc(1_000);
        for reason in [
            WakeReason::ColdBoot,
            WakeReason::External,
            WakeReason::Watchdog,
            WakeReason::Unknown,
        ] {
            assert_eq!(
                credit_elapsed(reason, Some(&rtc), 901_000),
                MonotonicMillis(0),
                "{reason:?}"
            );
        }
    }

    /// A corrupted RTC word must not become free watering budget. This is the
    /// branch that is only reachable with a fake RTC, which is why `app` is a
    /// host-testable crate at all.
    #[test]
    fn safety_015_a_failed_checksum_credits_zero_even_on_a_timer_wake() {
        let mut rtc = valid_rtc(1_000);
        rtc.slept_at_ms = 5;
        assert!(!rtc.checksum_valid());
        assert_eq!(
            credit_elapsed(WakeReason::Timer, Some(&rtc), 901_000),
            MonotonicMillis(0)
        );
        assert_eq!(
            credit_elapsed(WakeReason::Timer, None, 901_000),
            MonotonicMillis(0)
        );
    }

    /// The RTC counter cannot legitimately run backwards, but a corrupt read
    /// could make it look as though it had. Saturating subtraction means the
    /// worst case is zero credit, never a wrapped enormous one.
    #[test]
    fn a_backwards_rtc_counter_credits_zero_rather_than_wrapping() {
        let rtc = valid_rtc(900_000);
        assert_eq!(
            credit_elapsed(WakeReason::Timer, Some(&rtc), 1_000),
            MonotonicMillis(0)
        );
    }

    #[test]
    fn a_reboot_neither_replenishes_the_budget_nor_shortens_the_cooldown() {
        let mut runtime = OfflineRuntime {
            budget_used_ml: 120.0,
            window_elapsed_ms: 3_600_000,
            cooldown_remaining_ms: 900_000,
            ..OfflineRuntime::default()
        };
        let encoded = serde_json::to_vec(&runtime).expect("encodes");
        let restored: OfflineRuntime = serde_json::from_slice(&encoded).expect("decodes");
        assert_eq!(restored, runtime);

        // A reboot credits zero. Nothing moves.
        runtime.credit_window(MonotonicMillis(0), 86_400_000);
        assert_eq!(runtime.budget_used_ml, 120.0);
        assert_eq!(runtime.cooldown_remaining_ms, 900_000);
    }

    #[test]
    fn the_budget_releases_only_when_a_full_window_has_actually_elapsed() {
        let mut runtime = OfflineRuntime {
            budget_used_ml: 120.0,
            ..OfflineRuntime::default()
        };
        runtime.credit_window(MonotonicMillis(86_399_999), 86_400_000);
        assert_eq!(runtime.budget_used_ml, 120.0);
        runtime.credit_window(MonotonicMillis(1), 86_400_000);
        assert_eq!(runtime.budget_used_ml, 0.0);
    }

    #[test]
    fn the_edge_baseline_replaces_rather_than_merges() {
        let mut runtime = OfflineRuntime {
            budget_used_ml: 120.0,
            ..OfflineRuntime::default()
        };
        runtime.accept_edge_baseline(40.0);
        assert_eq!(runtime.budget_used_ml, 40.0);
        runtime.accept_edge_baseline(f32::NAN);
        assert_eq!(
            runtime.budget_used_ml, 40.0,
            "a non-finite baseline is ignored"
        );
        runtime.accept_edge_baseline(-1.0);
        assert_eq!(
            runtime.budget_used_ml, 40.0,
            "a negative baseline is ignored"
        );
    }

    #[test]
    fn a_saturating_window_credit_cannot_overflow_into_free_budget() {
        let mut runtime = OfflineRuntime {
            budget_used_ml: 50.0,
            window_elapsed_ms: u64::MAX - 5,
            ..OfflineRuntime::default()
        };
        runtime.credit_window(MonotonicMillis(u64::MAX), 86_400_000);
        // Saturation cannot wrap, so the accumulator only ever moves forward
        // and the release is the honest consequence of an enormous credit
        // rather than an arithmetic accident.
        assert!(runtime.window_elapsed_ms < 86_400_000);
    }
}
