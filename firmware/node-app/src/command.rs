//! Command handling: the device's one and only actuation path (M9-011).
//!
//! # One gate
//!
//! [`rhizo_mqtt_contract::validate_water_command`] is called from exactly one
//! place in this crate, and that call is inside [`authorise`]. Every dose the
//! device delivers — commanded or autonomous — passes through
//! [`execute_authorised`], which is the only function that touches the pump.
//! A second implementation of the rules would make every simulator-based safety
//! test worthless (ADR-008, SAFETY-007).
//!
//! # The ordering that makes an interruption detectable
//!
//! ```text
//! gate accepts -> ledger has room -> NVS records the dose -> pump runs
//! ```
//!
//! The NVS write is **before** actuation, and **if it fails the dose is
//! aborted** and reported `failed`. Stated plainly: if the device cannot record
//! that it is about to pump, it must not pump. Otherwise an interrupted dose
//! becomes undetectable and SAFETY-011 has nothing to detect.
//!
//! # Step 13a
//!
//! The pending-result ledger check sits between the gate and the persist, as an
//! additional veto after an acceptance. It reads no gate step and satisfies
//! none; it can only ever stop a dose. See [`crate::ledger`] for the decision
//! and its reasoning.

use rhizo_mqtt_contract::{
    CommandId, UtcMillis,
    payload::{CommandOrigin, CommandResult, CommandStatus, RejectReason, WaterCommand},
    safety::{
        CommandVerdict, DeviceGuardState, LeakState, PreviousCommand, validate_water_command,
    },
};

use crate::dedup;
use crate::persist::{InFlightDose, PersistedState};
use crate::ports::{NvsStore, Pump};

/// The device-side inputs the gate needs, gathered from sensors and config.
#[derive(Clone, Copy, Debug)]
pub struct GateInputs {
    /// Whether the wall clock is synchronised and not aged out.
    pub clock_synced: bool,
    /// Device wall time.
    pub now_ms: UtcMillis,
    /// Leak state; `Unknown` for an unreadable sensor and for no sensor at all.
    pub leak: LeakState,
    /// Reservoir level.
    pub tank_percent: Option<f32>,
    /// Configured reservoir minimum.
    pub tank_min_percent: f32,
    /// Operational pump enable from configuration.
    pub pump_enabled: bool,
    /// Whether the driver has latched a fault.
    pub pump_faulted: bool,
    /// Pump calibration.
    pub pump_ml_per_second: f32,
}

/// What the gate decided, plus the device-local veto.
#[derive(Clone, Debug, PartialEq)]
pub enum Authorisation {
    /// A bounded dose the device may deliver.
    Dose {
        /// Volume after clamping.
        effective_ml: f32,
        /// Bounded run duration.
        run_ms: u32,
        /// Whether a hard limit changed the request.
        clamped: bool,
    },
    /// A refusal, with the reason the edge will see.
    Refused(RejectReason),
    /// This `command_id` has already been executed; republish the stored result.
    AlreadyExecuted(CommandResult),
}

/// Runs the shared gate and the device-local ledger veto.
///
/// **The one call site of `validate_water_command` in this crate.**
#[must_use]
pub fn authorise(
    state: &PersistedState,
    command: &WaterCommand,
    inputs: &GateInputs,
) -> Authorisation {
    let previous: Vec<PreviousCommand<'_>> = state
        .dedup_ring
        .iter()
        .map(|entry| PreviousCommand {
            command_id: entry.command_id,
            result: &entry.result,
        })
        .collect();

    let guard = DeviceGuardState {
        previous: &previous,
        clock_synced: inputs.clock_synced,
        now_ms: inputs.now_ms,
        leak: inputs.leak,
        tank_percent: inputs.tank_percent,
        tank_min_percent: inputs.tank_min_percent,
        pump_enabled: inputs.pump_enabled,
        pump_faulted: inputs.pump_faulted,
        pump_ml_per_second: inputs.pump_ml_per_second,
        delivered_today_ml: state.daily.delivered_ml,
    };

    match validate_water_command(command, &guard) {
        CommandVerdict::AlreadyExecuted { previous } => {
            Authorisation::AlreadyExecuted(previous.clone())
        }
        CommandVerdict::Reject(reason) => Authorisation::Refused(reason),
        CommandVerdict::Accept {
            effective_ml,
            run_ms,
            clamped,
        } => {
            // Step 13a. After the gate, never instead of it.
            if !state.pending_results.permits_actuation() {
                return Authorisation::Refused(RejectReason::ResultLedgerFull);
            }
            Authorisation::Dose {
                effective_ml,
                run_ms,
                clamped,
            }
        }
    }
}

