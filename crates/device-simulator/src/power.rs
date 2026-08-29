//! The battery wake cycle (M5-021, [ADR-018](../../../../docs/adr/018-battery-and-deep-sleep-device-mode.md) §5).
//!
//! ```text
//! wake -> power peripherals -> warm up -> sample -> connect -> publish
//!      -> receive -> announce sleep -> disconnect -> sleep
//! ```
//!
//! # Sleep is a clean disconnect that publishes first
//!
//! The ordering is the whole point. The retained sleep announcement replaces the
//! retained online status, so a fresh subscriber sees a *sleeping* device rather
//! than a stale online one — and the Edge opens a bounded wake window from its
//! own receipt time. Publishing after disconnecting is not possible, and
//! disconnecting without announcing is what the `sleep-without-announcing`
//! fault does on purpose: it fires the Last Will, which is `connection_lost`,
//! which is `isolated`.
//!
//! The announcement does **not** replace the Last Will. The will stays armed for
//! the abnormal case, which is why an unclean drop still reads as an unexplained
//! absence (SAFETY-021).
//!
//! # The awake window is bounded by work, not by a clock
//!
//! `awake_budget_seconds` bounds an *idle* wake. An active watering cycle
//! extends it: a budget that could truncate a dose would be a way to strand an
//! energised pump, which the independent run guard would then have to catch —
//! correct, but a much worse design than not sleeping mid-dose.
use rhizo_mqtt_contract::payload::{PowerMode, PowerStatus, WakeReason};

/// What the device is doing between samples.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Phase {
    /// Awake, and within the peripheral warm-up. A reading taken now is not a
    /// reading.
    WarmingUp,
    /// Awake and usable.
    Awake,
    /// Off the air. Nothing is published and nothing is received.
    Sleeping,
}

/// The device's power behaviour.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PowerState {
    battery: bool,
    wake_interval_ms: u64,
    awake_budget_ms: u64,
    warmup_ms: u64,
    /// Monotonic milliseconds elapsed in the current wake.
    awake_ms: u64,
    /// Milliseconds of sleep still owed.
    sleep_remaining_ms: u64,
    phase: Phase,
    wake_reason: WakeReason,
    /// Wake cycles still to be skipped by `miss-wake`.
    misses_remaining: u32,
    /// Whether this wake has published a sampling cycle yet.
    sampled_this_wake: bool,
    /// Set when a wake completes, so the caller can spend the charge once.
    woke: bool,
}

impl PowerState {
    /// A mains device: awake for ever, and none of this applies.
    #[must_use]
    pub const fn always_on() -> Self {
        Self {
            battery: false,
            wake_interval_ms: 0,
            awake_budget_ms: 0,
            warmup_ms: 0,
            awake_ms: 0,
            sleep_remaining_ms: 0,
            phase: Phase::Awake,
            wake_reason: WakeReason::ColdBoot,
            misses_remaining: 0,
            sampled_this_wake: false,
            woke: false,
        }
    }

    /// A battery device with the given cycle.
    #[must_use]
    pub const fn battery(
        wake_interval_seconds: u32,
        awake_budget_seconds: u32,
        warmup_ms: u32,
    ) -> Self {
        Self {
            battery: true,
            wake_interval_ms: wake_interval_seconds as u64 * 1_000,
            awake_budget_ms: awake_budget_seconds as u64 * 1_000,
            warmup_ms: warmup_ms as u64,
            awake_ms: 0,
            sleep_remaining_ms: 0,
            // A device that has just booted is warming up like any other wake.
            phase: Phase::WarmingUp,
            wake_reason: WakeReason::ColdBoot,
            misses_remaining: 0,
            sampled_this_wake: false,
            woke: false,
        }
    }

    /// Whether this device sleeps at all.
    #[must_use]
    pub const fn is_battery(&self) -> bool {
        self.battery
    }
    /// The current phase.
    #[must_use]
    pub const fn phase(&self) -> Phase {
        self.phase
    }
    /// Whether the device is off the air.
    #[must_use]
    pub const fn is_sleeping(&self) -> bool {
        matches!(self.phase, Phase::Sleeping)
    }
    /// Whether a reading taken now is usable.
    #[must_use]
    pub const fn readings_usable(&self) -> bool {
        matches!(self.phase, Phase::Awake) || !self.battery
    }
    /// Virtual milliseconds of sleep still owed, for logging.
    #[must_use]
    pub const fn sleep_remaining_ms(&self) -> u64 {
        self.sleep_remaining_ms
    }
    /// The declared cycle, in seconds.
    #[must_use]
    pub const fn wake_interval_seconds(&self) -> u32 {
        (self.wake_interval_ms / 1_000) as u32
    }
    /// Records that this wake has published a sampling cycle.
    pub const fn note_sampled(&mut self) {
        self.sampled_this_wake = true;
    }

