//! The safety gate: the most safety-critical function in the project
//! (M6-002 … M6-005).
//!
//! It runs **first**, always, as the first statement of
//! [`super::machine::evaluate`], which is the only public decision function in
//! this crate. A second decision path that skipped the gate would be
//! undetectable in review, so there is not one
//! ([ADR-006](../../../../docs/adr/006-irrigation-state-machine-ownership.md),
//! F-060-01).
//!
//! Invariants enforced here: **SAFETY-003** (leak blocks every mode),
//! **SAFETY-004** (tank low or unknown blocks), **SAFETY-005** (invalid or stale
//! control data blocks automatic watering), **SAFETY-006** (the rolling cap),
//! **SAFETY-012** (uncertainty is never permission), **SAFETY-016** (no dose
//! across an unreconciled seam), **SAFETY-017** (a missing required measurement
//! blocks), and **SAFETY-018** (a plant with no actuator has no actuation path).
//!
//! # No catch-all arm
//!
//! Every `match` on a safety input below is exhaustive and spells out each
//! variant. That is the compile-time half of SAFETY-012: adding a
//! `LeakState::Degraded` breaks the build until someone decides what it means,
//! rather than silently falling through a `_ =>` into permission. `cargo test
//! -p rhizo-domain no_catch_all_arm_on_a_safety_match` reads this file and fails
//! if one appears.
//!
//! # The manual exception is exactly two checks wide
//!
//! `mode: "manual"` skips **only** `SensorFault` and `StaleData`, because a
//! human has looked at the plant and taken responsibility for it (F-060-05).
//! It skips nothing else: leak, tank, the rolling cap, an explicit lockout, an
//! incomplete reconciliation, and the firmware hard limits all still apply. The
//! exception must not widen; [`tests::the_manual_exception_is_exactly_two_checks_wide`]
//! fails if it does.

use crate::state::LockoutReason;

use super::types::{IrrigationInputs, LeakState, RequiredInputState, TankState, is_auto_clearable};

/// The persisted lifecycle of a leak lockout.
///
/// SAFETY-003's asymmetry, encoded rather than remembered: a **tank** lockout
/// clears automatically when the reservoir is refilled, but a **leak** lockout
/// does not clear when the tray dries out. The signal going away is evidence
/// about the tray, not about the burst joint that filled it, so a cleared signal
/// moves to [`Self::AwaitingReset`] and waits for a person.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum LeakLockout {
    /// No leak has been seen; watering is permitted by this check.
    #[default]
    Clear,
    /// Water is present right now.
    Detected,
    /// The signal has gone, and an operator has not yet said so.
    AwaitingReset,
}

/// Why an operator's leak reset was refused.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LeakResetRefused {
    /// The tray still reports water.
    StillDetected,
    /// The leak sensor is silent, so "the signal is absent" cannot be shown.
    SignalUnknown,
    /// There is no leak lockout to clear.
    NotLockedOut,
}

impl LeakLockout {
    /// Folds one observed signal into the lockout lifecycle.
    ///
    /// Exhaustive over both the lockout and the signal, with no catch-all arm.
    /// `Unknown` never advances the lifecycle in either direction: it is not
    /// evidence that the tray is wet, and it is certainly not evidence that it
    /// is dry.
    #[must_use]
    pub const fn observe(self, signal: LeakState) -> Self {
        match (self, signal) {
            (_, LeakState::Detected) => Self::Detected,
            (Self::Detected, LeakState::Clear) => Self::AwaitingReset,
            (Self::Detected, LeakState::Unknown) => Self::Detected,
            (Self::AwaitingReset, LeakState::Clear | LeakState::Unknown) => Self::AwaitingReset,
            (Self::Clear, LeakState::Clear | LeakState::Unknown) => Self::Clear,
        }
    }

    /// Applies an explicit operator reset.
    ///
    /// # Errors
    ///
    /// Refuses unless the lockout is awaiting a reset **and** the signal is
    /// currently, positively `Clear`. A silent sensor cannot demonstrate an
    /// absent leak (SAFETY-012).
    pub const fn reset(self, signal: LeakState) -> Result<Self, LeakResetRefused> {
        match (self, signal) {
            (_, LeakState::Detected) => Err(LeakResetRefused::StillDetected),
            (_, LeakState::Unknown) => Err(LeakResetRefused::SignalUnknown),
            (Self::AwaitingReset | Self::Detected, LeakState::Clear) => Ok(Self::Clear),
            (Self::Clear, LeakState::Clear) => Err(LeakResetRefused::NotLockedOut),
        }
    }

