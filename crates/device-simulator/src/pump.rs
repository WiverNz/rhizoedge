//! The pump, as a mechanism.
//!
//! Runs for a bounded time and moves water. It contains **no rules about
//! whether a dose is allowed**: that decision belongs entirely to
//! `rhizo_mqtt_contract::validate_water_command`, and duplicating any part of it
//! here would be the second implementation ADR-008 exists to prevent.
//!
//! # The run guard is independent
//!
//! A separate timer stops the pump even if the normal completion path does not.
//! In firmware (M11-002) that is a distinct task; here it is a check on every
//! step. The `pump-stuck-on` fault exists to prove the guard is genuinely
//! independent — a guard implemented inside the thing it guards would fail that
//! test, which is the point of having it.

use rhizo_mqtt_contract::CommandId;
use rhizo_mqtt_contract::safety::FIRMWARE_MAX_RUN_SECONDS;

/// The hard ceiling on any single run, from the shared constants.
pub const MAX_RUN_MS: u32 = FIRMWARE_MAX_RUN_SECONDS * 1000;

/// What the pump is doing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PumpState {
    /// De-energised.
    Idle,
    /// Running for a command.
    Running {
        /// Which command it is running for.
        command_id: CommandId,
        /// How long it was authorised to run.
        run_ms: u32,
        /// How much of that has elapsed.
        elapsed_ms: u32,
        /// The authorised volume.
        effective_ml: f32,
    },
}

/// What a step of the pump produced.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PumpStep {
    /// Still running.
    Running,
    /// Finished normally.
    Finished {
        /// The command it was running for.
        command_id: CommandId,
        /// Actual run duration.
        duration_ms: u32,
        /// The authorised volume, to be delivered by the caller.
        effective_ml: f32,
    },
    /// The independent run guard stopped a pump that would not de-energise.
    GuardTripped {
        /// The command it was running for.
        command_id: CommandId,
        /// How long it actually ran before the guard cut it.
        duration_ms: u32,
        /// The authorised volume.
        effective_ml: f32,
    },
    /// Nothing to do.
    Idle,
}

/// The pump.
#[derive(Clone, Debug)]
pub struct Pump {
    state: PumpState,
    /// Set by the `pump-stuck-on` fault: the pump refuses to de-energise on
    /// its own and only the independent guard stops it.
    stuck_on: bool,
}

impl Default for Pump {
    fn default() -> Self {
        Self::new()
    }
}

