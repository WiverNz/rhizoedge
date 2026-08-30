//! The offline-autonomy safety property suite (M6-021).
//!
//! SAFETY-013…020 get the same treatment SAFETY-001…012 received: properties
//! against the pure evaluator, cheap enough that they actually get run.
//!
//! # The flagship
//!
//! [`safety_014_combined_budget_never_exceeded`] is the offline counterpart of
//! `safety_006`. It interleaves commanded and autonomous doses across 72
//! simulated hours with reboots and reconnections, and asserts that the rolling
//! sum across **both** control paths stays within the cap. There is one budget
//! per plant, not one per control path, and this is the test that says so.
//!
//! # The converse nobody writes
//!
//! [`safety_017_missing_advisory_does_not_block`] is easy to forget and matters
//! as much as its partner: an implementation that refused on *any* missing
//! measurement would pass every other test here and would make advisory
//! bindings useless.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use proptest::prelude::*;
use proptest::test_runner::FileFailurePersistence;
use rhizo_mqtt_contract::payload::{
    ActuatorKind, AdvisoryMeasurement, ControlMeasurement, MeasurementKind, MeasurementPoint,
    MeasurementValue, OfflineActuator, OfflineLimits, OfflinePolicy, OfflineSafety, Quality,
    RequiredMeasurement, SensorId,
};
use rhizo_mqtt_contract::safety::{FIRMWARE_MAX_ML_PER_RUN, LeakState, bound_dose};
use rhizo_policy::{
    MonotonicMillis, OfflineCycle, OfflineDecision, OfflineInputs, OfflineSample, OfflineState,
    RefuseReason, evaluate_offline, next_offline_state,
};

