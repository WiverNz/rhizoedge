//! Peripheral power gating and sensor warm-up (M9-020).
//!
//! A device that sleeps for fourteen minutes out of every fifteen has achieved
//! nothing if its RS485 transceiver and soil probe stay powered the whole time.
//! They are needed for a few hundred milliseconds per cycle.
//!
//! Powering a sensor also means it is cold when it is first asked for a
//! reading, and **a reading taken before a sensor has settled is not a bad
//! reading — it is a plausible one**, which is worse. So a read attempted
//! before `sensor_warmup_ms` has elapsed raises `sensor_warmup_incomplete` and
//! publishes no sample, rather than publishing a value nobody can tell is wrong.
//!
//! # Rails are the board's business
//!
//! Which rails exist, on which pins, at which polarity, is supplied by the
//! board profile. A load switch that is active-low on one board and active-high
//! on another is a board fact, and getting it backwards powers a rail through a
//! whole sleep. Nothing in this module names a pin or an active level.
//!
//! # The ordering trade-off, stated rather than argued about
//!
//! Enabling the rail before bringing up Wi-Fi lets association and DHCP run
//! during the warm-up window instead of after it — the same wall time, less of
//! it awake. It also draws the sensor rail and the radio concurrently. Which
//! ordering wins is an energy question M10-012 settles with a meter, not a
//! preference to be settled now.
//!
//! # Always-on leaves the rails on
//!
//! Gating is a battery-mode behaviour and must not become a new failure mode
//! for a mains device (F-090-61).

use rhizo_mqtt_contract::payload::PowerMode;

use crate::ports::PowerRail;

/// The device's switched supplies.
///
/// Each is optional because a board that has no RS485 transceiver says so in
/// its own profile, and the sampling code asks the board rather than assuming.
/// This is the first place the board seam earns its keep: a DEVKITM-1's header
/// and a XIAO's much smaller pin budget will not agree on which rails exist,
/// and that disagreement must stay inside `src/board/`.
pub struct Rails<'a> {
    /// The sensor supply.
    pub sensor: Option<&'a mut dyn PowerRail>,
    /// The RS485 transceiver supply.
    pub rs485: Option<&'a mut dyn PowerRail>,
}

impl Rails<'_> {
    /// Powers every rail this board has.
    pub fn enable(&mut self) {
        if let Some(rail) = self.sensor.as_mut() {
            rail.enable();
        }
        if let Some(rail) = self.rs485.as_mut() {
            rail.enable();
        }
    }

    /// Removes power from every rail this board has.
    pub fn disable(&mut self) {
        if let Some(rail) = self.sensor.as_mut() {
            rail.disable();
        }
        if let Some(rail) = self.rs485.as_mut() {
            rail.disable();
        }
    }

    /// Whether any rail is currently powered.
    #[must_use]
    pub fn any_enabled(&self) -> bool {
        self.sensor.as_ref().is_some_and(|r| r.is_enabled())
            || self.rs485.as_ref().is_some_and(|r| r.is_enabled())
    }
}

/// Holds the rails on for the life of the guard.
///
/// `disable()` lives in `Drop` so an error path — a failed read, an early
/// return, a `?` three frames up — cannot leave a rail powered through a
/// fifteen-minute sleep.
pub struct RailGuard<'a, 'b> {
    rails: &'a mut Rails<'b>,
    gate: bool,
}

impl<'a, 'b> RailGuard<'a, 'b> {
    /// Powers the rails for a sampling cycle.
    ///
    /// In [`PowerMode::AlwaysOn`] the rails are enabled and **left enabled**
    /// when the guard drops, so always-on behaviour is unchanged.
    pub fn acquire(rails: &'a mut Rails<'b>, mode: PowerMode) -> Self {
        rails.enable();
        Self {
            rails,
            gate: mode.effective() == PowerMode::Battery,
        }
    }

    /// The rails this guard holds.
    pub fn rails(&mut self) -> &mut Rails<'b> {
        self.rails
    }
}

impl Drop for RailGuard<'_, '_> {
    fn drop(&mut self) {
        if self.gate {
            self.rails.disable();
        }
    }
}

/// Whether a sample may be taken yet.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WarmupState {
    /// The rail has been powered for at least `sensor_warmup_ms`.
    Ready,
    /// Not yet. A read now would produce a plausible wrong value.
    Incomplete {
        /// How much longer to wait.
        remaining_ms: u64,
    },
}