impl Pump {
    /// A de-energised pump.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            state: PumpState::Idle,
            stuck_on: false,
        }
    }

    /// The current state.
    #[must_use]
    pub const fn state(&self) -> PumpState {
        self.state
    }

    /// Whether the pump is energised.
    #[must_use]
    pub const fn is_running(&self) -> bool {
        matches!(self.state, PumpState::Running { .. })
    }

    /// Makes the pump fail to de-energise on its own.
    pub const fn set_stuck_on(&mut self, stuck: bool) {
        self.stuck_on = stuck;
    }

    /// Starts a run that the validator has already authorised.
    ///
    /// `run_ms` is clamped to [`MAX_RUN_MS`] as a last line of defence. The
    /// validator already clamped it; a mechanism that also refuses to exceed its
    /// own physical limit is how a hardware limit behaves, and it costs one
    /// comparison.
    pub fn start(&mut self, command_id: CommandId, run_ms: u32, effective_ml: f32) {
        self.state = PumpState::Running {
            command_id,
            run_ms: run_ms.min(MAX_RUN_MS),
            elapsed_ms: 0,
            effective_ml,
        };
    }

    /// Advances the run.
    pub fn step(&mut self, dt_ms: u64) -> PumpStep {
        let PumpState::Running {
            command_id,
            run_ms,
            elapsed_ms,
            effective_ml,
        } = self.state
        else {
            return PumpStep::Idle;
        };
        let elapsed = elapsed_ms.saturating_add(u32::try_from(dt_ms).unwrap_or(u32::MAX));

        // The independent guard, checked before the normal completion path so
        // it cannot be short-circuited by it.
        if elapsed >= MAX_RUN_MS && self.stuck_on {
            self.state = PumpState::Idle;
            self.stuck_on = false;
            return PumpStep::GuardTripped {
                command_id,
                duration_ms: elapsed.min(MAX_RUN_MS),
                effective_ml,
            };
        }

        if elapsed >= run_ms && !self.stuck_on {
            self.state = PumpState::Idle;
            return PumpStep::Finished {
                command_id,
                duration_ms: run_ms,
                effective_ml,
            };
        }

        self.state = PumpState::Running {
            command_id,
            run_ms,
            elapsed_ms: elapsed,
            effective_ml,
        };
        PumpStep::Running
    }

    /// De-energises the pump without reporting completion.
    ///
    /// Used when a run is abandoned — a restart, or a shutdown mid-dose. A boot
    /// always begins with the pump off (SAFETY-011).
    pub const fn stop(&mut self) {
        self.state = PumpState::Idle;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn command_id() -> CommandId {
        CommandId::from_uuid(Uuid::from_u128(1))
    }

    #[test]
    fn a_new_pump_is_off_and_does_nothing() {
        let mut pump = Pump::new();
        assert_eq!(pump.state(), PumpState::Idle);
        assert!(!pump.is_running());
        assert_eq!(pump.step(10_000), PumpStep::Idle);
    }

    #[test]
    fn a_run_finishes_after_its_authorised_duration() {
        let mut pump = Pump::new();
        pump.start(command_id(), 4_878, 40.0);
        assert!(pump.is_running());
        assert_eq!(pump.step(4_000), PumpStep::Running);
        assert_eq!(
            pump.step(878),
            PumpStep::Finished {
                command_id: command_id(),
                duration_ms: 4_878,
                effective_ml: 40.0
            }
        );
        assert!(!pump.is_running(), "and the pump de-energises");
        assert_eq!(pump.step(1_000), PumpStep::Idle);
    }

    #[test]
    fn a_run_can_never_be_authorised_beyond_the_hard_maximum() {
        let mut pump = Pump::new();
        pump.start(command_id(), u32::MAX, 40.0);
        let PumpState::Running { run_ms, .. } = pump.state() else {
            panic!("the pump must be running");
        };
        assert_eq!(run_ms, MAX_RUN_MS);
        assert_eq!(MAX_RUN_MS, 20_000);
    }

    #[test]
    fn the_independent_guard_stops_a_pump_that_will_not_de_energise() {
        let mut pump = Pump::new();
        pump.start(command_id(), 4_000, 40.0);
        pump.set_stuck_on(true);

        // Past its own duration, and still running: the normal path failed.
        assert_eq!(pump.step(5_000), PumpStep::Running);
        assert!(pump.is_running(), "the fault is doing what it claims");

        // The guard cuts it at the hard maximum, whatever the run asked for.
        assert_eq!(
            pump.step(MAX_RUN_MS as u64),
            PumpStep::GuardTripped {
                command_id: command_id(),
                duration_ms: MAX_RUN_MS,
                effective_ml: 40.0
            }
        );
        assert!(
            !pump.is_running(),
            "something else has to stop it, and does"
        );
    }

    #[test]
    fn stopping_abandons_a_run_without_reporting_completion() {
        let mut pump = Pump::new();
        pump.start(command_id(), 4_000, 40.0);
        pump.step(1_000);
        pump.stop();
        assert!(
            !pump.is_running(),
            "a boot always begins with the pump off (SAFETY-011)"
        );
        assert_eq!(pump.step(10_000), PumpStep::Idle);
    }

    #[test]
    fn an_enormous_step_completes_rather_than_overflowing() {
        let mut pump = Pump::new();
        pump.start(command_id(), 4_000, 40.0);
        assert!(matches!(
            pump.step(u64::MAX),
            PumpStep::Finished {
                duration_ms: 4_000,
                ..
            }
        ));
    }
}