fn policy() -> OfflinePolicy {
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

fn sample(kind: MeasurementKind, value: f64, age_ms: u64) -> OfflineSample {
    OfflineSample {
        kind,
        value: Some(MeasurementValue::Scalar(value)),
        quality: Quality::Ok,
        age: MonotonicMillis(age_ms),
    }
}

fn inputs(moisture: f64) -> OfflineInputs {
    OfflineInputs {
        control: Some(sample(MeasurementKind::SoilMoisture, moisture, 0)),
        required: Vec::new(),
        leak: Some(LeakState::Clear),
        tank_percent: Some(70.0),
        pump_healthy: Some(true),
    }
}

/// One evaluation, returning the decision and the state it leaves.
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

/// The rolling accumulator the device keeps, and the window roll the shared
/// state does **not** perform — that is device bookkeeping (offline-autonomy §5).
#[derive(Clone, Copy, Debug, Default)]
struct Window {
    elapsed_ms: u64,
    used_ml: f32,
}

impl Window {
    /// Advances by observed monotonic time, rolling only when a whole window
    /// has genuinely passed. A device that reboots repeatedly does not thereby
    /// earn more water (SAFETY-015).
    fn advance(&mut self, elapsed_ms: u64, window_ms: u64) {
        self.elapsed_ms = self.elapsed_ms.saturating_add(elapsed_ms);
        if window_ms > 0 && self.elapsed_ms >= window_ms {
            self.elapsed_ms = 0;
            self.used_ml = 0.0;
        }
    }
}

// ------------------------------------------------------------------ SAFETY-013

proptest! {
    #![proptest_config(ProptestConfig {
        failure_persistence: Some(Box::new(FileFailurePersistence::Direct(
            "proptest-regressions/safety.txt",
        ))),
        ..ProptestConfig::default()
    })]

    /// **SAFETY-013.** A device with no *enabled* policy never actuates,
    /// whatever the soil says and however long it has been dry.
    ///
    /// A device with no policy at all never reaches the evaluator — the caller
    /// has nothing to pass — so the strongest thing this level can assert is
    /// that a disabled one never doses.
    #[test]
    fn safety_013_no_policy_never_actuates(
        moisture in -50.0f64..200.0,
        elapsed_ms in 0u64..86_400_000,
        cycle in 0usize..4,
    ) {
        let mut policy = policy();
        policy.enabled = false;
        let state = OfflineState {
            cycle: [
                OfflineCycle::Idle,
                OfflineCycle::Confirming,
                OfflineCycle::WaitAbsorption,
                OfflineCycle::Cooldown,
            ][cycle],
            ..OfflineState::default()
        };
        let (decision, next) = step(&policy, &state, &inputs(moisture), elapsed_ms);
        prop_assert!(!matches!(decision, OfflineDecision::Dose { .. }), "{decision:?}");
        prop_assert_eq!(next.budget_used_ml, state.budget_used_ml);
        prop_assert_eq!(next.dose_count, state.dose_count);
    }

    /// **SAFETY-013.** A policy that fails validation is refused rather than
    /// repaired, guessed at, or partially applied.
    #[test]
    fn safety_013_corrupt_policy_never_actuates(
        which in 0usize..6,
        moisture in -50.0f64..200.0,
    ) {
        let mut policy = policy();
        match which {
            0 => policy.control_measurement.resume_above = 0.0,
            1 => policy.actuator.as_mut().unwrap().dose_ml = FIRMWARE_MAX_ML_PER_RUN + 1.0,
            2 => policy.actuator.as_mut().unwrap().dose_ml = f32::NAN,
            3 => policy.limits.max_volume_per_window_ml = f32::INFINITY,
            4 => policy.control_measurement.confirm_duration_ms = 0,
            _ => policy.actuator = None,
        }
        let (decision, _) = step(&policy, &OfflineState::default(), &inputs(moisture), 0);
        prop_assert!(
            matches!(
                decision,
                OfflineDecision::Refuse(RefuseReason::PolicyInvalid | RefuseReason::NoActuator)
            ),
            "corruption {which} produced {decision:?}"
        );
    }

    /// **SAFETY-014, the flagship.** Interleaved commanded and autonomous doses
    /// across 72 simulated hours with reboots never take the rolling window past
    /// its cap.
    ///
    /// The commanded doses are the seam: they spend the *same* budget, because
    /// there is one budget per plant and not one per control path. Two
    /// independent budgets is the obvious way to build this and the obvious way
    /// to double-water a plant.
    #[test]
    fn safety_014_combined_budget_never_exceeded(
        script in proptest::collection::vec(
            prop_oneof![
                (1u64..7_200_000).prop_map(Move::Advance),
                (0.0f64..60.0).prop_map(Move::Moisture),
                Just(Move::Reboot),
                (1.0f32..80.0).prop_map(Move::CommandedDose),
                Just(Move::Reconnect),
            ],
            1..300,
        ),
    ) {
        let policy = policy();
        let cap = policy.limits.max_volume_per_window_ml;
        let mut state = OfflineState::default();
        let mut window = Window::default();
        let mut moisture = 20.0;
        let mut isolated = true;

        for mv in script {
            match mv {
                Move::Moisture(value) => moisture = value,
                Move::Reconnect => isolated = !isolated,
                // A reboot credits zero elapsed time and preserves every
                // persisted counter. It is not a way to earn water.
                Move::Reboot => {}
                Move::CommandedDose(requested) => {
                    // A commanded dose reaches the device only while connected,
                    // and is bounded by the same firmware ceilings.
                    if isolated {
                        continue;
                    }
                    let rhizo_mqtt_contract::safety::DoseBound::Accept { effective_ml, .. } =
                        bound_dose(requested, 8.0, window.used_ml)
                    else {
                        continue;
                    };
                    // The Edge refuses a dose that would cross the plant's cap
                    // (SAFETY-006); the device's own window is charged for it.
                    if window.used_ml + effective_ml > cap {
                        continue;
                    }
                    window.used_ml += effective_ml;
                    state.budget_used_ml = window.used_ml;
                }
                Move::Advance(elapsed_ms) => {
                    window.advance(elapsed_ms, u64::from(policy.limits.window_ms));
                    state.budget_used_ml = window.used_ml;
                    if !isolated {
                        continue;
                    }
                    let (decision, next) = step(&policy, &state, &inputs(moisture), elapsed_ms);
                    if let OfflineDecision::Dose { ml } = decision {
                        window.used_ml += ml;
                    }
                    state = next;
                    state.budget_used_ml = window.used_ml;
                }
            }
            prop_assert!(
                window.used_ml <= cap + f32::EPSILON,
                "the combined rolling window reached {} against a {cap} ml cap",
                window.used_ml
            );
        }
    }

    /// **SAFETY-015.** However many times a device reboots, the budget is not
    /// replenished: a reboot credits zero observed time, and the accumulator is
    /// reduced only when a whole window has been *observed* to pass.
    #[test]
    fn safety_015_reboot_does_not_replenish_budget(
        reboots in 1usize..200,
        used in 0.0f32..300.0,
    ) {
        let policy = policy();
        let mut state = OfflineState {
            budget_used_ml: used,
            ..OfflineState::default()
        };
        let mut window = Window {
            elapsed_ms: 0,
            used_ml: used,
        };
        for _ in 0..reboots {
            // A reboot: zero credited time, state restored from persistence.
            window.advance(0, u64::from(policy.limits.window_ms));
            let (_, next) = step(&policy, &state, &inputs(10.0), 0);
            state = next;
            prop_assert!(
                state.budget_used_ml >= used - f32::EPSILON,
                "a reboot reduced the budget from {used} to {}",
                state.budget_used_ml
            );
        }
        prop_assert_eq!(window.used_ml, used);
    }

    /// **SAFETY-015.** A reboot never shortens a cooldown either, and the
    /// cooldown is stored as a *remaining duration* precisely so that it cannot.
    #[test]
    fn safety_015_reboot_does_not_shorten_cooldown(
        remaining_ms in 1u64..86_400_000,
        reboots in 1usize..200,
    ) {
        let policy = policy();
        let mut state = OfflineState {
            cycle: OfflineCycle::Cooldown,
            cooldown_remaining: MonotonicMillis(remaining_ms),
            ..OfflineState::default()
        };
        for _ in 0..reboots {
            let (decision, next) = step(&policy, &state, &inputs(10.0), 0);
            prop_assert_eq!(decision, OfflineDecision::Cooldown);
            state = next;
        }
        prop_assert_eq!(
            state.cooldown_remaining,
            MonotonicMillis(remaining_ms),
            "the cooldown must be untouched by any number of reboots"
        );
    }

    /// **SAFETY-017.** Every declared required measurement blocks when it is
    /// missing, stale, or of non-`Ok` quality.
    #[test]
    fn safety_017_missing_required_blocks(
        condition in 0usize..4,
        moisture in 0.0f64..20.0,
    ) {
        let mut policy = policy();
        policy.required_measurements = vec![RequiredMeasurement {
            kind: MeasurementKind::PotWeight,
            point: MeasurementPoint::parse("default").unwrap(),
            max_age_ms: 900_000,
        }];
        let mut i = inputs(moisture);
        i.required = match condition {
            // Never sampled at all.
            0 => Vec::new(),
            // Sampled, but the read failed.
            1 => vec![OfflineSample {
                kind: MeasurementKind::PotWeight,
                value: None,
                quality: Quality::Ok,
                age: MonotonicMillis(0),
            }],
            // Sampled and fresh, but not usable.
            2 => {
                let mut s = sample(MeasurementKind::PotWeight, 1_800.0, 0);
                s.quality = Quality::Fault;
                vec![s]
            }
            // Sampled and usable, but too old.
            _ => vec![sample(MeasurementKind::PotWeight, 1_800.0, 900_001)],
        };
        let (decision, _) = step(&policy, &OfflineState::default(), &i, 0);
        prop_assert!(
            matches!(
                decision,
                OfflineDecision::Refuse(
                    RefuseReason::RequiredMissing
                        | RefuseReason::RequiredStale
                        | RefuseReason::RequiredQuality
                )
            ),
            "condition {condition} produced {decision:?}"
        );
    }

    /// **SAFETY-017's converse, and the one that is easy to forget.** A plant
    /// that never declared a requirement is not blocked by the absence of the
    /// sensor. An implementation that refused on *any* missing measurement would
    /// pass every other test here and make advisory bindings useless.
    #[test]
    fn safety_017_missing_advisory_does_not_block(
        moisture in 0.0f64..27.9,
        advisory_present in any::<bool>(),
    ) {
        let mut policy = policy();
        policy.advisory_measurements = vec![AdvisoryMeasurement {
            kind: MeasurementKind::AmbientTemperature,
            point: MeasurementPoint::parse("default").unwrap(),
        }];
        let mut i = inputs(moisture);
        if advisory_present {
            i.required
                .push(sample(MeasurementKind::AmbientTemperature, 21.0, 0));
        }
        let (decision, _) = step(&policy, &OfflineState::default(), &i, 0);
        prop_assert_eq!(
            decision,
            OfflineDecision::Confirming,
            "an advisory measurement gates nothing, present or absent"
        );
    }

    /// The offline counterpart of `prop_state_machine_total`: every state
    /// crossed with adversarial inputs yields a defined decision and never
    /// panics.
    #[test]
    fn prop_offline_evaluator_total(
        cycle in 0usize..4,
        moisture in proptest::option::of(prop_oneof![
            Just(f64::NAN), Just(f64::INFINITY), -100.0f64..200.0
        ]),
        age_ms in 0u64..10_000_000,
        leak in prop_oneof![
            Just(None),
            Just(Some(LeakState::Clear)),
            Just(Some(LeakState::Detected)),
            Just(Some(LeakState::Unknown)),
        ],
        tank in proptest::option::of(prop_oneof![Just(f32::NAN), -10.0f32..150.0]),
        pump in proptest::option::of(any::<bool>()),
        used in prop_oneof![Just(f32::NAN), -10.0f32..500.0],
        doses in 0u16..8,
        cooldown_ms in 0u64..90_000_000,
        elapsed_ms in 0u64..90_000_000,
        enabled in any::<bool>(),
    ) {
        let mut policy = policy();
        policy.enabled = enabled;
        let state = OfflineState {
            cycle: [
                OfflineCycle::Idle,
                OfflineCycle::Confirming,
                OfflineCycle::WaitAbsorption,
                OfflineCycle::Cooldown,
            ][cycle],
            dose_count: doses,
            budget_used_ml: used,
            cooldown_remaining: MonotonicMillis(cooldown_ms),
            confirm_elapsed: MonotonicMillis(0),
        };
        let i = OfflineInputs {
            control: moisture.map(|value| OfflineSample {
                kind: MeasurementKind::SoilMoisture,
                value: Some(MeasurementValue::Scalar(value)),
                quality: Quality::Ok,
                age: MonotonicMillis(age_ms),
            }),
            required: Vec::new(),
            leak,
            tank_percent: tank,
            pump_healthy: pump,
        };

        let (decision, next) = step(&policy, &state, &i, elapsed_ms);
        // A dose is issued only when **every** safety input was positively good.
        if let OfflineDecision::Dose { ml } = decision {
            prop_assert!(enabled);
            prop_assert_eq!(leak, Some(LeakState::Clear));
            prop_assert!(tank.is_some_and(|t| t.is_finite() && t > 15.0));
            prop_assert_eq!(pump, Some(true));
            prop_assert!(used.is_finite());
            prop_assert!(used + ml <= policy.limits.max_volume_per_window_ml);
            prop_assert!(doses < 2);
            prop_assert!(moisture.is_some_and(|v| v.is_finite() && v < 28.0));
            prop_assert!(age_ms < 900_000);
            prop_assert!((ml - 35.0).abs() < f32::EPSILON, "the policy dose, never a computed one");
        }
        // ...and the cooldown never lengthens on its own or wraps.
        prop_assert!(next.cooldown_remaining.0 <= cooldown_ms.max(21_600_000));
    }
}

