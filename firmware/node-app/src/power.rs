//! Power mode, the wake cycle, and RTC-retained sleep state (M9-019, ADR-018).
//!
//! # Why the state machine is here and not next to `esp_deep_sleep`
//!
//! F-090-51 requires **exactly one `deep_sleep` call site**, reachable only
//! from the top of the wake loop. A `deep_sleep()` reachable from a command
//! handler or an error path is how a device sleeps with the pump energised, and
//! the structure has to make that unrepresentable rather than merely unlikely.
//!
//! [`WakePhase`] is that structure: the only phase that yields
//! [`WakeAction::Sleep`] is [`WakePhase::ReadyToSleep`], and the only way into
//! it is through [`WakeCycle::request_sleep`], which refuses while an awake
//! hold is held. The ESP-IDF side does nothing but obey.
//!
//! # RTC memory is a cache that may vanish
//!
//! [`RtcSleepState`] lives in `.rtc.data` on the device: it survives deep sleep
//! and **not** a power cut, brownout, or most other resets. Its checksum is
//! therefore not defensive, it is the discriminator between "we know how long
//! we slept" and "we do not" — see [`crate::budget::credit_elapsed`].

use serde::{Deserialize, Serialize};

use crate::awake_hold::{AwakeHold, HoldCount};
use crate::persist::crc32;

pub use rhizo_mqtt_contract::payload::{PowerMode, WakeReason};

/// The sleep-cycle accounting kept in RTC-retained memory.
///
/// Deliberately small and fixed-shape: it is copied into a linker-placed static
/// on the device, and a growing struct there is a silent corruption waiting for
/// a firmware upgrade.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RtcSleepState {
    /// RTC counter value at the moment sleep was entered.
    pub slept_at_ms: u64,
    /// Boot generation carried across the sleep.
    pub boot_generation: u64,
    /// Cooldown remaining when sleep was entered.
    pub cooldown_remaining_ms: u64,
    /// CRC-32 over the three fields above.
    pub checksum: u32,
}

impl RtcSleepState {
    /// Seals the state with its checksum.
    #[must_use]
    pub fn seal(slept_at_ms: u64, boot_generation: u64, cooldown_remaining_ms: u64) -> Self {
        let mut state = Self {
            slept_at_ms,
            boot_generation,
            cooldown_remaining_ms,
            checksum: 0,
        };
        state.checksum = state.compute_checksum();
        state
    }

    /// Whether the retained words are self-consistent.
    #[must_use]
    pub fn checksum_valid(&self) -> bool {
        self.checksum == self.compute_checksum()
    }

    fn compute_checksum(&self) -> u32 {
        let mut bytes = [0u8; 24];
        bytes[0..8].copy_from_slice(&self.slept_at_ms.to_le_bytes());
        bytes[8..16].copy_from_slice(&self.boot_generation.to_le_bytes());
        bytes[16..24].copy_from_slice(&self.cooldown_remaining_ms.to_le_bytes());
        crc32(&bytes)
    }
}

/// Where the device is in one wake cycle (ADR-018 §5).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum WakePhase {
    /// Just woken; peripherals still unpowered.
    #[default]
    Woken,
    /// Rails on, waiting out `sensor_warmup_ms`.
    WarmingUp,
    /// Sampling and networking.
    Active,
    /// A dose is in progress or its result is unacknowledged.
    Holding,
    /// Sleep has been announced and its PUBACK observed.
    ReadyToSleep,
    /// Always-on: this phase is never left.
    StayingAwake,
}

/// What the caller should do next.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WakeAction {
    /// Enable the rails and start the warm-up timer.
    PowerUp,
    /// Keep waiting for warm-up.
    Wait,
    /// Sample, publish, and service the connection.
    Work,
    /// Enter deep sleep. **The only value that authorises the one call site.**
    Sleep,
    /// Remain awake indefinitely (always-on mode).
    StayAwake,
}

/// Why a sleep request was refused.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SleepRefused {
    /// The device is always-on and never sleeps.
    AlwaysOn,
    /// An awake hold is outstanding: a dose is running or unreported.
    HoldOutstanding,
    /// The sleep announcement has not been published and acknowledged.
    AnnouncementUnacknowledged,
}

/// The wake-cycle state machine.
#[derive(Clone, Debug)]
pub struct WakeCycle {
    mode: PowerMode,
    phase: WakePhase,
    wake_reason: WakeReason,
    holds: HoldCount,
    announced: bool,
    woke_at_ms: u64,
    warmup_started_ms: Option<u64>,
}