/// Whether the warm-up delay has elapsed.
///
/// `enabled_at_ms` is `None` when the rail has never been powered, which is
/// never "ready": a sensor that was never given power has certainly not
/// settled.
#[must_use]
pub fn warmup_state(
    mode: PowerMode,
    enabled_at_ms: Option<u64>,
    monotonic_ms: u64,
    warmup_ms: u32,
) -> WarmupState {
    if mode.effective() == PowerMode::AlwaysOn {
        return WarmupState::Ready;
    }
    match enabled_at_ms {
        Some(at) => {
            let elapsed = monotonic_ms.saturating_sub(at);
            if elapsed >= u64::from(warmup_ms) {
                WarmupState::Ready
            } else {
                WarmupState::Incomplete {
                    remaining_ms: u64::from(warmup_ms) - elapsed,
                }
            }
        }
        None => WarmupState::Incomplete {
            remaining_ms: u64::from(warmup_ms),
        },
    }
}

/// The device event raised when a read is attempted too early.
///
/// A misconfiguration must be visible rather than silent: without this, a
/// warm-up set too short looks like a sensor that reads slightly wrong.
pub const SENSOR_WARMUP_INCOMPLETE: &str = "sensor_warmup_incomplete";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fakes::{FakeRail, call_log};

    #[test]
    fn a_battery_cycle_powers_the_rails_and_the_guard_turns_them_off() {
        let log = call_log();
        let mut sensor = FakeRail::new(log.clone(), "sensor");
        let mut rs485 = FakeRail::new(log.clone(), "rs485");
        let mut rails = Rails {
            sensor: Some(&mut sensor),
            rs485: Some(&mut rs485),
        };
        {
            let mut guard = RailGuard::acquire(&mut rails, PowerMode::Battery);
            assert!(guard.rails().any_enabled());
        }
        assert!(!rails.any_enabled(), "both rails are off before sleep");
        assert_eq!(
            log.borrow().clone(),
            vec![
                "rail_enable(sensor)".to_owned(),
                "rail_enable(rs485)".to_owned(),
                "rail_disable(sensor)".to_owned(),
                "rail_disable(rs485)".to_owned(),
            ]
        );
    }

    /// The guard is the reason an error path cannot leave a rail powered for a
    /// fortnight.
    #[test]
    fn an_error_during_sampling_still_disables_both_rails() {
        let log = call_log();
        let mut sensor = FakeRail::new(log.clone(), "sensor");
        let mut rails = Rails {
            sensor: Some(&mut sensor),
            rs485: None,
        };
        fn sample_and_fail(rails: &mut Rails<'_>) -> Result<(), &'static str> {
            let _guard = RailGuard::acquire(rails, PowerMode::Battery);
            Err("bus error")
        }
        assert!(sample_and_fail(&mut rails).is_err());
        assert!(!rails.any_enabled());
    }

    #[test]
    fn always_on_leaves_the_rails_enabled() {
        let log = call_log();
        let mut sensor = FakeRail::new(log.clone(), "sensor");
        let mut rails = Rails {
            sensor: Some(&mut sensor),
            rs485: None,
        };
        {
            let _guard = RailGuard::acquire(&mut rails, PowerMode::AlwaysOn);
        }
        assert!(rails.any_enabled(), "gating is a battery-mode behaviour");
    }

    #[test]
    fn a_board_with_no_rs485_rail_never_enables_one() {
        let log = call_log();
        let mut sensor = FakeRail::new(log.clone(), "sensor");
        let mut rails = Rails {
            sensor: Some(&mut sensor),
            rs485: None,
        };
        rails.enable();
        assert!(
            !log.borrow().iter().any(|call| call.contains("rs485")),
            "a rail the board does not have is never touched"
        );
    }

    #[test]
    fn a_read_before_the_warm_up_has_elapsed_is_incomplete() {
        assert_eq!(
            warmup_state(PowerMode::Battery, Some(1_000), 1_400, 500),
            WarmupState::Incomplete { remaining_ms: 100 }
        );
        assert_eq!(
            warmup_state(PowerMode::Battery, Some(1_000), 1_500, 500),
            WarmupState::Ready
        );
    }

    #[test]
    fn a_rail_that_was_never_powered_is_never_ready() {
        assert_eq!(
            warmup_state(PowerMode::Battery, None, 999_999, 500),
            WarmupState::Incomplete { remaining_ms: 500 }
        );
    }

    #[test]
    fn always_on_never_reports_an_incomplete_warm_up() {
        assert_eq!(
            warmup_state(PowerMode::AlwaysOn, None, 0, 60_000),
            WarmupState::Ready
        );
    }
}