/// Why a dose did not happen after it had been authorised.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExecutionFault {
    /// The pre-actuation NVS write did not commit; nothing was pumped.
    NvsWriteFailed,
    /// The pump driver refused or failed.
    PumpFailed,
}

/// One dose the gate has already authorised.
///
/// A struct rather than eight positional parameters, because two of them are
/// volumes and two are booleans: `execute_authorised(.., 40.0, 38.0, .., true)`
/// is the shape of call that eventually gets its arguments swapped, and the
/// arguments here are how much water moves.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AuthorisedDose {
    /// The identity the dose is recorded under.
    pub command_id: CommandId,
    /// What was asked for.
    pub requested_ml: f32,
    /// What the hard limits allow.
    pub effective_ml: f32,
    /// The bounded run duration.
    pub run_ms: u32,
    /// Device wall time at start, where known.
    pub started_at_ms: Option<UtcMillis>,
    /// Whether the device authorised it itself while isolated.
    pub autonomous: bool,
}

/// Delivers an authorised dose. **The only function that runs the pump.**
///
/// The dose record is written to NVS before the pump is energised and cleared
/// only after the result is durably ledgered, so a reset at any point in
/// between leaves evidence for [`crate::recovery`] to find.
pub fn execute_authorised(
    state: &mut PersistedState,
    nvs: &mut impl NvsStore,
    pump: &mut impl Pump,
    dose: &AuthorisedDose,
) -> Result<f32, ExecutionFault> {
    let &AuthorisedDose {
        command_id,
        requested_ml,
        effective_ml,
        run_ms,
        started_at_ms,
        autonomous,
    } = dose;
    state.in_flight_dose = Some(InFlightDose {
        command_id,
        started_at_ms,
        requested_ml,
        autonomous,
    });
    if nvs.store(state).is_err() {
        // The record did not commit, so an interruption during this dose would
        // be invisible. Abort before anything moves.
        state.in_flight_dose = None;
        return Err(ExecutionFault::NvsWriteFailed);
    }

    let outcome = pump.run_for(run_ms);
    pump.off();

    match outcome {
        Ok(()) => {
            state.daily.delivered_ml += effective_ml;
            state.in_flight_dose = None;
            Ok(effective_ml)
        }
        Err(_) => {
            // A pump that refused delivered nothing, but the device cannot
            // prove that, so the in-flight record is cleared only because the
            // failure is being reported in the same breath. The volume is not
            // credited: crediting water that may not have moved is the
            // direction that under-waters, which is the safe one here because
            // the edge is told the dose failed.
            state.in_flight_dose = None;
            Err(ExecutionFault::PumpFailed)
        }
    }
}

/// Builds the `command.result` for an outcome.
#[must_use]
pub fn result_for(
    command_id: CommandId,
    requested_ml: f32,
    delivered_today_ml: f32,
    origin: CommandOrigin,
    outcome: ResultOutcome,
) -> CommandResult {
    let (status, delivered_ml, duration_ms, clamped, reason) = match outcome {
        ResultOutcome::Completed {
            delivered_ml,
            duration_ms,
            clamped,
        } => (
            CommandStatus::Completed,
            Some(delivered_ml),
            Some(duration_ms),
            clamped,
            None,
        ),
        // `None`, not `Some(0.0)`. The reference simulator publishes null here
        // and protocol §5.10's table says a rejection credits no volume at all,
        // so the field has nothing to report rather than a measured zero. The
        // conformance test found this: an early draft published `Some(0.0)`,
        // which decodes and reads plausibly and would have made every rejection
        // differ from the simulator's in a way no type would have caught.
        ResultOutcome::Rejected(reason) => {
            (CommandStatus::Rejected, None, None, false, Some(reason))
        }
        // `delivered_ml: None` means genuinely unknown. Reporting 0.0 would let
        // the edge grant the full budget again; reporting the requested volume
        // would be a guess. Null lets the edge apply its own conservative
        // policy (M9-013, M6-010).
        ResultOutcome::Interrupted => (CommandStatus::Interrupted, None, None, false, None),
        ResultOutcome::Failed => (CommandStatus::Failed, None, None, false, None),
    };
    CommandResult {
        command_id,
        status,
        requested_ml,
        delivered_ml,
        duration_ms,
        clamped,
        reason,
        delivered_today_ml,
        origin,
        detail: None,
    }
}