    /// Whether this lifecycle blocks watering.
    #[must_use]
    pub const fn blocks(self) -> bool {
        match self {
            Self::Detected | Self::AwaitingReset => true,
            Self::Clear => false,
        }
    }
}

/// The ordered veto. `None` means every check passed.
///
/// Order matters only for *which* reason an operator is shown; every step
/// refuses. The documented sequence (M6-002) is leak, leak-unknown, tank-unknown,
/// tank-low, tank-stale, sample-absent, sample-invalid, sample-stale, daily-cap.
/// Four structural checks bracket it: the actuator (SAFETY-018), a sticky
/// explicit lockout (F-060-41), reconciliation (SAFETY-016), and the required
/// measurements a binding declared (SAFETY-017).
#[must_use]
pub fn safety_gate(inputs: &IrrigationInputs<'_>) -> Option<LockoutReason> {
    // Step 0 (SAFETY-018). A monitoring-only plant is a normal plant with no
    // actuation route, not a degraded one — but it has nothing to water with,
    // so nothing below it can grant anything.
    if inputs.actuator_binding.is_none() {
        return Some(LockoutReason::NoActuator);
    }

    // Step 1-2 (SAFETY-003). The leak check blocks manual watering too: the
    // operator who would click "water anyway" is exactly the person who has not
    // yet looked at the floor. Exhaustive; `Unknown` is classified, not assumed.
    match inputs.leak {
        LeakState::Detected => return Some(LockoutReason::Leak),
        // Absence of a leak reading is not absence of a leak (SAFETY-012). It is
        // `Uncertain` rather than `Leak` because it auto-clears the moment the
        // sensor speaks, whereas a real leak needs a person (F-060-40/41).
        LeakState::Unknown => return Some(LockoutReason::Uncertain),
        LeakState::Clear => {}
    }

    // Step 3 (F-060-41). An explicit-clear lockout outlives the condition that
    // raised it. An auto-clearing one is deliberately *not* consulted: it is
    // re-derived from the current inputs below, which is what "clears when the
    // condition resolves" means — unless the edge is *holding* it for a fixed
    // period, which is how F-060-51's forward clock step keeps a plant locked
    // for one cooldown even though every input looks fine the instant after.
    if let Some(reason) = inputs.active_lockout {
        let held = inputs
            .lockout_held_until
            .is_some_and(|until| inputs.now < until);
        if held || !is_auto_clearable(reason) {
            return Some(reason);
        }
    }

    // Step 4 (SAFETY-016). A device that autonomously watered ninety seconds
    // before reconnecting has that dose in its buffer, not yet in the budget.
    if inputs.reconciling {
        return Some(LockoutReason::Uncertain);
    }

    // Steps 5-7 (SAFETY-004). `Uncertain` for what is not known, `TankLow` for
    // what is measured — the same distinction protocol §5.8 draws between
    // `tank_unknown` and `tank_low`, and for the same reason.
    match inputs.tank {
        None | Some(TankState::Unknown | TankState::Invalid) => {
            return Some(LockoutReason::Uncertain);
        }
        Some(TankState::Level { percent, age }) => {
            if !percent.is_finite() {
                return Some(LockoutReason::Uncertain);
            }
            if percent <= inputs.automation.tank_min_percent {
                return Some(LockoutReason::TankLow);
            }
            // A stale tank level is `Uncertain`, never `StaleData`: `StaleData`
            // is the reason `manual` is allowed to skip, and manual watering is
            // never allowed to skip the reservoir (M6-005).
            if age >= max_sample_age(inputs) {
                return Some(LockoutReason::Uncertain);
            }
        }
    }

    // Steps 8-13 are the sensor-health checks, and the only ones `manual` skips.
    if !inputs.mode.skips_sensor_health() {
        // SAFETY-017's edge half. Requirements are declared by binding role, not
        // inferred: a plant that never bound a scale is unaffected by the
        // absence of one, and a plant that declared pot weight `required` does
        // not water while its scale is silent.
        for required in inputs.required_inputs {
            match required.state {
                RequiredInputState::Missing | RequiredInputState::Invalid => {
                    return Some(LockoutReason::SensorFault);
                }
                RequiredInputState::Stale => return Some(LockoutReason::StaleData),
                RequiredInputState::Usable => {}
            }
        }

        // SAFETY-005. Validity and freshness are separate reasons on purpose:
        // "the probe is broken" and "the probe has not reported" need different
        // operator responses (M6-004).
        match inputs.latest_soil {
            None => return Some(LockoutReason::SensorFault),
            Some(sample) => {
                if !sample.is_valid() {
                    return Some(LockoutReason::SensorFault);
                }
                // `received_at`, never `device_time_ms`: a device with a
                // backwards clock must not be able to make stale data look
                // fresh. The threshold comes from the telemetry cadence and the
                // plant's own policy, and from no power field.
                if sample.is_stale(inputs.now, max_sample_age(inputs)) {
                    return Some(LockoutReason::StaleData);
                }
            }
        }
    }

    // Step 14 (SAFETY-006). Checked here before a dose is chosen; the machine
    // checks again *with the dose included*, so a dose that would cross the cap
    // is never issued at all.
    if !inputs.delivered_last_24h_ml.is_finite()
        || !inputs.automation.max_daily_ml.is_finite()
        || inputs.delivered_last_24h_ml >= inputs.automation.max_daily_ml
    {
        return Some(LockoutReason::DailyLimit);
    }

    None
}

