//! Autonomous watering while isolated — **the one call site** of the shared
//! evaluator in this crate (M9-016, ADR-015, ADR-008).
//!
//! # One evaluator, one actuation path
//!
//! `rhizo_policy::evaluate_offline` is the same function the simulator calls
//! and the Edge validates policies with. A firmware-specific evaluator, even a
//! small one, even a temporary one, would make every offline safety test in M6
//! and every isolation scenario in M8 exercise rules the hardware does not
//! follow. `tests/single_actuation_path.rs` counts both call sites and fails at
//! two.
//!
//! The dose it produces reaches the pump through
//! [`crate::command::execute_authorised`] — the same in-flight NVS write before
//! actuation (SAFETY-011), the same hard limits through the shared
//! [`rhizo_mqtt_contract::safety::bound_dose`] (SAFETY-007, SAFETY-014).
//!
//! # What an autonomous dose does *not* go through, and why
//!
//! `validate_water_command` steps 2 and 3 — `clock_unsynced` and `expired` —
//! are about a **command**: a decision another machine made at a wall-clock
//! instant this device must be able to compare against. An autonomous dose has
//! no issuer, no TTL, and by construction no synchronised clock. SAFETY-015
//! governs the autonomous path and SAFETY-002 the commanded one. Applying the
//! TTL check here would mean an isolated device could never water at all, which
//! is the entire feature.
//!
//! # Elapsed time comes from the monotonic timer, never the wall clock
//!
//! `evaluate_offline` cannot read a clock — the crate has no dependency that
//! could — so the only way to get this wrong is at the call site, which is
//! exactly where the test looks.

use rhizo_mqtt_contract::payload::OfflinePolicy;
use rhizo_mqtt_contract::{
    CommandId, UtcMillis,
    payload::{
        CommandOrigin, EventDetail, EventKind, EventTier, MeasurementKind, MeasurementValue,
        Quality,
    },
    safety::{DoseBound, LeakState, bound_dose},
};
use rhizo_policy::{
    MonotonicMillis, OfflineDecision, OfflineInputs, OfflineSample, RefuseReason, evaluate_offline,
    next_offline_state,
};

use crate::command::{ExecutionFault, ResultOutcome, execute_authorised, result_for};
use crate::persist::PersistedState;
use crate::ports::{NvsStore, Pump};

/// The measurements and vetoes an isolated evaluation needs.
#[derive(Clone, Debug)]
pub struct OfflineSeamInputs {
    /// The control measurement.
    pub control: Option<OfflineSample>,
    /// Every declared required measurement.
    pub required: Vec<OfflineSample>,
    /// Leak state; `None` is not "clear".
    pub leak: Option<LeakState>,
    /// Reservoir level.
    pub tank_percent: Option<f32>,
    /// Pump health.
    pub pump_healthy: Option<bool>,
    /// Pump calibration, for the shared dose bound.
    pub pump_ml_per_second: f32,
}

/// One evaluation of one plant at one instant.
///
/// A struct rather than five positional parameters, because two of them are
/// times with different meanings — `elapsed` is a monotonic **delta** the
/// device observed, `monotonic_ms` is the instant an event is stamped with, and
/// `device_time_ms` is a wall clock that may not exist. Passing them
/// positionally is how the delta and the instant get swapped, and swapping them
/// would hand the evaluator a since-boot number where it expects a credit.
#[derive(Clone, Copy, Debug)]
pub struct OfflineTick<'a> {
    /// The plant whose policy is being evaluated.
    pub plant_id: &'a str,
    /// The measurements and vetoes.
    pub inputs: &'a OfflineSeamInputs,
    /// Monotonic time **observed** since the previous evaluation.
    pub elapsed: MonotonicMillis,
    /// The monotonic instant, for stamping buffered events.
    pub monotonic_ms: u64,
    /// Wall time, where the clock is synchronised.
    pub device_time_ms: Option<UtcMillis>,
}

/// What an autonomous evaluation did.
#[derive(Clone, Debug, PartialEq)]
pub enum AutonomousOutcome {
    /// Nothing to do; the cycle is idle, confirming, absorbing, or cooling.
    Waiting,
    /// A dose was delivered.
    Dosed {
        /// Volume actually delivered.
        delivered_ml: f32,
        /// The policy version that authorised it.
        policy_version: u32,
    },
    /// The dose was refused, with the shared evaluator's reason.
    Refused(RefuseReason),
    /// The evaluator authorised a dose the hard limits then refused.
    BoundRefused(rhizo_mqtt_contract::payload::RejectReason),
    /// The device could not carry the dose out.
    Failed(ExecutionFault),
    /// No valid policy, so nothing to evaluate. **Absence is not permission.**
    NoValidPolicy,
}