impl WakeCycle {
    /// Starts a cycle in the configured mode.
    ///
    /// An absent or unrecognised mode has already resolved to
    /// [`PowerMode::AlwaysOn`] by the time it reaches here — the resolution
    /// lives in the shared contract's `PowerMode::effective`, so there is one
    /// copy of "uncertainty must not choose the branch that makes a device
    /// unreachable" (SAFETY-012).
    #[must_use]
    pub fn new(mode: PowerMode, wake_reason: WakeReason, monotonic_ms: u64) -> Self {
        let mode = mode.effective();
        Self {
            mode,
            phase: if mode == PowerMode::Battery {
                WakePhase::Woken
            } else {
                WakePhase::StayingAwake
            },
            wake_reason,
            holds: HoldCount::new(),
            announced: false,
            woke_at_ms: monotonic_ms,
            warmup_started_ms: None,
        }
    }

    /// The effective power mode.
    #[must_use]
    pub const fn mode(&self) -> PowerMode {
        self.mode
    }

    /// The current phase.
    #[must_use]
    pub const fn phase(&self) -> WakePhase {
        self.phase
    }

    /// Why the device is awake, reported truthfully in status (F-090-54).
    #[must_use]
    pub const fn wake_reason(&self) -> WakeReason {
        self.wake_reason
    }

    /// How long this wake has lasted.
    #[must_use]
    pub const fn awake_ms(&self, monotonic_ms: u64) -> u64 {
        monotonic_ms.saturating_sub(self.woke_at_ms)
    }

    /// Whether any awake hold is outstanding.
    #[must_use]
    pub fn is_held(&self) -> bool {
        self.holds.is_held()
    }

    /// Acquires an awake hold (M9-021).
    ///
    /// Returns a guard: the hold is released when it is dropped, which is what
    /// makes every error path correct without the author having remembered.
    #[must_use]
    pub fn acquire_hold(&mut self) -> AwakeHold {
        let hold = self.holds.acquire();
        if self.mode == PowerMode::Battery {
            self.phase = WakePhase::Holding;
        }
        hold
    }

    /// Re-synchronises the phase after a hold guard has been dropped.
    ///
    /// The guard cannot reach the cycle's phase — that is the price of getting
    /// the error paths for free — so the loop calls this once per iteration.
    /// The authoritative fact is the count, which `Drop` maintains; the phase
    /// only mirrors it.
    pub fn sync_holds(&mut self) {
        if !self.holds.is_held() && self.phase == WakePhase::Holding {
            self.phase = WakePhase::Active;
        }
    }

    /// Records that the sleep announcement's PUBACK was observed (F-090-52).
    pub fn announcement_acknowledged(&mut self) {
        self.announced = true;
    }

    /// Advances the cycle and says what to do next.
    pub fn step(&mut self, monotonic_ms: u64, warmup_ms: u32) -> WakeAction {
        match self.phase {
            WakePhase::StayingAwake => WakeAction::StayAwake,
            WakePhase::Woken => {
                self.phase = WakePhase::WarmingUp;
                self.warmup_started_ms = Some(monotonic_ms);
                WakeAction::PowerUp
            }
            WakePhase::WarmingUp => {
                let started = self.warmup_started_ms.unwrap_or(monotonic_ms);
                if monotonic_ms.saturating_sub(started) >= u64::from(warmup_ms) {
                    self.phase = WakePhase::Active;
                    WakeAction::Work
                } else {
                    WakeAction::Wait
                }
            }
            WakePhase::Active | WakePhase::Holding => WakeAction::Work,
            WakePhase::ReadyToSleep => WakeAction::Sleep,
        }
    }

    /// Whether the sensor rail has been powered long enough to be trusted.
    ///
    /// A reading taken before a sensor has settled is not a bad reading — it is
    /// a *plausible* one, which is worse (M9-020).
    #[must_use]
    pub fn warmup_complete(&self, monotonic_ms: u64, warmup_ms: u32) -> bool {
        if self.mode == PowerMode::AlwaysOn {
            return true;
        }
        match self.warmup_started_ms {
            Some(started) => monotonic_ms.saturating_sub(started) >= u64::from(warmup_ms),
            None => false,
        }
    }

    /// Whether an *idle* wake has spent its budget.
    ///
    /// Bounds only an idle wake: a held wake runs until the hold is released,
    /// because a budget that could truncate a dose would be a way to strand an
    /// energised pump (ADR-018 §5).
    #[must_use]
    pub fn idle_budget_spent(&self, monotonic_ms: u64, awake_budget_seconds: u32) -> bool {
        !self.is_held() && self.awake_ms(monotonic_ms) >= u64::from(awake_budget_seconds) * 1000
    }