/// The control-freshness threshold this evaluation uses.
///
/// The plant's own `MeasurementPolicy.stale_after` for the control kind when it
/// has one, and otherwise the caller-supplied bound already folded into the
/// policies. **No power field reaches this**: a battery device declaring an
/// 86 400-second wake interval must not thereby make a three-day-old moisture
/// reading actionable (PRD 040 F-040-26, ADR-018 §7). The edge adapter narrows
/// `stale_after_ms` to `min(policy, max(15 min, 3 x telemetry interval))` before
/// building these inputs, so the stricter of the two always wins.
fn max_sample_age(inputs: &IrrigationInputs<'_>) -> chrono::Duration {
    inputs
        .measurement_policies
        .iter()
        .map(|policy| chrono::Duration::milliseconds(i64::from(policy.stale_after_ms)))
        .min()
        .unwrap_or_else(|| chrono::Duration::minutes(15))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
pub(crate) mod fixture {
    //! One fully-permitting set of inputs the tests bend one field at a time.
    use super::super::types::EvaluationMode;
    use super::*;
    use crate::plant::{ActuatorBinding, AutomationPolicy, MeasurementPolicy, SensorBinding};
    use crate::profile::SoilSample;
    use crate::state::IrrigationState;
    use chrono::{DateTime, Duration, TimeZone, Utc};
    use rhizo_mqtt_contract::DeviceId;
    use rhizo_mqtt_contract::payload::{ActuatorKind, MeasurementKind, SensorId};

    pub fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, 30, 12, 0, 0)
            .single()
            .expect("a valid fixed instant")
    }

    pub struct Scene {
        pub state: IrrigationState,
        pub automation: AutomationPolicy,
        pub soil: SoilSample,
        pub pre_dose: SoilSample,
        pub actuator: ActuatorBinding,
        pub bindings: Vec<SensorBinding>,
        pub policies: Vec<MeasurementPolicy>,
        pub required: Vec<super::super::types::RequiredInput>,
    }

    impl Default for Scene {
        fn default() -> Self {
            Self {
                state: IrrigationState::Normal,
                automation: AutomationPolicy::default(),
                soil: SoilSample {
                    moisture_vwc: Some(20.0),
                    received_at: now(),
                },
                pre_dose: SoilSample {
                    moisture_vwc: Some(20.0),
                    received_at: now() - Duration::minutes(30),
                },
                actuator: ActuatorBinding {
                    device_id: DeviceId::parse("plant-node-01").unwrap(),
                    actuator_id: SensorId::parse("pump-0").unwrap(),
                    kind: ActuatorKind::IrrigationPump,
                },
                bindings: Vec::new(),
                policies: vec![MeasurementPolicy {
                    kind: MeasurementKind::SoilMoisture,
                    target_min: Some(28.0),
                    target_max: Some(45.0),
                    warning_low: None,
                    warning_high: None,
                    critical_low: None,
                    critical_high: None,
                    stale_after_ms: 900_000,
                    hysteresis: None,
                    confirm_duration_ms: Some(1_800_000),
                }],
                required: Vec::new(),
            }
        }
    }

    impl Scene {
        /// A permitted scene already in `state`.
        ///
        /// A named constructor rather than `Default::default()` plus a field
        /// assignment: the state is what each transition-table test is *about*,
        /// so it belongs in the construction.
        pub fn at(state: IrrigationState) -> Self {
            Self {
                state,
                ..Self::default()
            }
        }

        pub fn inputs(&self) -> IrrigationInputs<'_> {
            IrrigationInputs {
                now: now(),
                state: &self.state,
                mode: EvaluationMode::Automatic,
                latest_soil: Some(&self.soil),
                pre_dose_soil: Some(&self.pre_dose),
                latest_weight: None,
                pre_dose_weight: None,
                tank: Some(TankState::Level {
                    percent: 70.0,
                    age: Duration::minutes(1),
                }),
                leak: LeakState::Clear,
                sensor_bindings: &self.bindings,
                actuator_binding: Some(&self.actuator),
                measurement_policies: &self.policies,
                automation: &self.automation,
                delivered_last_24h_ml: 0.0,
                doses_this_cycle: 0,
                last_cycle_completed_at: None,
                wait_until: None,
                auto_watering_enabled: true,
                device_online: true,
                dry_duration: Duration::minutes(45),
                reconciling: false,
                required_inputs: &self.required,
                active_lockout: None,
                lockout_held_until: None,
            }
        }
    }
}