/// Evaluates offline autonomy for one plant and acts on the answer.
///
/// `elapsed` is the monotonic time the device actually **observed** — a reboot
/// credits zero, which is what stops a reboot loop earning water.
///
/// `mint_command_id` supplies the identity an autonomous dose is recorded
/// under. It is a real `command_id` so the dose flows through the same dedup
/// ring and the same in-flight record a commanded dose does.
pub fn evaluate_and_act(
    state: &mut PersistedState,
    nvs: &mut impl NvsStore,
    pump: &mut impl Pump,
    tick: &OfflineTick<'_>,
    mint_command_id: impl FnOnce() -> CommandId,
    mint_event_id: impl Fn() -> rhizo_mqtt_contract::EventId,
) -> AutonomousOutcome {
    let &OfflineTick {
        plant_id,
        inputs,
        elapsed,
        monotonic_ms,
        device_time_ms,
    } = tick;
    let Some(policy) = crate::policy::active_for_plant(state, plant_id).cloned() else {
        // SAFETY-013. Not a fault, not a default: the documented behaviour of an
        // unprovisioned device is a data logger.
        buffer_refusal(
            state,
            RefuseReason::NoValidPolicy,
            monotonic_ms,
            device_time_ms,
            &mint_event_id,
        );
        return AutonomousOutcome::NoValidPolicy;
    };

    let evaluator_inputs = OfflineInputs {
        control: inputs.control.clone(),
        required: inputs.required.clone(),
        leak: inputs.leak,
        tank_percent: inputs.tank_percent,
        pump_healthy: inputs.pump_healthy,
    };
    let evaluator_state = state.offline_runtime.to_offline_state();

    // --------------------------------------------------------- the one call
    let decision = evaluate_offline(&policy, &evaluator_state, &evaluator_inputs, elapsed);
    let next = next_offline_state(&policy, &evaluator_state, &decision, elapsed);
    state.offline_runtime.apply_offline_state(next);
    state
        .offline_runtime
        .credit_window(elapsed, u64::from(policy.limits.window_ms));

    match decision {
        OfflineDecision::Dose { ml } => {
            // The volume ceilings apply to every drop this device moves,
            // whoever decided to move it, and they come from the same shared
            // function the commanded path uses.
            match bound_dose(ml, inputs.pump_ml_per_second, state.daily.delivered_ml) {
                DoseBound::Reject(reason) => AutonomousOutcome::BoundRefused(reason),
                DoseBound::Accept {
                    effective_ml,
                    run_ms,
                    ..
                } => {
                    let command_id = mint_command_id();
                    match execute_authorised(
                        state,
                        nvs,
                        pump,
                        &crate::command::AuthorisedDose {
                            command_id,
                            requested_ml: ml,
                            effective_ml,
                            run_ms,
                            started_at_ms: device_time_ms,
                            autonomous: true,
                        },
                    ) {
                        Ok(delivered_ml) => {
                            let trigger_value = inputs
                                .control
                                .as_ref()
                                .and_then(scalar_of)
                                .unwrap_or(f64::NAN);
                            state.buffer.push(
                                mint_event_id(),
                                EventTier::Audit,
                                EventKind::WateringOfflineAutonomous,
                                monotonic_ms,
                                device_time_ms,
                                EventDetail::Watering {
                                    // The dose names its own subject. The edge
                                    // would otherwise infer ownership from
                                    // bindings that may have been edited while
                                    // this device was alone, and charge the
                                    // wrong budget in both directions at once.
                                    plant_id: Some(policy.plant_id.clone()),
                                    policy_version: policy.policy_version,
                                    delivered_ml,
                                    trigger_value,
                                    duration_ms: run_ms,
                                },
                            );
                            // An autonomous dose still produces a result, so the
                            // edge's ledger learns the volume through the same
                            // channel a commanded dose uses.
                            let result = result_for(
                                command_id,
                                ml,
                                state.daily.delivered_ml,
                                CommandOrigin::OfflineAutonomous,
                                ResultOutcome::Completed {
                                    delivered_ml,
                                    duration_ms: run_ms,
                                    clamped: effective_ml < ml,
                                },
                            );
                            crate::dedup::record(&mut state.dedup_ring, command_id, result.clone());
                            let _ = state.pending_results.insert(result);
                            let _ = nvs.store(state);
                            AutonomousOutcome::Dosed {
                                delivered_ml,
                                policy_version: policy.policy_version,
                            }
                        }
                        Err(fault) => AutonomousOutcome::Failed(fault),
                    }
                }
            }
        }
        OfflineDecision::Refuse(reason) => {
            buffer_refusal(state, reason, monotonic_ms, device_time_ms, &mint_event_id);
            AutonomousOutcome::Refused(reason)
        }
        OfflineDecision::Idle
        | OfflineDecision::Confirming
        | OfflineDecision::WaitAbsorption
        | OfflineDecision::Cooldown => AutonomousOutcome::Waiting,
    }
}