/// The shape of a completed command.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ResultOutcome {
    /// The dose ran.
    Completed {
        /// Volume delivered.
        delivered_ml: f32,
        /// Run duration.
        duration_ms: u32,
        /// Whether a hard limit changed the request.
        clamped: bool,
    },
    /// The gate or the ledger refused.
    Rejected(RejectReason),
    /// A reset happened while the dose was in flight.
    Interrupted,
    /// The device could not carry out an authorised dose.
    Failed,
}

/// What handling a command produced.
#[derive(Clone, Debug, PartialEq)]
pub struct Handled {
    /// The result to publish.
    pub result: CommandResult,
    /// Whether the pump actually ran.
    pub actuated: bool,
    /// Whether the ledger refused the result a durable slot.
    ///
    /// True only for a rejection issued while the ledger is completely full. A
    /// rejection reports zero delivered water, so publishing it once
    /// un-ledgered cannot under-count anything; the saturation audit event is
    /// what carries the condition durably.
    pub unledgered: bool,
    /// Whether the ledger crossed into saturation and a fault should be emitted.
    pub saturation_raised: bool,
}

/// Handles one inbound `command.water` end to end.
pub fn handle_water(
    state: &mut PersistedState,
    nvs: &mut impl NvsStore,
    pump: &mut impl Pump,
    command: &WaterCommand,
    inputs: &GateInputs,
) -> Handled {
    match authorise(state, command, inputs) {
        Authorisation::AlreadyExecuted(previous) => {
            // Re-publish the stored result and do **not** actuate. The stored
            // one, not a freshly computed one: the edge asked what happened,
            // not what would happen now.
            Handled {
                result: previous,
                actuated: false,
                unledgered: false,
                saturation_raised: false,
            }
        }
        Authorisation::Refused(reason) => {
            let result = result_for(
                command.command_id,
                command.requested_ml,
                state.daily.delivered_ml,
                CommandOrigin::EdgeCommand,
                ResultOutcome::Rejected(reason),
            );
            settle(state, nvs, result, false)
        }
        Authorisation::Dose {
            effective_ml,
            run_ms,
            clamped,
        } => {
            let started_at_ms = inputs.clock_synced.then_some(inputs.now_ms);
            let outcome = execute_authorised(
                state,
                nvs,
                pump,
                &AuthorisedDose {
                    command_id: command.command_id,
                    requested_ml: command.requested_ml,
                    effective_ml,
                    run_ms,
                    started_at_ms,
                    autonomous: false,
                },
            );
            let (result_outcome, actuated) = match outcome {
                Ok(delivered_ml) => (
                    ResultOutcome::Completed {
                        delivered_ml,
                        duration_ms: run_ms,
                        clamped,
                    },
                    true,
                ),
                Err(ExecutionFault::NvsWriteFailed) => (ResultOutcome::Failed, false),
                Err(ExecutionFault::PumpFailed) => (ResultOutcome::Failed, true),
            };
            let result = result_for(
                command.command_id,
                command.requested_ml,
                state.daily.delivered_ml,
                CommandOrigin::EdgeCommand,
                result_outcome,
            );
            settle(state, nvs, result, actuated)
        }
    }
}

/// Records a result in the dedup ring and the ledger, and commits.
fn settle(
    state: &mut PersistedState,
    nvs: &mut impl NvsStore,
    result: CommandResult,
    actuated: bool,
) -> Handled {
    dedup::record(&mut state.dedup_ring, result.command_id, result.clone());
    let unledgered = state.pending_results.insert(result.clone()).is_err();
    let saturation_raised = state.pending_results.raise_fault_if_crossed();
    // A commit failure here does not undo the dose; it means the device will
    // republish from an older ledger after a reset, which the edge deduplicates
    // on `command_id`. Losing the write is the safe direction: the result is
    // sent again rather than forgotten.
    let _ = nvs.store(state);
    Handled {
        result,
        actuated,
        unledgered,
        saturation_raised,
    }
}

/// Applies a `command.result.ack` (protocol §5.14).
///
/// Returns whether the saturation fault cleared as a result. An acknowledgement
/// **must not** affect the dedup ring: an acknowledged result is still a
/// command the device has executed, and forgetting that would let a repeat
/// actuate.
pub fn acknowledge_result(state: &mut PersistedState, command_id: CommandId) -> bool {
    let ring_before = state.dedup_ring.len();
    state.pending_results.acknowledge(command_id);
    let cleared = state.pending_results.clear_fault_if_crossed();
    debug_assert_eq!(ring_before, state.dedup_ring.len());
    cleared
}