    /// Applies a new configured cycle, from `device.config` (M5-019).
    ///
    /// A configuration cannot wake a sleeping device or put an awake one to
    /// sleep mid-cycle; it changes what the *next* cycle does. Anything else
    /// would make a retained message able to strand a device.
    pub fn reconfigure(
        &mut self,
        mode: PowerMode,
        wake_interval_seconds: Option<u32>,
        warmup_ms: Option<u32>,
        awake_budget_seconds: Option<u32>,
    ) {
        let battery = mode.effective() == PowerMode::Battery;
        if battery != self.battery {
            self.battery = battery;
            self.phase = if battery {
                Phase::WarmingUp
            } else {
                Phase::Awake
            };
            self.awake_ms = 0;
        }
        if let Some(seconds) = wake_interval_seconds {
            self.wake_interval_ms = u64::from(seconds) * 1_000;
        }
        if let Some(ms) = warmup_ms {
            self.warmup_ms = u64::from(ms);
        }
        if let Some(seconds) = awake_budget_seconds {
            self.awake_budget_ms = u64::from(seconds) * 1_000;
        }
    }

    /// Skips the next `count` wake cycles without announcing anything.
    pub const fn miss_wakes(&mut self, count: u32) {
        self.misses_remaining = count;
    }

    /// Advances the cycle. Returns `true` when the device has just woken, so the
    /// caller can spend the charge once per wake.
    pub fn advance(&mut self, elapsed_ms: u64) -> bool {
        self.woke = false;
        if !self.battery {
            return false;
        }
        match self.phase {
            Phase::Sleeping => {
                self.sleep_remaining_ms = self.sleep_remaining_ms.saturating_sub(elapsed_ms);
                if self.sleep_remaining_ms == 0 {
                    if self.misses_remaining > 0 {
                        // The wake that never happened. Nothing is announced,
                        // nothing is published, and the Edge's window closes on
                        // a device that simply stopped talking.
                        self.misses_remaining -= 1;
                        self.sleep_remaining_ms = self.wake_interval_ms;
                    } else {
                        self.phase = Phase::WarmingUp;
                        self.awake_ms = 0;
                        self.wake_reason = WakeReason::Timer;
                        self.sampled_this_wake = false;
                        self.woke = true;
                    }
                }
            }
            Phase::WarmingUp => {
                self.awake_ms = self.awake_ms.saturating_add(elapsed_ms);
                if self.awake_ms >= self.warmup_ms {
                    self.phase = Phase::Awake;
                }
            }
            Phase::Awake => self.awake_ms = self.awake_ms.saturating_add(elapsed_ms),
        }
        self.woke
    }

    /// Whether the device should go back to sleep now.
    ///
    /// `busy` is true while a watering cycle is in flight, and it holds the
    /// device awake for as long as it takes — the awake budget bounds an idle
    /// wake only.
    #[must_use]
    pub const fn should_sleep(&self, busy: bool) -> bool {
        if !self.battery || busy || !matches!(self.phase, Phase::Awake) {
            return false;
        }
        // A wake that has not yet published its readings has not done its job.
        self.sampled_this_wake
            && self.awake_ms >= self.warmup_ms.saturating_add(self.awake_budget_ms)
    }

    /// Enters sleep. The caller publishes the announcement **first**.
    pub const fn sleep(&mut self) {
        self.phase = Phase::Sleeping;
        self.sleep_remaining_ms = self.wake_interval_ms;
        self.awake_ms = 0;
        self.sampled_this_wake = false;
    }