/// Buffers one audit event for a refusal.
///
/// Every refusal is recorded with its reason so a reconnect makes it
/// observable. The caller is responsible for suppressing repeats of an
/// unchanged reason: a leak that lasts a week would otherwise fill the 64-slot
/// audit ring with the same sentence and evict the record of the dose that
/// matters (SAFETY-020).
fn buffer_refusal(
    state: &mut PersistedState,
    reason: RefuseReason,
    monotonic_ms: u64,
    device_time_ms: Option<UtcMillis>,
    mint_event_id: &impl Fn() -> rhizo_mqtt_contract::EventId,
) {
    state.buffer.push(
        mint_event_id(),
        EventTier::Audit,
        EventKind::OfflineRefused,
        monotonic_ms,
        device_time_ms,
        EventDetail::Refused {
            reason: refuse_reason_name(reason).to_owned(),
        },
    );
}

/// The scalar behind a sample, or `None` when there is not one.
fn scalar_of(sample: &OfflineSample) -> Option<f64> {
    if sample.quality != Quality::Ok {
        return None;
    }
    match sample.value.as_ref()? {
        MeasurementValue::Scalar(value) => Some(*value),
        MeasurementValue::Boolean(_) => None,
    }
}

/// The stable name a refusal is buffered under.
///
/// Exhaustive with no catch-all, so a refusal reason added to the shared crate
/// has to be named here before it can be recorded. The names are compared
/// against the simulator's by the conformance test — a divergence here is a
/// divergence in what the edge sees, which is the whole point of M9-014.
#[must_use]
pub const fn refuse_reason_name(reason: RefuseReason) -> &'static str {
    match reason {
        RefuseReason::NoValidPolicy => "no_valid_policy",
        RefuseReason::PolicyDisabled => "policy_disabled",
        RefuseReason::PolicyInvalid => "policy_invalid",
        RefuseReason::NoActuator => "no_actuator",
        RefuseReason::ControlMissing => "control_missing",
        RefuseReason::ControlStale => "control_stale",
        RefuseReason::ControlQuality => "control_quality",
        RefuseReason::ControlKindUnknown => "control_kind_unknown",
        RefuseReason::RequiredMissing => "required_missing",
        RefuseReason::RequiredStale => "required_stale",
        RefuseReason::RequiredQuality => "required_quality",
        RefuseReason::LeakDetected => "leak_detected",
        RefuseReason::LeakUnknown => "leak_unknown",
        RefuseReason::TankUnknown => "tank_unknown",
        RefuseReason::TankLow => "tank_low",
        RefuseReason::PumpUnknown => "pump_unknown",
        RefuseReason::PumpUnhealthy => "pump_unhealthy",
        RefuseReason::CooldownActive => "cooldown_active",
        RefuseReason::BudgetExhausted => "budget_exhausted",
        RefuseReason::MaxDosesReached => "max_doses_reached",
    }
}

/// Builds a control sample from a scalar reading.
#[must_use]
pub fn scalar_sample(kind: MeasurementKind, value: f64, age: MonotonicMillis) -> OfflineSample {
    OfflineSample {
        kind,
        value: Some(MeasurementValue::Scalar(value)),
        quality: Quality::Ok,
        age,
    }
}