    /// Requests entry into sleep.
    ///
    /// # Errors
    ///
    /// Refuses while the device is always-on, while a hold is outstanding, or
    /// before the sleep announcement has been acknowledged. Those three
    /// refusals are the whole of "the device does not sleep with a pump
    /// running or a result unreported".
    pub fn request_sleep(&mut self) -> Result<(), SleepRefused> {
        if self.mode != PowerMode::Battery {
            return Err(SleepRefused::AlwaysOn);
        }
        if self.is_held() {
            return Err(SleepRefused::HoldOutstanding);
        }
        if !self.announced {
            return Err(SleepRefused::AnnouncementUnacknowledged);
        }
        self.phase = WakePhase::ReadyToSleep;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_absent_or_unrecognised_mode_yields_always_on() {
        for mode in [PowerMode::Unknown, PowerMode::AlwaysOn] {
            let cycle = WakeCycle::new(mode, WakeReason::ColdBoot, 0);
            assert_eq!(cycle.mode(), PowerMode::AlwaysOn);
            assert_eq!(cycle.phase(), WakePhase::StayingAwake);
        }
    }

    #[test]
    fn always_on_never_produces_the_sleep_action() {
        let mut cycle = WakeCycle::new(PowerMode::AlwaysOn, WakeReason::ColdBoot, 0);
        assert_eq!(cycle.step(0, 500), WakeAction::StayAwake);
        assert_eq!(cycle.request_sleep(), Err(SleepRefused::AlwaysOn));
        assert_eq!(cycle.step(10_000, 500), WakeAction::StayAwake);
    }

    #[test]
    fn a_battery_wake_powers_up_then_warms_up_then_works() {
        let mut cycle = WakeCycle::new(PowerMode::Battery, WakeReason::Timer, 0);
        assert_eq!(cycle.step(0, 500), WakeAction::PowerUp);
        assert_eq!(cycle.step(100, 500), WakeAction::Wait);
        assert!(!cycle.warmup_complete(100, 500));
        assert_eq!(cycle.step(500, 500), WakeAction::Work);
        assert!(cycle.warmup_complete(500, 500));
    }

    /// F-090-51 and ADR-018 §5, as a property of the type rather than of a
    /// reviewer's attention: there is no path from a watering state into sleep.
    #[test]
    fn no_path_from_a_held_wake_into_sleep() {
        let mut cycle = WakeCycle::new(PowerMode::Battery, WakeReason::Timer, 0);
        cycle.step(0, 0);
        cycle.step(0, 0);
        cycle.announcement_acknowledged();
        let hold = cycle.acquire_hold();
        assert_eq!(cycle.phase(), WakePhase::Holding);
        assert_eq!(cycle.request_sleep(), Err(SleepRefused::HoldOutstanding));
        assert_eq!(cycle.step(999_999, 0), WakeAction::Work);
        drop(hold);
        cycle.sync_holds();
        assert_eq!(cycle.request_sleep(), Ok(()));
        assert_eq!(cycle.step(999_999, 0), WakeAction::Sleep);
    }

    #[test]
    fn nested_holds_do_not_release_each_other() {
        let mut cycle = WakeCycle::new(PowerMode::Battery, WakeReason::Timer, 0);
        cycle.announcement_acknowledged();
        let first = cycle.acquire_hold();
        let second = cycle.acquire_hold();
        drop(second);
        assert!(cycle.is_held());
        assert_eq!(cycle.request_sleep(), Err(SleepRefused::HoldOutstanding));
        drop(first);
        assert!(!cycle.is_held());
        assert_eq!(cycle.request_sleep(), Ok(()));
    }

    /// F-090-52: the announcement's PUBACK is observed *before* sleep is
    /// entered, so a device cannot vanish having told nobody.
    #[test]
    fn sleep_is_refused_until_the_announcement_is_acknowledged() {
        let mut cycle = WakeCycle::new(PowerMode::Battery, WakeReason::Timer, 0);
        assert_eq!(
            cycle.request_sleep(),
            Err(SleepRefused::AnnouncementUnacknowledged)
        );
        cycle.announcement_acknowledged();
        assert_eq!(cycle.request_sleep(), Ok(()));
    }

    #[test]
    fn the_idle_budget_never_truncates_a_held_wake() {
        let mut cycle = WakeCycle::new(PowerMode::Battery, WakeReason::Timer, 0);
        assert!(cycle.idle_budget_spent(30_000, 30));
        let _hold = cycle.acquire_hold();
        assert!(!cycle.idle_budget_spent(3_600_000, 30));
    }

    #[test]
    fn the_rtc_checksum_detects_a_single_corrupted_word() {
        let sealed = RtcSleepState::seal(1_000, 4, 900_000);
        assert!(sealed.checksum_valid());
        for corrupt in [
            RtcSleepState {
                slept_at_ms: 1_001,
                ..sealed
            },
            RtcSleepState {
                boot_generation: 5,
                ..sealed
            },
            RtcSleepState {
                cooldown_remaining_ms: 0,
                ..sealed
            },
            RtcSleepState {
                checksum: sealed.checksum ^ 1,
                ..sealed
            },
        ] {
            assert!(!corrupt.checksum_valid(), "{corrupt:?}");
        }
    }
}