    /// The `power` block a status carries, or `None` for a mains device.
    ///
    /// A mains device declares nothing rather than declaring always-on: an
    /// absent block is what a pre-ADR-018 payload carries, and reproducing it
    /// exactly is what keeps the compatibility fixtures honest.
    #[must_use]
    pub fn status(&self, battery_mv: Option<u32>) -> Option<PowerStatus> {
        if !self.battery {
            return None;
        }
        Some(PowerStatus {
            mode: PowerMode::Battery,
            wake_interval_seconds: Some(self.wake_interval_seconds()),
            // A diagnostic, and deliberately relative to the device's own clock.
            // The Edge computes the window from its own receipt time and never
            // reads this (SAFETY-021).
            expected_wake_ms: Some(self.wake_interval_ms),
            wake_reason: Some(self.wake_reason),
            battery_mv,
            awake_ms: Some(self.awake_ms),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> PowerState {
        PowerState::battery(900, 20, 2_000)
    }

    #[test]
    fn a_wake_warms_up_before_its_readings_count() {
        let mut power = state();
        assert_eq!(power.phase(), Phase::WarmingUp);
        assert!(!power.readings_usable());
        power.advance(1_999);
        assert!(
            !power.readings_usable(),
            "a reading during warm-up is not one"
        );
        power.advance(1);
        assert_eq!(power.phase(), Phase::Awake);
        assert!(power.readings_usable());
    }

    #[test]
    fn an_idle_wake_ends_at_its_budget_and_a_busy_one_does_not() {
        let mut power = state();
        power.advance(2_000);
        power.note_sampled();
        power.advance(20_000);
        assert!(power.should_sleep(false));
        assert!(
            !power.should_sleep(true),
            "an active watering cycle holds the device awake however long it takes"
        );
    }

    #[test]
    fn a_wake_that_has_not_sampled_does_not_go_back_to_sleep() {
        let mut power = state();
        power.advance(2_000 + 20_000);
        assert!(
            !power.should_sleep(false),
            "a wake that published nothing has not done its job"
        );
    }

    #[test]
    fn sleeping_and_waking_are_a_cycle() {
        let mut power = state();
        power.advance(2_000);
        power.note_sampled();
        power.advance(20_000);
        power.sleep();
        assert!(power.is_sleeping());
        assert_eq!(power.sleep_remaining_ms(), 900_000);
        assert!(!power.advance(899_999));
        assert!(power.is_sleeping());
        assert!(power.advance(1), "the timer elapsed, so the device wakes");
        assert_eq!(power.phase(), Phase::WarmingUp);
        assert_eq!(
            power.status(None).unwrap().wake_reason,
            Some(WakeReason::Timer)
        );
    }

    /// `miss-wake` skips whole cycles without announcing: from the Edge's side
    /// the device simply stopped waking.
    #[test]
    fn miss_wake_skips_cycles_without_waking() {
        let mut power = state();
        power.advance(2_000);
        power.note_sampled();
        power.advance(20_000);
        power.miss_wakes(2);
        power.sleep();
        for cycle in 0..2 {
            assert!(!power.advance(900_000), "cycle {cycle} must be skipped");
            assert!(power.is_sleeping());
        }
        assert!(power.advance(900_000), "the third wake happens");
    }

    #[test]
    fn a_mains_device_never_sleeps() {
        let mut power = PowerState::always_on();
        assert!(!power.is_battery());
        assert!(power.readings_usable());
        assert!(!power.advance(10_000_000));
        assert!(!power.should_sleep(false));
        assert_eq!(
            power.status(Some(3_300)),
            None,
            "a mains device declares nothing"
        );
    }

    /// A configuration changes the next cycle. It cannot wake a sleeping device,
    /// which would make a retained message able to strand one.
    #[test]
    fn reconfiguration_changes_the_next_cycle_only() {
        let mut power = state();
        power.advance(2_000);
        power.note_sampled();
        power.advance(20_000);
        power.sleep();
        power.reconfigure(PowerMode::Battery, Some(60), Some(500), Some(5));
        assert!(
            power.is_sleeping(),
            "a config must not wake a sleeping device"
        );
        assert_eq!(power.wake_interval_seconds(), 60);

        // Retiring battery mode leaves the device awake from the next tick.
        power.reconfigure(PowerMode::AlwaysOn, None, None, None);
        assert!(!power.is_battery());
        assert!(!power.is_sleeping());
        assert!(power.readings_usable());

        // An unrecognised mode resolves to always-on (SAFETY-012).
        power.reconfigure(PowerMode::Unknown, None, None, None);
        assert!(!power.is_battery());
    }
}