/// How many call sites of the shared gate exist in this crate.
///
/// Asserted by `tests/single_actuation_path.rs` against the source text. One,
/// and it is in [`authorise`].
pub const EXPECTED_GATE_CALL_SITES: usize = 1;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fakes::{FakeNvs, FakePump, call_log};
    use crate::ledger::ACTUATION_THRESHOLD;
    use uuid::Uuid;

    fn command(n: u128, requested_ml: f32) -> WaterCommand {
        WaterCommand {
            command_id: CommandId::from_uuid(Uuid::from_u128(n)),
            requested_ml,
            issued_at_ms: UtcMillis(1_000),
            expires_at_ms: UtcMillis(121_000),
        }
    }

    fn inputs() -> GateInputs {
        GateInputs {
            clock_synced: true,
            now_ms: UtcMillis(2_000),
            leak: LeakState::Clear,
            tank_percent: Some(60.0),
            tank_min_percent: 15.0,
            pump_enabled: true,
            pump_faulted: false,
            pump_ml_per_second: 8.0,
        }
    }

    fn rig() -> (PersistedState, FakeNvs, FakePump) {
        let log = call_log();
        (
            PersistedState::default(),
            FakeNvs::new(),
            FakePump::new(log),
        )
    }

    #[test]
    fn a_valid_command_actuates_and_reports_completed() {
        let (mut state, mut nvs, mut pump) = rig();
        let handled = handle_water(
            &mut state,
            &mut nvs,
            &mut pump,
            &command(1, 40.0),
            &inputs(),
        );
        assert!(handled.actuated);
        assert_eq!(handled.result.status, CommandStatus::Completed);
        assert_eq!(handled.result.delivered_ml, Some(40.0));
        assert_eq!(handled.result.duration_ms, Some(5_000));
        assert_eq!(state.daily.delivered_ml, 40.0);
        assert!(state.in_flight_dose.is_none());
        assert_eq!(state.pending_results.len(), 1);
    }

    /// SAFETY-001, and the half of it that needs NVS: the ring is reloaded from
    /// storage and the repeat is recognised across the power cycle.
    #[test]
    fn safety_001_a_duplicate_command_republishes_and_does_not_actuate() {
        let (mut state, mut nvs, mut pump) = rig();
        handle_water(
            &mut state,
            &mut nvs,
            &mut pump,
            &command(1, 40.0),
            &inputs(),
        );
        let runs_before = pump.total_run_ms;

        let rebooted = nvs.power_cycle();
        let mut state = rebooted.load().expect("state survived the power cycle");
        let mut nvs = rebooted;

        let handled = handle_water(
            &mut state,
            &mut nvs,
            &mut pump,
            &command(1, 40.0),
            &inputs(),
        );
        assert!(!handled.actuated);
        assert_eq!(pump.total_run_ms, runs_before, "the pump did not run again");
        assert_eq!(handled.result.status, CommandStatus::Completed);
        assert_eq!(state.daily.delivered_ml, 40.0, "the total did not double");
    }

    /// If the device cannot record that it is about to pump, it must not pump.
    #[test]
    fn safety_011_a_failed_nvs_write_aborts_the_dose() {
        let (mut state, mut nvs, mut pump) = rig();
        nvs.fail_writes(true);
        let handled = handle_water(
            &mut state,
            &mut nvs,
            &mut pump,
            &command(1, 40.0),
            &inputs(),
        );
        assert!(!handled.actuated);
        assert_eq!(handled.result.status, CommandStatus::Failed);
        assert_eq!(handled.result.delivered_ml, None);
        assert_eq!(pump.total_run_ms, 0, "nothing was pumped");
        assert_eq!(state.daily.delivered_ml, 0.0);
        assert!(state.in_flight_dose.is_none());
    }

    #[test]
    fn safety_002_a_command_is_refused_while_the_clock_is_unsynced() {
        let (mut state, mut nvs, mut pump) = rig();
        let mut unsynced = inputs();
        unsynced.clock_synced = false;
        let handled = handle_water(
            &mut state,
            &mut nvs,
            &mut pump,
            &command(1, 40.0),
            &unsynced,
        );
        assert_eq!(handled.result.reason, Some(RejectReason::ClockUnsynced));
        assert_eq!(pump.total_run_ms, 0);
    }

    #[test]
    fn safety_007_an_oversized_command_is_clamped_to_the_hard_limit() {
        let (mut state, mut nvs, mut pump) = rig();
        let handled = handle_water(
            &mut state,
            &mut nvs,
            &mut pump,
            &command(1, 5_000.0),
            &inputs(),
        );
        assert!(handled.result.clamped);
        assert_eq!(
            handled.result.delivered_ml,
            Some(rhizo_mqtt_contract::safety::FIRMWARE_MAX_ML_PER_RUN)
        );
    }

    #[test]
    fn safety_007_the_device_daily_cap_is_enforced_independently_of_the_edge() {
        let (mut state, mut nvs, mut pump) = rig();
        state.daily.delivered_ml = 480.0;
        let handled = handle_water(
            &mut state,
            &mut nvs,
            &mut pump,
            &command(1, 40.0),
            &inputs(),
        );
        assert_eq!(handled.result.reason, Some(RejectReason::OverDailyMax));
        assert_eq!(pump.total_run_ms, 0);
    }

    /// Step 13a: the ledger veto turns an acceptance into a refusal, and the
    /// reason names the real condition rather than blaming the pump.
    #[test]
    fn a_saturated_ledger_refuses_actuation_with_result_ledger_full() {
        let (mut state, mut nvs, mut pump) = rig();
        for n in 100..(100 + ACTUATION_THRESHOLD as u128) {
            let result = result_for(
                CommandId::from_uuid(Uuid::from_u128(n)),
                10.0,
                0.0,
                CommandOrigin::EdgeCommand,
                ResultOutcome::Completed {
                    delivered_ml: 10.0,
                    duration_ms: 1_000,
                    clamped: false,
                },
            );
            state
                .pending_results
                .insert(result)
                .expect("within capacity");
        }
        let handled = handle_water(
            &mut state,
            &mut nvs,
            &mut pump,
            &command(1, 40.0),
            &inputs(),
        );
        assert_eq!(handled.result.reason, Some(RejectReason::ResultLedgerFull));
        assert_eq!(handled.result.status, CommandStatus::Rejected);
        assert!(handled.saturation_raised, "the fault is emitted once");
        assert_eq!(pump.total_run_ms, 0);
        assert!(!handled.unledgered, "the reserved slot held the refusal");
    }

    #[test]
    fn acknowledgement_frees_the_ledger_without_touching_the_dedup_ring() {
        let (mut state, mut nvs, mut pump) = rig();
        handle_water(
            &mut state,
            &mut nvs,
            &mut pump,
            &command(1, 40.0),
            &inputs(),
        );
        assert_eq!(state.pending_results.len(), 1);
        assert_eq!(state.dedup_ring.len(), 1);

        acknowledge_result(&mut state, CommandId::from_uuid(Uuid::from_u128(1)));
        assert_eq!(state.pending_results.len(), 0);
        assert_eq!(
            state.dedup_ring.len(),
            1,
            "an acknowledged result is still a command the device executed"
        );

        // And the repeat still does not actuate.
        let handled = handle_water(
            &mut state,
            &mut nvs,
            &mut pump,
            &command(1, 40.0),
            &inputs(),
        );
        assert!(!handled.actuated);
    }

    #[test]
    fn a_result_is_published_for_every_command_including_refusals() {
        let (mut state, mut nvs, mut pump) = rig();
        let mut leaking = inputs();
        leaking.leak = LeakState::Detected;
        let handled = handle_water(&mut state, &mut nvs, &mut pump, &command(1, 40.0), &leaking);
        assert_eq!(handled.result.status, CommandStatus::Rejected);
        assert_eq!(state.pending_results.len(), 1, "refusals are ledgered too");
    }

    /// A power cycle exactly at the saturation boundary neither drops nor
    /// duplicates a result.
    #[test]
    fn the_ledger_state_at_saturation_survives_a_power_cycle_intact() {
        let (mut state, mut nvs, mut pump) = rig();
        let mut inputs = inputs();
        inputs.pump_ml_per_second = 8.0;
        for n in 1..=(ACTUATION_THRESHOLD as u128) {
            handle_water(&mut state, &mut nvs, &mut pump, &command(n, 10.0), &inputs);
        }
        assert!(!state.pending_results.permits_actuation());
        let unacknowledged = state.pending_results.unacknowledged_ml();

        let rebooted = nvs.power_cycle();
        let restored = rebooted.load().expect("state survived");
        assert_eq!(restored.pending_results.len(), ACTUATION_THRESHOLD);
        assert_eq!(restored.pending_results.unacknowledged_ml(), unacknowledged);
        assert!(!restored.pending_results.permits_actuation());
        assert!(
            restored.pending_results.fault_raised(),
            "the fault is not re-emitted after a reboot"
        );
    }
}
