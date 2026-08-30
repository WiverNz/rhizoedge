//! The single shared offline evaluator (M6-019, ADR-015, ADR-008).
//!
//! **One implementation, called from exactly one place per consumer.** The
//! simulator calls it, the firmware will call it in M9-016, and the Edge links
//! it so it can *predict* what an isolated device will do and refuse to publish
//! a policy it cannot evaluate. A second copy of these rules would make every
//! offline safety test in M6 and every isolation scenario in M8 exercise rules
//! the hardware does not follow.
//!
//! # Pure, `no_std`, and clock-free
//!
//! The crate cannot read a clock — there is no dependency that could — so
//! SAFETY-015 is **structural** rather than disciplined. Elapsed time arrives as
//! a parameter, and the caller is responsible for having measured it.
//!
//! ## What `elapsed` means
//!
//! It is the monotonic time **observed since the previous evaluation**, which is
//! exactly what [offline-autonomy.md](../../../docs/architecture/offline-autonomy.md)
//! §5b's `credit_elapsed(wake_reason, rtc_state) -> Duration` produces: a timer
//! wake with a valid RTC checksum credits the measured interval, and *every*
//! other reset reason credits `Duration::ZERO`.
//!
//! It has to be a delta rather than a since-boot instant, because §5 stores the
//! cooldown as a **remaining duration** and never as an absolute deadline — a
//! device that may have no absolute time cannot interpret one after a boot. With
//! remaining durations on one side and an instant on the other there is nothing
//! to subtract. Crediting zero is the conservative direction and is the one a
//! reboot takes: a device in a reboot loop never advances a cooldown, never
//! replenishes a budget, and never shortens a confirmation.
//!
//! # The restricted subset, and why it stays restricted
//!
//! ```text
//! threshold comparison + confirmation duration + hysteresis + cooldown
//! + bounded dose + absorption wait + bounded dose count + rolling volume cap
//! + required-sensor and staleness checks + the full safety gate
//! ```
//!
//! No trends, no recommendations, no confidence, no dose sizing. `dose_ml` is a
//! **value the Edge authored**, not a formula: computing a dose on the device
//! would be the beginning of reimplementing the recommendation engine, on the
//! hardware where it is hardest to debug.

use rhizo_mqtt_contract::payload::OfflinePolicy;

use crate::gate::{control_value, offline_gate};
use crate::types::{
    MonotonicMillis, OfflineCycle, OfflineDecision, OfflineInputs, OfflineState, RefuseReason,
};