/// One move in the combined-budget script.
#[derive(Clone, Copy, Debug)]
enum Move {
    Advance(u64),
    Moisture(f64),
    Reboot,
    CommandedDose(f32),
    Reconnect,
}

/// **SAFETY-014's hard-limit half.** A policy dose is still bounded by the
/// firmware ceiling, whoever authored it.
#[test]
fn safety_014_hard_limit_applies_offline() {
    // The contract refuses to *validate* a policy above the ceiling, which is
    // the first line of defence...
    let mut policy = policy();
    policy.actuator.as_mut().unwrap().dose_ml = FIRMWARE_MAX_ML_PER_RUN + 10.0;
    assert!(policy.validate().is_err());
    let (decision, _) = step(&policy, &OfflineState::default(), &inputs(10.0), 0);
    assert_eq!(
        decision,
        OfflineDecision::Refuse(RefuseReason::PolicyInvalid)
    );

    // ...and the actuation path clamps regardless, which is the second. Both
    // are the same shared function the commanded path uses.
    match bound_dose(FIRMWARE_MAX_ML_PER_RUN + 100.0, 8.0, 0.0) {
        rhizo_mqtt_contract::safety::DoseBound::Accept {
            effective_ml,
            clamped,
            ..
        } => {
            assert!(effective_ml <= FIRMWARE_MAX_ML_PER_RUN);
            assert!(clamped);
        }
        other => panic!("{other:?}"),
    }
}

/// **SAFETY-013.** A disabled policy is refused for being disabled, and the
/// refusal is specific rather than a generic "no".
#[test]
fn safety_013_disabled_policy_never_actuates() {
    let mut policy = policy();
    policy.enabled = false;
    let (decision, _) = step(&policy, &OfflineState::default(), &inputs(5.0), 3_600_000);
    assert_eq!(
        decision,
        OfflineDecision::Refuse(RefuseReason::PolicyDisabled)
    );
}