/// Whether the persisted safety state can be trusted enough to act on.
///
/// A device that cannot trust its stored safety history must not water,
/// whatever any evaluator would say: the budget, the cooldown, and the dedup
/// ring are exactly the state that is in doubt.
#[must_use]
pub fn actuation_permitted(state: &PersistedState, nvs_healthy: bool) -> bool {
    nvs_healthy && crate::policy::active(state).is_some()
}

/// Re-exported so a caller does not have to depend on `rhizo_policy` directly
/// to name the policy it is acting on.
pub type ActivePolicy = OfflinePolicy;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fakes::{FakeNvs, FakePump, call_log};
    use crate::policy::UpdateStep;
    use rhizo_mqtt_contract::EventId;
    use rhizo_mqtt_contract::payload::{
        ActuatorKind, ControlMeasurement, MeasurementPoint, OfflineActuator, OfflineLimits,
        OfflinePolicySet, OfflineSafety, SensorId,
    };
    use std::cell::Cell;
    use uuid::Uuid;

    fn policy_set(enabled: bool) -> OfflinePolicySet {
        OfflinePolicySet {
            policies: vec![OfflinePolicy {
                plant_id: SensorId::parse("basil").expect("id"),
                policy_version: 1,
                enabled,
                actuator: Some(OfflineActuator {
                    actuator_id: SensorId::parse("pump-1").expect("id"),
                    kind: ActuatorKind::IrrigationPump,
                    dose_ml: 40.0,
                    max_doses_per_cycle: 2,
                    absorption_wait_ms: 900_000,
                }),
                control_measurement: ControlMeasurement {
                    kind: MeasurementKind::SoilMoisture,
                    point: MeasurementPoint::parse("default").expect("valid point"),
                    trigger_below: 25.0,
                    resume_above: 35.0,
                    confirm_duration_ms: 600_000,
                    max_age_ms: 900_000,
                },
                required_measurements: Vec::new(),
                advisory_measurements: Vec::new(),
                limits: OfflineLimits {
                    cooldown_ms: 3_600_000,
                    max_volume_per_window_ml: 200.0,
                    window_ms: 86_400_000,
                },
                safety: OfflineSafety {
                    require_leak_clear: true,
                    require_tank_above_percent: 15.0,
                    require_pump_healthy: true,
                },
            }],
        }
    }

    fn seam(vwc: f64) -> OfflineSeamInputs {
        OfflineSeamInputs {
            control: Some(scalar_sample(
                MeasurementKind::SoilMoisture,
                vwc,
                MonotonicMillis(0),
            )),
            required: Vec::new(),
            leak: Some(LeakState::Clear),
            tank_percent: Some(60.0),
            pump_healthy: Some(true),
            pump_ml_per_second: 8.0,
        }
    }

    struct Ids(Cell<u128>);
    impl Ids {
        fn new() -> Self {
            Self(Cell::new(1))
        }
        fn next(&self) -> u128 {
            let n = self.0.get();
            self.0.set(n + 1);
            n
        }
    }

    fn run(
        state: &mut PersistedState,
        nvs: &mut FakeNvs,
        pump: &mut FakePump,
        inputs: &OfflineSeamInputs,
        elapsed: MonotonicMillis,
        ids: &Ids,
    ) -> AutonomousOutcome {
        evaluate_and_act(
            state,
            nvs,
            pump,
            &OfflineTick {
                plant_id: "basil",
                inputs,
                elapsed,
                monotonic_ms: elapsed.0,
                device_time_ms: None,
            },
            || CommandId::from_uuid(Uuid::from_u128(ids.next())),
            || EventId::from_uuid(Uuid::from_u128(ids.next())),
        )
    }

    fn rig(enabled: bool) -> (PersistedState, FakeNvs, FakePump) {
        let mut state = PersistedState::default();
        crate::policy::apply(&mut state, &policy_set(enabled), UpdateStep::Complete);
        (state, FakeNvs::new(), FakePump::new(call_log()))
    }

    /// SAFETY-013: an isolated device with no policy never waters, and the
    /// refusal is recorded so the edge learns of it on reconnect.
    #[test]
    fn safety_013_an_isolated_device_with_no_policy_never_waters() {
        let mut state = PersistedState::default();
        let mut nvs = FakeNvs::new();
        let mut pump = FakePump::new(call_log());
        let ids = Ids::new();
        let outcome = run(
            &mut state,
            &mut nvs,
            &mut pump,
            &seam(10.0),
            MonotonicMillis(600_000),
            &ids,
        );
        assert_eq!(outcome, AutonomousOutcome::NoValidPolicy);
        assert_eq!(pump.total_run_ms, 0);
        assert_eq!(state.buffer.len(), 1);
    }

    #[test]
    fn a_disabled_policy_refuses_and_records_the_reason() {
        let (mut state, mut nvs, mut pump) = rig(false);
        let ids = Ids::new();
        let outcome = run(
            &mut state,
            &mut nvs,
            &mut pump,
            &seam(10.0),
            MonotonicMillis(600_000),
            &ids,
        );
        assert_eq!(
            outcome,
            AutonomousOutcome::Refused(RefuseReason::PolicyDisabled)
        );
        assert_eq!(pump.total_run_ms, 0);
    }

    #[test]
    fn safety_017_an_isolated_device_waters_within_bounds_after_confirmation() {
        let (mut state, mut nvs, mut pump) = rig(true);
        let ids = Ids::new();
        // Enter confirmation, then accumulate the confirm duration.
        assert_eq!(
            run(
                &mut state,
                &mut nvs,
                &mut pump,
                &seam(10.0),
                MonotonicMillis(0),
                &ids
            ),
            AutonomousOutcome::Waiting
        );
        let outcome = run(
            &mut state,
            &mut nvs,
            &mut pump,
            &seam(10.0),
            MonotonicMillis(600_000),
            &ids,
        );
        assert_eq!(
            outcome,
            AutonomousOutcome::Dosed {
                delivered_ml: 40.0,
                policy_version: 1
            }
        );
        assert_eq!(state.daily.delivered_ml, 40.0);
        assert_eq!(pump.total_run_ms, 5_000);
        assert!(state.in_flight_dose.is_none());
        // The audit record and the result both exist: the edge learns what
        // happened through history *and* through the ledger.
        assert!(!state.buffer.is_empty());
        assert_eq!(state.pending_results.len(), 1);
    }

    /// SAFETY-012 through the shared gate: an unreadable leak sensor is
    /// `Unknown`, and `Unknown` is a refusal.
    #[test]
    fn safety_012_an_unknown_leak_state_refuses_rather_than_permits() {
        let (mut state, mut nvs, mut pump) = rig(true);
        let ids = Ids::new();
        let mut inputs = seam(10.0);
        inputs.leak = None;
        let outcome = run(
            &mut state,
            &mut nvs,
            &mut pump,
            &inputs,
            MonotonicMillis(600_000),
            &ids,
        );
        assert_eq!(
            outcome,
            AutonomousOutcome::Refused(RefuseReason::LeakUnknown)
        );
        assert_eq!(pump.total_run_ms, 0);
    }

    /// SAFETY-015: a reboot credits zero elapsed time, so a device that reboots
    /// through a confirmation never completes one.
    #[test]
    fn safety_015_a_reboot_loop_never_completes_a_confirmation() {
        let (mut state, mut nvs, mut pump) = rig(true);
        let ids = Ids::new();
        for _ in 0..100 {
            let outcome = run(
                &mut state,
                &mut nvs,
                &mut pump,
                &seam(10.0),
                MonotonicMillis(0),
                &ids,
            );
            assert_eq!(outcome, AutonomousOutcome::Waiting);
        }
        assert_eq!(pump.total_run_ms, 0);
        assert_eq!(state.daily.delivered_ml, 0.0);
    }

    /// SAFETY-007/-014: the hard limits bound an autonomous dose exactly as
    /// they bound a commanded one, from the same shared function.
    #[test]
    fn safety_014_the_shared_hard_limits_bound_an_autonomous_dose() {
        let (mut state, mut nvs, mut pump) = rig(true);
        state.daily.delivered_ml = 480.0;
        let ids = Ids::new();
        run(
            &mut state,
            &mut nvs,
            &mut pump,
            &seam(10.0),
            MonotonicMillis(0),
            &ids,
        );
        let outcome = run(
            &mut state,
            &mut nvs,
            &mut pump,
            &seam(10.0),
            MonotonicMillis(600_000),
            &ids,
        );
        assert_eq!(
            outcome,
            AutonomousOutcome::BoundRefused(
                rhizo_mqtt_contract::payload::RejectReason::OverDailyMax
            )
        );
        assert_eq!(pump.total_run_ms, 0);
    }
}