/// Decides what an isolated device should do next.
///
/// Pure and total: every (state, input) pair yields a defined decision,
/// including inputs that are absent — which resolve to a refusal through the
/// gate. The gate is the first statement, unconditionally.
#[must_use]
pub fn evaluate_offline(
    policy: &OfflinePolicy,
    state: &OfflineState,
    inputs: &OfflineInputs,
    elapsed: MonotonicMillis,
) -> OfflineDecision {
    // The cooldown is counted down before the gate, because being *in* a
    // cooldown is not a refusal — it is the cycle working.
    let cooldown_remaining = state.cooldown_remaining.0.saturating_sub(elapsed.0);
    if state.cycle == OfflineCycle::Cooldown && cooldown_remaining > 0 {
        return OfflineDecision::Cooldown;
    }

    if let Some(reason) = offline_gate(policy, state, inputs) {
        // Reaching the dose limit inside a cycle is a *cycle* outcome rather
        // than a fault, so it ends the cycle into a cooldown instead of
        // refusing for ever. Everything else the gate says is a refusal.
        if reason == RefuseReason::MaxDosesReached
            && matches!(
                state.cycle,
                OfflineCycle::WaitAbsorption | OfflineCycle::Confirming
            )
        {
            return OfflineDecision::Cooldown;
        }
        return OfflineDecision::Refuse(reason);
    }

    // The gate guarantees a usable scalar past this point.
    let Some(value) = inputs.control.as_ref().and_then(control_value) else {
        return OfflineDecision::Refuse(RefuseReason::ControlMissing);
    };
    let control = &policy.control_measurement;

    match state.cycle {
        // Hysteresis: a reading between `trigger_below` and `resume_above` does
        // nothing at all. Without it a sensor sitting on the threshold produces
        // one dose per evaluation tick.
        OfflineCycle::Idle => {
            if value < control.trigger_below {
                OfflineDecision::Confirming
            } else {
                OfflineDecision::Idle
            }
        }
        OfflineCycle::Confirming => {
            if value >= control.resume_above {
                // The plant recovered on its own — rain, a person, absorption
                // from an earlier cycle. Nothing to do.
                return OfflineDecision::Idle;
            }
            if value >= control.trigger_below {
                // Inside the hysteresis band: neither dry enough to continue
                // confirming nor wet enough to stop. Hold.
                return OfflineDecision::Confirming;
            }
            let confirmed = state.confirm_elapsed.0.saturating_add(elapsed.0);
            if confirmed >= u64::from(control.confirm_duration_ms) {
                match policy.actuator.as_ref() {
                    // Unreachable: the gate refuses a policy with no actuator.
                    None => OfflineDecision::Refuse(RefuseReason::NoActuator),
                    Some(actuator) => OfflineDecision::Dose {
                        ml: actuator.dose_ml,
                    },
                }
            } else {
                OfflineDecision::Confirming
            }
        }
        OfflineCycle::WaitAbsorption => {
            let waited = state.confirm_elapsed.0.saturating_add(elapsed.0);
            let Some(actuator) = policy.actuator.as_ref() else {
                return OfflineDecision::Refuse(RefuseReason::NoActuator);
            };
            if waited < u64::from(actuator.absorption_wait_ms) {
                return OfflineDecision::WaitAbsorption;
            }
            if value >= control.resume_above {
                // Recovered: the cycle is complete and the cooldown begins.
                return OfflineDecision::Cooldown;
            }
            if state.dose_count >= actuator.max_doses_per_cycle {
                return OfflineDecision::Cooldown;
            }
            OfflineDecision::Confirming
        }
        OfflineCycle::Cooldown => OfflineDecision::Idle,
    }
}