/// The gate's own tests, in modules named for the checks they cover.
///
/// The names are the API: the M6 issues quote `cargo test -p rhizo-domain
/// safety_gate::`, `gate::leak`, `gate::tank`, `gate::validity`, and
/// `gate::stale` literally, and each of those filters selects exactly the
/// tests it means because the paths below spell them out.
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod safety_gate {
    use super::super::types::EvaluationMode;
    use super::fixture::{Scene, now};
    use super::*;
    use crate::state::IrrigationState;
    use chrono::Duration;

    /// A permitted set of inputs really is permitted, or every refusal test
    /// below would pass vacuously.
    #[test]
    fn a_healthy_plant_passes_the_gate() {
        let scene = Scene::default();
        assert_eq!(safety_gate(&scene.inputs()), None);
    }

    /// SAFETY-018.
    #[test]
    fn a_plant_with_no_actuator_has_no_actuation_path() {
        let scene = Scene::default();
        let mut inputs = scene.inputs();
        inputs.actuator_binding = None;
        assert_eq!(safety_gate(&inputs), Some(LockoutReason::NoActuator));
    }

    /// SAFETY-006's first half: the cap refuses before a dose is chosen, and a
    /// total the edge cannot read refuses too.
    #[test]
    fn the_daily_cap_blocks() {
        let scene = Scene::default();
        for delivered in [300.0, 301.0, f32::NAN, f32::INFINITY] {
            let mut inputs = scene.inputs();
            inputs.delivered_last_24h_ml = delivered;
            assert_eq!(
                safety_gate(&inputs),
                Some(LockoutReason::DailyLimit),
                "{delivered}"
            );
        }
    }

    /// F-060-41: an explicit lockout survives the condition that raised it, and
    /// F-060-40: an auto-clearing one does not.
    #[test]
    fn sticky_lockouts_survive_and_auto_clearing_ones_do_not() {
        let scene = Scene::default();
        for reason in [
            LockoutReason::Leak,
            LockoutReason::PumpFault,
            LockoutReason::NoDeliveryDetected,
            LockoutReason::MaxDosesReached,
            LockoutReason::Unknown,
        ] {
            let mut inputs = scene.inputs();
            inputs.active_lockout = Some(reason);
            assert_eq!(safety_gate(&inputs), Some(reason), "{reason:?}");
        }
        for reason in [
            LockoutReason::StaleData,
            LockoutReason::SensorFault,
            LockoutReason::TankLow,
            LockoutReason::DailyLimit,
            LockoutReason::Uncertain,
        ] {
            let mut inputs = scene.inputs();
            inputs.active_lockout = Some(reason);
            assert_eq!(
                safety_gate(&inputs),
                None,
                "{reason:?} clears when its condition does"
            );
        }
    }

    /// F-060-51: a held lockout does not clear early, and stops holding the
    /// moment its deadline passes.
    #[test]
    fn a_held_auto_clearing_lockout_stays_until_its_deadline() {
        let scene = Scene::default();
        let mut inputs = scene.inputs();
        inputs.active_lockout = Some(LockoutReason::Uncertain);
        inputs.lockout_held_until = Some(now() + Duration::hours(6));
        assert_eq!(safety_gate(&inputs), Some(LockoutReason::Uncertain));

        let mut inputs = scene.inputs();
        inputs.active_lockout = Some(LockoutReason::Uncertain);
        inputs.lockout_held_until = Some(now());
        assert_eq!(
            safety_gate(&inputs),
            None,
            "the hold ends at its deadline, not one tick later"
        );
    }

    /// SAFETY-016: no dose is granted while the seam is open.
    #[test]
    fn reconciling_blocks() {
        let scene = Scene::default();
        let mut inputs = scene.inputs();
        inputs.reconciling = true;
        assert_eq!(safety_gate(&inputs), Some(LockoutReason::Uncertain));
    }

    /// F-060-12: the gate answers from every state, so nothing can reach it in a
    /// state it refuses to classify.
    #[test]
    fn the_gate_answers_from_every_state() {
        let mut scene = Scene::default();
        for state in [
            IrrigationState::Normal,
            IrrigationState::Drying,
            IrrigationState::DryConfirmed,
            IrrigationState::DoseIssued,
            IrrigationState::WaitForAbsorption,
            IrrigationState::Recheck,
            IrrigationState::Locked,
        ] {
            scene.state = state;
            let mut inputs = scene.inputs();
            inputs.leak = LeakState::Detected;
            assert_eq!(safety_gate(&inputs), Some(LockoutReason::Leak), "{state:?}");
        }
    }

    /// F-060-05, precisely: manual skips the two sensor-health checks and
    /// nothing else.
    #[test]
    fn the_manual_exception_is_exactly_two_checks_wide() {
        let manual = EvaluationMode::ManualRequest { ml: 30.0 };

        // Permitted under sensor fault...
        let mut scene = Scene::default();
        scene.soil.moisture_vwc = None;
        let mut inputs = scene.inputs();
        inputs.mode = manual;
        assert_eq!(safety_gate(&inputs), None);

        // ...and under stale data...
        let mut scene = Scene::default();
        scene.soil.received_at = now() - Duration::days(3);
        let mut inputs = scene.inputs();
        inputs.mode = manual;
        assert_eq!(safety_gate(&inputs), None);

        // ...but blocked by leak, tank, the rolling cap, an explicit lockout,
        // and an incomplete reconciliation.
        let scene = Scene::default();
        /// One bent input and the refusal it must produce.
        type Bend = fn(&mut IrrigationInputs<'_>);
        let cases: [(&str, Bend, LockoutReason); 5] = [
            (
                "leak",
                |i: &mut IrrigationInputs<'_>| i.leak = LeakState::Detected,
                LockoutReason::Leak,
            ),
            (
                "tank",
                |i: &mut IrrigationInputs<'_>| {
                    i.tank = Some(TankState::Level {
                        percent: 1.0,
                        age: Duration::zero(),
                    });
                },
                LockoutReason::TankLow,
            ),
            (
                "cap",
                |i: &mut IrrigationInputs<'_>| i.delivered_last_24h_ml = 300.0,
                LockoutReason::DailyLimit,
            ),
            (
                "explicit lockout",
                |i: &mut IrrigationInputs<'_>| {
                    i.active_lockout = Some(LockoutReason::NoDeliveryDetected);
                },
                LockoutReason::NoDeliveryDetected,
            ),
            (
                "reconciling",
                |i: &mut IrrigationInputs<'_>| i.reconciling = true,
                LockoutReason::Uncertain,
            ),
        ];
        for (label, mutate, expected) in cases {
            let mut inputs = scene.inputs();
            inputs.mode = manual;
            mutate(&mut inputs);
            assert_eq!(safety_gate(&inputs), Some(expected), "manual + {label}");
        }
    }

    /// The compile-time half of SAFETY-012, checked as a test because a review
    /// habit is not a mechanism.
    ///
    /// A `_ =>` arm in this file would let a new `LeakState`, `TankState`,
    /// `EvaluationMode`, or `RequiredInputState` variant fall through into
    /// permission without breaking the build. There is none, and this fails if
    /// one appears.
    #[test]
    fn no_catch_all_arm_on_a_safety_match() {
        let source = include_str!("gate.rs");
        let offenders: Vec<(usize, &str)> = source
            .lines()
            .enumerate()
            .filter(|(_, line)| {
                let trimmed = line.trim_start();
                !trimmed.starts_with("//") && trimmed.starts_with("_ =>")
            })
            .map(|(index, line)| (index + 1, line.trim()))
            .collect();
        assert!(
            offenders.is_empty(),
            "the safety gate must classify every variant explicitly: {offenders:?}"
        );
    }

    /// SAFETY-003 (M6-003). Reached by `cargo test -p rhizo-domain gate::leak`.
    mod leak {
        use super::*;

        /// A detected leak blocks every mode, including manual.
        #[test]
        fn detected_blocks_all_modes() {
            let scene = Scene::default();
            for mode in [
                EvaluationMode::Automatic,
                EvaluationMode::ManualRequest { ml: 30.0 },
                EvaluationMode::RecommendedRequest { ml: 30.0 },
            ] {
                let mut inputs = scene.inputs();
                inputs.leak = LeakState::Detected;
                inputs.mode = mode;
                assert_eq!(safety_gate(&inputs), Some(LockoutReason::Leak), "{mode:?}");
            }
        }

        /// SAFETY-012: an unknown leak is not a clear one.
        #[test]
        fn unknown_blocks_all_modes() {
            let scene = Scene::default();
            for mode in [
                EvaluationMode::Automatic,
                EvaluationMode::ManualRequest { ml: 30.0 },
                EvaluationMode::RecommendedRequest { ml: 30.0 },
            ] {
                let mut inputs = scene.inputs();
                inputs.leak = LeakState::Unknown;
                inputs.mode = mode;
                assert_eq!(
                    safety_gate(&inputs),
                    Some(LockoutReason::Uncertain),
                    "{mode:?}"
                );
            }
        }

        /// M6-003's clearing asymmetry, as a state machine rather than a habit.
        #[test]
        fn a_cleared_signal_awaits_a_reset_rather_than_clearing_itself() {
            let mut lockout = LeakLockout::Clear;
            assert!(!lockout.blocks());

            lockout = lockout.observe(LeakState::Detected);
            assert_eq!(lockout, LeakLockout::Detected);
            assert!(lockout.blocks());

            lockout = lockout.observe(LeakState::Clear);
            assert_eq!(
                lockout,
                LeakLockout::AwaitingReset,
                "the tray drying out is not evidence the joint was fixed"
            );
            assert!(lockout.blocks());

            // Days of clear readings do not add up to a reset.
            for _ in 0..100 {
                lockout = lockout.observe(LeakState::Clear);
            }
            assert_eq!(lockout, LeakLockout::AwaitingReset);

            assert_eq!(lockout.reset(LeakState::Clear), Ok(LeakLockout::Clear));
        }

        /// A reset is refused unless the signal is positively absent.
        #[test]
        fn a_reset_needs_the_signal_to_be_positively_absent() {
            assert_eq!(
                LeakLockout::Detected.reset(LeakState::Detected),
                Err(LeakResetRefused::StillDetected)
            );
            assert_eq!(
                LeakLockout::AwaitingReset.reset(LeakState::Detected),
                Err(LeakResetRefused::StillDetected)
            );
            assert_eq!(
                LeakLockout::AwaitingReset.reset(LeakState::Unknown),
                Err(LeakResetRefused::SignalUnknown),
                "a silent sensor cannot demonstrate an absent leak"
            );
            assert_eq!(
                LeakLockout::Clear.reset(LeakState::Clear),
                Err(LeakResetRefused::NotLockedOut)
            );
        }

        /// An unknown signal never advances the lifecycle in either direction.
        #[test]
        fn an_unknown_signal_moves_the_lifecycle_nowhere() {
            assert_eq!(
                LeakLockout::Detected.observe(LeakState::Unknown),
                LeakLockout::Detected
            );
            assert_eq!(
                LeakLockout::AwaitingReset.observe(LeakState::Unknown),
                LeakLockout::AwaitingReset
            );
            assert_eq!(
                LeakLockout::Clear.observe(LeakState::Unknown),
                LeakLockout::Clear
            );
        }
    }

    /// SAFETY-004 (M6-003). Reached by `cargo test -p rhizo-domain gate::tank`.
    mod tank {
        use super::*;

        #[test]
        fn unknown_or_invalid_blocks() {
            let scene = Scene::default();
            for state in [None, Some(TankState::Unknown), Some(TankState::Invalid)] {
                let mut inputs = scene.inputs();
                inputs.tank = state;
                assert_eq!(
                    safety_gate(&inputs),
                    Some(LockoutReason::Uncertain),
                    "{state:?}"
                );
            }
            let mut inputs = scene.inputs();
            inputs.tank = Some(TankState::Level {
                percent: f64::NAN,
                age: Duration::minutes(1),
            });
            assert_eq!(
                safety_gate(&inputs),
                Some(LockoutReason::Uncertain),
                "a non-finite level is unreadable, not full"
            );
        }

        #[test]
        fn at_or_below_the_floor_blocks() {
            let scene = Scene::default();
            for percent in [0.0, 10.0, 15.0] {
                let mut inputs = scene.inputs();
                inputs.tank = Some(TankState::Level {
                    percent,
                    age: Duration::minutes(1),
                });
                assert_eq!(
                    safety_gate(&inputs),
                    Some(LockoutReason::TankLow),
                    "{percent}% is at or below the 15% floor"
                );
            }
        }

        /// A refill clears the tank lockout on its own — the asymmetry with
        /// leak, which does not.
        #[test]
        fn a_refill_clears_it_automatically() {
            let scene = Scene::default();
            let mut inputs = scene.inputs();
            inputs.active_lockout = Some(LockoutReason::TankLow);
            inputs.tank = Some(TankState::Level {
                percent: 5.0,
                age: Duration::minutes(1),
            });
            assert_eq!(safety_gate(&inputs), Some(LockoutReason::TankLow));

            let mut inputs = scene.inputs();
            inputs.active_lockout = Some(LockoutReason::TankLow);
            inputs.tank = Some(TankState::Level {
                percent: 80.0,
                age: Duration::minutes(1),
            });
            assert_eq!(
                safety_gate(&inputs),
                None,
                "a refilled reservoir needs no operator to say so"
            );
        }

        /// A stale tank blocks, and blocks manual watering too — which is why it
        /// is `Uncertain` and not `StaleData`.
        #[test]
        fn a_stale_level_blocks_every_mode() {
            let scene = Scene::default();
            for mode in [
                EvaluationMode::Automatic,
                EvaluationMode::ManualRequest { ml: 30.0 },
                EvaluationMode::RecommendedRequest { ml: 30.0 },
            ] {
                let mut inputs = scene.inputs();
                inputs.mode = mode;
                inputs.tank = Some(TankState::Level {
                    percent: 70.0,
                    age: Duration::minutes(20),
                });
                assert_eq!(
                    safety_gate(&inputs),
                    Some(LockoutReason::Uncertain),
                    "{mode:?}"
                );
            }
        }
    }

    /// SAFETY-005's validity half (M6-004). Reached by
    /// `cargo test -p rhizo-domain gate::validity`.
    mod validity {
        use super::super::super::types::{RequiredInput, RequiredInputState};
        use super::*;
        use rhizo_mqtt_contract::payload::MeasurementKind;

        #[test]
        fn an_absent_sample_is_a_sensor_fault() {
            let scene = Scene::default();
            let mut inputs = scene.inputs();
            inputs.latest_soil = None;
            assert_eq!(safety_gate(&inputs), Some(LockoutReason::SensorFault));
        }

        #[test]
        fn an_out_of_range_or_non_finite_sample_is_a_sensor_fault() {
            let mut scene = Scene::default();
            for bad in [
                Some(f64::NAN),
                Some(f64::INFINITY),
                Some(-1.0),
                Some(101.0),
                None,
            ] {
                scene.soil.moisture_vwc = bad;
                assert_eq!(
                    safety_gate(&scene.inputs()),
                    Some(LockoutReason::SensorFault),
                    "{bad:?}"
                );
            }
        }

        /// The two reasons are distinct, because the operator responses are.
        #[test]
        fn sensor_fault_and_stale_data_are_different_answers() {
            assert_ne!(LockoutReason::SensorFault, LockoutReason::StaleData);
        }

        /// A valid sample clears it with no operator action.
        #[test]
        fn a_valid_sample_clears_it_automatically() {
            let mut scene = Scene::default();
            scene.soil.moisture_vwc = None;
            let mut inputs = scene.inputs();
            inputs.active_lockout = Some(LockoutReason::SensorFault);
            assert_eq!(safety_gate(&inputs), Some(LockoutReason::SensorFault));

            scene.soil.moisture_vwc = Some(20.0);
            let mut inputs = scene.inputs();
            inputs.active_lockout = Some(LockoutReason::SensorFault);
            assert_eq!(safety_gate(&inputs), None);
        }

        /// SAFETY-017's edge half, and its converse.
        #[test]
        fn required_measurements_block_and_an_undeclared_one_does_not() {
            let scene = Scene::default();
            for (state, expected) in [
                (RequiredInputState::Missing, LockoutReason::SensorFault),
                (RequiredInputState::Invalid, LockoutReason::SensorFault),
                (RequiredInputState::Stale, LockoutReason::StaleData),
            ] {
                let required = vec![RequiredInput {
                    kind: MeasurementKind::PotWeight,
                    state,
                }];
                let mut inputs = scene.inputs();
                inputs.required_inputs = &required;
                assert_eq!(safety_gate(&inputs), Some(expected), "{state:?}");
            }
            let required = vec![RequiredInput {
                kind: MeasurementKind::PotWeight,
                state: RequiredInputState::Usable,
            }];
            let mut inputs = scene.inputs();
            inputs.required_inputs = &required;
            assert_eq!(
                safety_gate(&inputs),
                None,
                "a plant that never declared a requirement is not blocked by it"
            );
        }
    }

    /// SAFETY-005's freshness half (M6-005). Reached by
    /// `cargo test -p rhizo-domain gate::stale`.
    mod stale {
        use super::*;

        /// The boundary is inclusive: age == limit is stale.
        #[test]
        fn blocks_automatic_at_the_exact_boundary() {
            let mut scene = Scene::default();
            scene.soil.received_at = now() - Duration::milliseconds(899_999);
            assert_eq!(safety_gate(&scene.inputs()), None);
            scene.soil.received_at = now() - Duration::milliseconds(900_000);
            assert_eq!(safety_gate(&scene.inputs()), Some(LockoutReason::StaleData));
        }

        /// SAFETY-005: the age is measured from **edge receipt**, so a device
        /// clock running hours ahead changes nothing at all. `SoilSample`
        /// carries only `received_at`; there is deliberately no device timestamp
        /// for the gate to prefer.
        #[test]
        fn a_wrong_device_clock_changes_nothing() {
            let mut scene = Scene::default();
            scene.soil.received_at = now() - Duration::hours(4);
            assert_eq!(safety_gate(&scene.inputs()), Some(LockoutReason::StaleData));
        }

        /// ADR-018 §7: no power field reaches the control-freshness threshold.
        /// The gate takes its limit from the plant's `MeasurementPolicy` alone —
        /// there is no field on `IrrigationInputs` a device could widen — so a
        /// battery device declaring an 86 400-second wake interval is blocked on
        /// a stale sample exactly like any other device.
        #[test]
        fn a_long_declared_wake_interval_cannot_widen_the_window() {
            let mut scene = Scene::default();
            scene.soil.received_at = now() - Duration::hours(20);
            assert_eq!(safety_gate(&scene.inputs()), Some(LockoutReason::StaleData));
            let source = include_str!("types.rs");
            assert!(
                !source.contains("wake_interval"),
                "no power field may appear in the irrigation inputs"
            );
        }

        /// Manual watering is permitted on stale data; nothing else is.
        #[test]
        fn manual_is_permitted_and_automatic_is_not() {
            let mut scene = Scene::default();
            scene.soil.received_at = now() - Duration::hours(4);
            assert_eq!(safety_gate(&scene.inputs()), Some(LockoutReason::StaleData));

            let mut inputs = scene.inputs();
            inputs.mode = EvaluationMode::ManualRequest { ml: 30.0 };
            assert_eq!(safety_gate(&inputs), None);
        }

        /// Fresh data clears it with no operator action.
        #[test]
        fn fresh_data_clears_it_automatically() {
            let mut scene = Scene::default();
            scene.soil.received_at = now();
            let mut inputs = scene.inputs();
            inputs.active_lockout = Some(LockoutReason::StaleData);
            assert_eq!(safety_gate(&inputs), None);
        }
    }
}