/// The persisted state a decision leaves behind.
///
/// The companion of [`evaluate_offline`], split the same way the Edge's machine
/// is: one function answers *what to do*, the other answers *where that leaves
/// the device*, and the caller persists the second atomically with the first.
#[must_use]
pub fn next_offline_state(
    policy: &OfflinePolicy,
    state: &OfflineState,
    decision: &OfflineDecision,
    elapsed: MonotonicMillis,
) -> OfflineState {
    let mut next = *state;
    // Time only ever *counts down* a cooldown, and only by what was observed.
    next.cooldown_remaining = MonotonicMillis(state.cooldown_remaining.0.saturating_sub(elapsed.0));

    match decision {
        OfflineDecision::Idle => {
            next.cycle = OfflineCycle::Idle;
            next.confirm_elapsed = MonotonicMillis(0);
            next.dose_count = 0;
        }
        OfflineDecision::Confirming => {
            // Entering confirmation from anywhere but confirmation restarts the
            // accumulator: "continuous time below the trigger" means continuous.
            next.confirm_elapsed = if state.cycle == OfflineCycle::Confirming {
                MonotonicMillis(state.confirm_elapsed.0.saturating_add(elapsed.0))
            } else {
                MonotonicMillis(0)
            };
            next.cycle = OfflineCycle::Confirming;
        }
        OfflineDecision::Dose { ml } => {
            next.cycle = OfflineCycle::WaitAbsorption;
            next.confirm_elapsed = MonotonicMillis(0);
            next.dose_count = state.dose_count.saturating_add(1);
            // The budget is charged **before** the pump runs, so a device that
            // dies mid-dose has already paid for it. Over-counting reduces the
            // next dose; under-counting would permit an extra one.
            next.budget_used_ml = if ml.is_finite() {
                state.budget_used_ml + ml
            } else {
                state.budget_used_ml
            };
        }
        OfflineDecision::WaitAbsorption => {
            next.cycle = OfflineCycle::WaitAbsorption;
            next.confirm_elapsed =
                MonotonicMillis(state.confirm_elapsed.0.saturating_add(elapsed.0));
        }
        OfflineDecision::Cooldown => {
            if state.cycle == OfflineCycle::Cooldown {
                next.cycle = OfflineCycle::Cooldown;
            } else {
                // A completed cycle starts a fresh cooldown at its full length.
                next.cycle = OfflineCycle::Cooldown;
                next.cooldown_remaining = MonotonicMillis(u64::from(policy.limits.cooldown_ms));
                next.confirm_elapsed = MonotonicMillis(0);
                next.dose_count = 0;
            }
        }
        // A refusal changes nothing but the clock. In particular it does not
        // reset a confirmation or a cooldown: a leak that interrupts a
        // confirmation must not hand the plant a fresh start when it clears.
        OfflineDecision::Refuse(_) => {}
    }
    next
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
pub(crate) mod fixture {
    //! One valid, permitting policy and one set of inputs, bent field by field.
    use super::*;
    use alloc::vec;
    use alloc::vec::Vec;
    use rhizo_mqtt_contract::payload::{
        ActuatorKind, ControlMeasurement, MeasurementKind, MeasurementPoint, MeasurementValue,
        OfflineActuator, OfflineLimits, OfflineSafety, Quality, RequiredMeasurement, SensorId,
    };
    use rhizo_mqtt_contract::safety::LeakState;

    use crate::types::OfflineSample;

    pub fn policy() -> OfflinePolicy {
        OfflinePolicy {
            plant_id: SensorId::parse("monstera-01").unwrap(),
            policy_version: 7,
            enabled: true,
            actuator: Some(OfflineActuator {
                actuator_id: SensorId::parse("pump-0").unwrap(),
                kind: ActuatorKind::IrrigationPump,
                dose_ml: 35.0,
                max_doses_per_cycle: 2,
                absorption_wait_ms: 900_000,
            }),
            control_measurement: ControlMeasurement {
                kind: MeasurementKind::SoilMoisture,
                point: MeasurementPoint::parse("default").unwrap(),
                trigger_below: 28.0,
                resume_above: 34.0,
                confirm_duration_ms: 1_800_000,
                max_age_ms: 900_000,
            },
            required_measurements: Vec::new(),
            advisory_measurements: Vec::new(),
            limits: OfflineLimits {
                cooldown_ms: 21_600_000,
                max_volume_per_window_ml: 300.0,
                window_ms: 86_400_000,
            },
            safety: OfflineSafety {
                require_leak_clear: true,
                require_tank_above_percent: 15.0,
                require_pump_healthy: true,
            },
        }
    }

    pub fn requiring_weight() -> OfflinePolicy {
        let mut policy = policy();
        policy.required_measurements = vec![RequiredMeasurement {
            kind: MeasurementKind::PotWeight,
            point: MeasurementPoint::parse("default").unwrap(),
            max_age_ms: 900_000,
        }];
        policy
    }

    pub fn sample(kind: MeasurementKind, value: f64, age_ms: u64) -> OfflineSample {
        OfflineSample {
            kind,
            value: Some(MeasurementValue::Scalar(value)),
            quality: Quality::Ok,
            age: MonotonicMillis(age_ms),
        }
    }

    /// Dry, fresh, and every safety input positively good.
    pub fn inputs(moisture: f64) -> OfflineInputs {
        OfflineInputs {
            control: Some(sample(MeasurementKind::SoilMoisture, moisture, 0)),
            required: Vec::new(),
            leak: Some(LeakState::Clear),
            tank_percent: Some(70.0),
            pump_healthy: Some(true),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
#[allow(
    clippy::module_inception,
    reason = "the module name is the verification filter the issue quotes literally"
)]
mod evaluate {
    use super::fixture::{inputs, policy, requiring_weight, sample};
    use super::*;
    use crate::types::OfflineSample;
    use alloc::vec;
    use rhizo_mqtt_contract::payload::{MeasurementKind, MeasurementValue, Quality};
    use rhizo_mqtt_contract::safety::LeakState;

    fn idle() -> OfflineState {
        OfflineState::default()
    }

    fn confirming(elapsed_ms: u64) -> OfflineState {
        OfflineState {
            cycle: OfflineCycle::Confirming,
            confirm_elapsed: MonotonicMillis(elapsed_ms),
            ..OfflineState::default()
        }
    }

    /// Drive one step, returning the decision and the state it leaves.
    fn step(
        policy: &OfflinePolicy,
        state: &OfflineState,
        inputs: &OfflineInputs,
        elapsed_ms: u64,
    ) -> (OfflineDecision, OfflineState) {
        let elapsed = MonotonicMillis(elapsed_ms);
        let decision = evaluate_offline(policy, state, inputs, elapsed);
        let next = next_offline_state(policy, state, &decision, elapsed);
        (decision, next)
    }

    // ------------------------------------------------------- one per gate step

    #[test]
    fn gate_step_1_a_disabled_policy_never_actuates() {
        let mut policy = policy();
        policy.enabled = false;
        let (decision, _) = step(&policy, &idle(), &inputs(10.0), 0);
        assert_eq!(
            decision,
            OfflineDecision::Refuse(RefuseReason::PolicyDisabled)
        );
    }

    #[test]
    fn gate_step_2_an_invalid_policy_never_actuates() {
        let mut policy = policy();
        policy.control_measurement.resume_above = policy.control_measurement.trigger_below;
        assert_eq!(
            evaluate_offline(&policy, &idle(), &inputs(10.0), MonotonicMillis(0)),
            OfflineDecision::Refuse(RefuseReason::PolicyInvalid)
        );

        let mut policy = super::fixture::policy();
        policy.actuator = None;
        // A policy with no actuator fails validation first, which is the same
        // refusal an operator would get from the Edge's authoring endpoint.
        assert!(matches!(
            evaluate_offline(&policy, &idle(), &inputs(10.0), MonotonicMillis(0)),
            OfflineDecision::Refuse(RefuseReason::PolicyInvalid | RefuseReason::NoActuator)
        ));
    }

    #[test]
    fn gate_step_3_leak_detected_or_unknown_refuses() {
        let policy = policy();
        for (leak, expected) in [
            (Some(LeakState::Detected), RefuseReason::LeakDetected),
            (Some(LeakState::Unknown), RefuseReason::LeakUnknown),
            (None, RefuseReason::LeakUnknown),
        ] {
            let mut i = inputs(10.0);
            i.leak = leak;
            assert_eq!(
                evaluate_offline(&policy, &idle(), &i, MonotonicMillis(0)),
                OfflineDecision::Refuse(expected),
                "{leak:?}"
            );
        }
    }

    #[test]
    fn gate_step_4_tank_unknown_or_low_refuses() {
        let policy = policy();
        for (tank, expected) in [
            (None, RefuseReason::TankUnknown),
            (Some(f32::NAN), RefuseReason::TankUnknown),
            (Some(15.0), RefuseReason::TankLow),
            (Some(0.0), RefuseReason::TankLow),
        ] {
            let mut i = inputs(10.0);
            i.tank_percent = tank;
            assert_eq!(
                evaluate_offline(&policy, &idle(), &i, MonotonicMillis(0)),
                OfflineDecision::Refuse(expected),
                "{tank:?}"
            );
        }
    }

    #[test]
    fn gate_step_5_pump_unknown_or_unhealthy_refuses() {
        let policy = policy();
        for (pump, expected) in [
            (None, RefuseReason::PumpUnknown),
            (Some(false), RefuseReason::PumpUnhealthy),
        ] {
            let mut i = inputs(10.0);
            i.pump_healthy = pump;
            assert_eq!(
                evaluate_offline(&policy, &idle(), &i, MonotonicMillis(0)),
                OfflineDecision::Refuse(expected),
                "{pump:?}"
            );
        }
    }

    #[test]
    fn gate_step_6_a_required_measurement_that_is_missing_stale_or_bad_refuses() {
        let policy = requiring_weight();
        // Never sampled at all.
        assert_eq!(
            evaluate_offline(&policy, &idle(), &inputs(10.0), MonotonicMillis(0)),
            OfflineDecision::Refuse(RefuseReason::RequiredMissing)
        );
        // Sampled, but the read failed.
        let mut i = inputs(10.0);
        i.required = vec![OfflineSample {
            kind: MeasurementKind::PotWeight,
            value: None,
            quality: Quality::Ok,
            age: MonotonicMillis(0),
        }];
        assert_eq!(
            evaluate_offline(&policy, &idle(), &i, MonotonicMillis(0)),
            OfflineDecision::Refuse(RefuseReason::RequiredMissing)
        );
        // Sampled and fresh, but not usable.
        for quality in [Quality::Uncalibrated, Quality::Suspect, Quality::Fault] {
            let mut i = inputs(10.0);
            let mut s = sample(MeasurementKind::PotWeight, 1_800.0, 0);
            s.quality = quality;
            i.required = vec![s];
            assert_eq!(
                evaluate_offline(&policy, &idle(), &i, MonotonicMillis(0)),
                OfflineDecision::Refuse(RefuseReason::RequiredQuality),
                "{quality:?}"
            );
        }
        // Sampled and usable, but too old.
        let mut i = inputs(10.0);
        i.required = vec![sample(MeasurementKind::PotWeight, 1_800.0, 900_000)];
        assert_eq!(
            evaluate_offline(&policy, &idle(), &i, MonotonicMillis(0)),
            OfflineDecision::Refuse(RefuseReason::RequiredStale)
        );
    }

    /// SAFETY-017's converse, and the easy one to forget: a plant that never
    /// declared a requirement is not blocked by the absence of the sensor.
    #[test]
    fn gate_step_6_an_undeclared_measurement_blocks_nothing() {
        let policy = policy();
        assert!(policy.required_measurements.is_empty());
        assert_eq!(
            evaluate_offline(&policy, &idle(), &inputs(10.0), MonotonicMillis(0)),
            OfflineDecision::Confirming,
            "no scale is bound, and none is required"
        );
    }

    #[test]
    fn gate_step_7_an_unusable_control_measurement_refuses() {
        let policy = policy();
        let mut i = inputs(10.0);
        i.control = None;
        assert_eq!(
            evaluate_offline(&policy, &idle(), &i, MonotonicMillis(0)),
            OfflineDecision::Refuse(RefuseReason::ControlMissing)
        );

        let mut i = inputs(10.0);
        i.control = Some(sample(MeasurementKind::SoilMoisture, 10.0, 900_000));
        assert_eq!(
            evaluate_offline(&policy, &idle(), &i, MonotonicMillis(0)),
            OfflineDecision::Refuse(RefuseReason::ControlStale)
        );

        for quality in [Quality::Uncalibrated, Quality::Suspect, Quality::Fault] {
            let mut i = inputs(10.0);
            if let Some(control) = i.control.as_mut() {
                control.quality = quality;
            }
            assert_eq!(
                evaluate_offline(&policy, &idle(), &i, MonotonicMillis(0)),
                OfflineDecision::Refuse(RefuseReason::ControlQuality),
                "{quality:?}"
            );
        }

        // A boolean reading of a scalar kind has no hysteresis and is not a
        // control value.
        let mut i = inputs(10.0);
        if let Some(control) = i.control.as_mut() {
            control.value = Some(MeasurementValue::Boolean(true));
        }
        assert_eq!(
            evaluate_offline(&policy, &idle(), &i, MonotonicMillis(0)),
            OfflineDecision::Refuse(RefuseReason::ControlMissing)
        );
    }

    #[test]
    fn gate_step_8_the_window_budget_refuses_with_the_dose_included() {
        let policy = policy();
        let spent = OfflineState {
            budget_used_ml: 280.0,
            ..OfflineState::default()
        };
        assert_eq!(
            evaluate_offline(&policy, &spent, &inputs(10.0), MonotonicMillis(0)),
            OfflineDecision::Refuse(RefuseReason::BudgetExhausted),
            "280 + 35 crosses the 300 ml window cap"
        );
        let unreadable = OfflineState {
            budget_used_ml: f32::NAN,
            ..OfflineState::default()
        };
        assert_eq!(
            evaluate_offline(&policy, &unreadable, &inputs(10.0), MonotonicMillis(0)),
            OfflineDecision::Refuse(RefuseReason::BudgetExhausted),
            "a device that cannot prove it is under budget assumes it is not"
        );
    }

    // ------------------------------------------------------------ cycle logic

    /// Confirm, dose, absorb, recheck, second dose, cooldown.
    #[test]
    fn a_full_cycle_runs_and_ends_in_a_cooldown() {
        let policy = policy();
        let dry = inputs(20.0);

        let (decision, state) = step(&policy, &idle(), &dry, 0);
        assert_eq!(decision, OfflineDecision::Confirming);
        assert_eq!(state.cycle, OfflineCycle::Confirming);

        // Half the confirmation is not the confirmation.
        let (decision, state) = step(&policy, &state, &dry, 900_000);
        assert_eq!(decision, OfflineDecision::Confirming);
        assert_eq!(state.confirm_elapsed, MonotonicMillis(900_000));

        let (decision, state) = step(&policy, &state, &dry, 900_000);
        assert_eq!(decision, OfflineDecision::Dose { ml: 35.0 });
        assert_eq!(state.cycle, OfflineCycle::WaitAbsorption);
        assert_eq!(state.dose_count, 1);
        assert!((state.budget_used_ml - 35.0).abs() < f32::EPSILON);

        // Absorbing.
        let (decision, state) = step(&policy, &state, &dry, 300_000);
        assert_eq!(decision, OfflineDecision::WaitAbsorption);

        // Absorbed and still dry: back to confirming for a second dose.
        let (decision, state) = step(&policy, &state, &dry, 600_000);
        assert_eq!(decision, OfflineDecision::Confirming);
        assert_eq!(
            state.dose_count, 1,
            "a dose is counted when it is delivered"
        );

        let (decision, state) = step(&policy, &state, &dry, 1_800_000);
        assert_eq!(decision, OfflineDecision::Dose { ml: 35.0 });
        assert_eq!(state.dose_count, 2);
        assert!((state.budget_used_ml - 70.0).abs() < f32::EPSILON);

        // The second dose exhausts the per-cycle limit, so the cycle ends.
        let (decision, state) = step(&policy, &state, &dry, 900_000);
        assert_eq!(decision, OfflineDecision::Cooldown);
        assert_eq!(state.cycle, OfflineCycle::Cooldown);
        assert_eq!(state.cooldown_remaining, MonotonicMillis(21_600_000));
        assert_eq!(state.dose_count, 0, "the cycle is over");

        // ...and holds for the whole cooldown.
        let (decision, state) = step(&policy, &state, &dry, 21_599_999);
        assert_eq!(decision, OfflineDecision::Cooldown);
        assert_eq!(state.cooldown_remaining, MonotonicMillis(1));
        let (decision, _) = step(&policy, &state, &dry, 1);
        assert_eq!(decision, OfflineDecision::Idle);
    }

    /// A plant that recovers during absorption ends the cycle rather than
    /// dosing again.
    #[test]
    fn recovery_during_absorption_ends_the_cycle() {
        let policy = policy();
        let state = OfflineState {
            cycle: OfflineCycle::WaitAbsorption,
            dose_count: 1,
            budget_used_ml: 35.0,
            confirm_elapsed: MonotonicMillis(900_000),
            ..OfflineState::default()
        };
        let (decision, next) = step(&policy, &state, &inputs(36.0), 0);
        assert_eq!(decision, OfflineDecision::Cooldown);
        assert_eq!(next.cooldown_remaining, MonotonicMillis(21_600_000));
    }

    /// Hysteresis: a reading inside the band neither starts nor ends anything.
    #[test]
    fn hysteresis_prevents_dosing_between_trigger_and_resume() {
        let policy = policy();
        // From idle, 30 % is above `trigger_below` (28) so nothing starts.
        assert_eq!(
            evaluate_offline(&policy, &idle(), &inputs(30.0), MonotonicMillis(0)),
            OfflineDecision::Idle
        );
        // Mid-confirmation, 30 % is below `resume_above` (34) so the cycle is
        // not abandoned — but it is not below the trigger either, so the
        // confirmation does not complete however long it waits.
        let held = confirming(1_800_000);
        assert_eq!(
            evaluate_offline(&policy, &held, &inputs(30.0), MonotonicMillis(1_000_000)),
            OfflineDecision::Confirming
        );
        // Above `resume_above`, the cycle ends.
        assert_eq!(
            evaluate_offline(&policy, &held, &inputs(34.0), MonotonicMillis(0)),
            OfflineDecision::Idle
        );
    }

    /// SAFETY-015: crediting zero elapsed time is what a reboot does, and it
    /// never shortens anything.
    #[test]
    fn crediting_no_elapsed_time_never_advances_a_cooldown_or_a_confirmation() {
        let policy = policy();
        let cooling = OfflineState {
            cycle: OfflineCycle::Cooldown,
            cooldown_remaining: MonotonicMillis(21_600_000),
            ..OfflineState::default()
        };
        let mut state = cooling;
        for _ in 0..1_000 {
            let (decision, next) = step(&policy, &state, &inputs(10.0), 0);
            assert_eq!(decision, OfflineDecision::Cooldown);
            state = next;
        }
        assert_eq!(
            state.cooldown_remaining,
            MonotonicMillis(21_600_000),
            "a thousand reboots are not six hours"
        );

        let mut state = confirming(0);
        for _ in 0..1_000 {
            let (decision, next) = step(&policy, &state, &inputs(10.0), 0);
            assert_eq!(decision, OfflineDecision::Confirming);
            state = next;
        }
        assert_eq!(state.confirm_elapsed, MonotonicMillis(0));
    }

    /// A refusal mid-confirmation does not hand the plant a fresh start when it
    /// clears, and does not reset the cooldown either.
    #[test]
    fn a_refusal_preserves_the_cycle_state() {
        let policy = policy();
        let state = confirming(1_200_000);
        let mut leaking = inputs(10.0);
        leaking.leak = Some(LeakState::Detected);
        let (decision, next) = step(&policy, &state, &leaking, 60_000);
        assert_eq!(
            decision,
            OfflineDecision::Refuse(RefuseReason::LeakDetected)
        );
        assert_eq!(next.cycle, OfflineCycle::Confirming);
        assert_eq!(next.confirm_elapsed, MonotonicMillis(1_200_000));
        assert_eq!(next.dose_count, 0);
    }

    /// A confirmation interrupted by recovery restarts from zero rather than
    /// resuming: "continuous time below the trigger" means continuous.
    #[test]
    fn an_interrupted_confirmation_restarts_from_zero() {
        let policy = policy();
        let state = confirming(1_700_000);
        let (_, recovered) = step(&policy, &state, &inputs(40.0), 60_000);
        assert_eq!(recovered.cycle, OfflineCycle::Idle);
        assert_eq!(recovered.confirm_elapsed, MonotonicMillis(0));
        let (decision, _) = step(&policy, &recovered, &inputs(10.0), 60_000);
        assert_eq!(
            decision,
            OfflineDecision::Confirming,
            "and it does not immediately dose on the old accumulator"
        );
    }

    /// The crate cannot read a clock, so SAFETY-015 is structural.
    #[test]
    fn nothing_in_this_crate_reads_a_clock() {
        for source in [
            include_str!("evaluate.rs"),
            include_str!("gate.rs"),
            include_str!("types.rs"),
            include_str!("validate.rs"),
            include_str!("lib.rs"),
        ] {
            // The needles are split so this file does not match itself: a
            // self-scanning test that trips on its own assertion list proves
            // nothing about the code it is scanning.
            for banned in [
                concat!("Utc::", "now"),
                concat!("Instant::", "now"),
                concat!("SystemTime::", "now"),
            ] {
                assert!(
                    !source.contains(banned),
                    "`{banned}` must not appear in rhizo-policy"
                );
            }
        }
    }

    /// The compile-time half of SAFETY-012 for the offline gate.
    #[test]
    fn no_catch_all_arm_on_an_offline_safety_match() {
        let source = include_str!("gate.rs");
        let offenders: alloc::vec::Vec<usize> = source
            .lines()
            .enumerate()
            .filter(|(_, line)| {
                let trimmed = line.trim_start();
                !trimmed.starts_with("//") && trimmed.starts_with("_ =>")
            })
            .map(|(index, _)| index + 1)
            .collect();
        assert!(
            offenders.is_empty(),
            "the offline gate must classify every variant explicitly: {offenders:?}"
        );
    }
}
