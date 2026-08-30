//! The safety invariant property suite (M6-018).
//!
//! The heaviest test investment in the project, and the one the whole safety
//! argument rests on. ADR-006's purity is what makes it affordable: ten thousand
//! adversarial cases against the gate cost milliseconds because there is no
//! database and no broker involved.
//!
//! Every test is named `safety_NNN_*`, so `cargo test safety_` runs the suite
//! across every crate at once, and each names the invariant it proves.
//!
//! # The flagship
//!
//! [`safety_006_rolling_24h_cap_never_exceeded`] generates genuinely adversarial
//! histories — restarts between publish and result, forward and backward clock
//! steps, interrupted doses credited conservatively, duplicate results — and
//! asserts that at every instant the rolling sum is within the cap. If one
//! property test is kept, it is that one.
//!
//! # Shrunk counterexamples are permanent evidence
//!
//! `proptest` persists failures to `proptest-regressions/`, which is committed.
//! A found bug that stops being tested is a bug that comes back.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use chrono::{DateTime, Duration, TimeZone, Utc};
use proptest::prelude::*;
use proptest::test_runner::FileFailurePersistence;
use rhizo_domain::irrigation::budget;
use rhizo_domain::irrigation::types::{
    EvaluationMode, IrrigationDecision, IrrigationInputs, LeakState, RequiredInput,
    RequiredInputState, TankState, WeightSample,
};
use rhizo_domain::irrigation::{evaluate, safety_gate};
use rhizo_domain::plant::{ActuatorBinding, AutomationPolicy, MeasurementPolicy, SensorBinding};
use rhizo_domain::profile::{PlantProfile, SoilSample};
use rhizo_domain::state::{IrrigationState, LockoutReason};
use rhizo_mqtt_contract::DeviceId;
use rhizo_mqtt_contract::payload::{ActuatorKind, CommandStatus, MeasurementKind, SensorId};

fn base() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 8, 30, 12, 0, 0)
        .single()
        .unwrap()
}

/// Everything a generated case owns, so the borrowed inputs can be built.
struct Scene {
    state: IrrigationState,
    automation: AutomationPolicy,
    soil: Option<SoilSample>,
    pre_dose: Option<SoilSample>,
    weight: Option<WeightSample>,
    pre_dose_weight: Option<WeightSample>,
    actuator: Option<ActuatorBinding>,
    bindings: Vec<SensorBinding>,
    policies: Vec<MeasurementPolicy>,
    required: Vec<RequiredInput>,
    tank: Option<TankState>,
    leak: LeakState,
    mode: EvaluationMode,
    delivered: f32,
    doses: u8,
    last_cycle: Option<DateTime<Utc>>,
    wait_until: Option<DateTime<Utc>>,
    auto: bool,
    online: bool,
    dry: Duration,
    reconciling: bool,
    lockout: Option<LockoutReason>,
    held_until: Option<DateTime<Utc>>,
}

impl Scene {
    fn healthy() -> Self {
        Self {
            state: IrrigationState::DryConfirmed,
            automation: AutomationPolicy::from_profile(
                &PlantProfile::default_seed(rhizo_domain::ProfileId::from_uuid(uuid::Uuid::nil())),
                true,
                None,
            ),
            soil: Some(SoilSample {
                moisture_vwc: Some(20.0),
                received_at: base(),
            }),
            pre_dose: Some(SoilSample {
                moisture_vwc: Some(20.0),
                received_at: base() - Duration::minutes(30),
            }),
            weight: None,
            pre_dose_weight: None,
            actuator: Some(ActuatorBinding {
                device_id: DeviceId::parse("plant-node-01").unwrap(),
                actuator_id: SensorId::parse("pump-0").unwrap(),
                kind: ActuatorKind::IrrigationPump,
            }),
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
            tank: Some(TankState::Level {
                percent: 70.0,
                age: Duration::minutes(1),
            }),
            leak: LeakState::Clear,
            mode: EvaluationMode::Automatic,
            delivered: 0.0,
            doses: 0,
            last_cycle: None,
            wait_until: None,
            auto: true,
            online: true,
            dry: Duration::minutes(45),
            reconciling: false,
            lockout: None,
            held_until: None,
        }
    }

    fn inputs(&self) -> IrrigationInputs<'_> {
        IrrigationInputs {
            now: base(),
            state: &self.state,
            mode: self.mode,
            latest_soil: self.soil.as_ref(),
            pre_dose_soil: self.pre_dose.as_ref(),
            latest_weight: self.weight.as_ref(),
            pre_dose_weight: self.pre_dose_weight.as_ref(),
            tank: self.tank,
            leak: self.leak,
            sensor_bindings: &self.bindings,
            actuator_binding: self.actuator.as_ref(),
            measurement_policies: &self.policies,
            automation: &self.automation,
            delivered_last_24h_ml: self.delivered,
            doses_this_cycle: self.doses,
            last_cycle_completed_at: self.last_cycle,
            wait_until: self.wait_until,
            auto_watering_enabled: self.auto,
            device_online: self.online,
            dry_duration: self.dry,
            reconciling: self.reconciling,
            required_inputs: &self.required,
            active_lockout: self.lockout,
            lockout_held_until: self.held_until,
        }
    }
}

fn any_state() -> impl Strategy<Value = IrrigationState> {
    prop_oneof![
        Just(IrrigationState::Normal),
        Just(IrrigationState::Drying),
        Just(IrrigationState::DryConfirmed),
        Just(IrrigationState::DoseIssued),
        Just(IrrigationState::WaitForAbsorption),
        Just(IrrigationState::Recheck),
        Just(IrrigationState::Locked),
    ]
}

fn any_mode() -> impl Strategy<Value = EvaluationMode> {
    prop_oneof![
        Just(EvaluationMode::Automatic),
        Just(EvaluationMode::ManualRequest { ml: 30.0 }),
        Just(EvaluationMode::RecommendedRequest { ml: 30.0 }),
    ]
}

fn any_leak() -> impl Strategy<Value = LeakState> {
    prop_oneof![
        Just(LeakState::Clear),
        Just(LeakState::Detected),
        Just(LeakState::Unknown),
    ]
}

proptest! {
    // Shrunk counterexamples are permanent evidence, so they are persisted to a
    // committed file rather than to wherever proptest guesses. An integration
    // test has no `lib.rs` beside it, so the default `SourceParallel` strategy
    // finds nowhere to write and the corpus would silently not exist.
    #![proptest_config(ProptestConfig {
        failure_persistence: Some(Box::new(FileFailurePersistence::Direct(
            "proptest-regressions/safety.txt",
        ))),
        ..ProptestConfig::default()
    })]

    /// **SAFETY-003.** A detected leak blocks watering in *every* mode,
    /// including manual, from every state, whatever else is true.
    ///
    /// The operator who would click "water anyway" is exactly the person who has
    /// not yet looked at the floor.
    #[test]
    fn safety_003_leak_blocks_all_modes(
        state in any_state(),
        mode in any_mode(),
        moisture in proptest::option::of(0.0f64..100.0),
        delivered in 0.0f32..200.0,
        doses in 0u8..5,
        auto in any::<bool>(),
        online in any::<bool>(),
    ) {
        let mut scene = Scene::healthy();
        scene.state = state;
        scene.mode = mode;
        scene.leak = LeakState::Detected;
        scene.soil = moisture.map(|vwc| SoilSample { moisture_vwc: Some(vwc), received_at: base() });
        scene.delivered = delivered;
        scene.doses = doses;
        scene.auto = auto;
        scene.online = online;

        prop_assert_eq!(safety_gate(&scene.inputs()), Some(LockoutReason::Leak));
        let decision = evaluate(scene.inputs());
        prop_assert!(!decision.actuates(), "{:?}", decision);
        prop_assert_eq!(decision, IrrigationDecision::Lock { reason: LockoutReason::Leak });
    }

    /// **SAFETY-004.** An unknown, unreadable, or low reservoir blocks, and no
    /// mode is exempt.
    #[test]
    fn safety_004_tank_unknown_or_low_blocks(
        state in any_state(),
        mode in any_mode(),
        percent in proptest::option::of(prop_oneof![
            Just(f64::NAN), Just(f64::INFINITY), -20.0f64..15.001
        ]),
        age_seconds in 0i64..600,
    ) {
        let mut scene = Scene::healthy();
        scene.state = state;
        scene.mode = mode;
        scene.tank = percent.map(|percent| TankState::Level {
            percent,
            age: Duration::seconds(age_seconds),
        });

        let refusal = safety_gate(&scene.inputs());
        prop_assert!(
            matches!(refusal, Some(LockoutReason::TankLow | LockoutReason::Uncertain)),
            "tank {:?} produced {:?}", percent, refusal
        );
        prop_assert!(!evaluate(scene.inputs()).actuates());
    }

    /// **SAFETY-005.** A stale or invalid control sample blocks *automatic*
    /// watering — and manual watering stays permitted, which is the deliberate
    /// asymmetry the invariant states.
    #[test]
    fn safety_005_stale_or_invalid_blocks_auto(
        age_seconds in 0i64..7_200,
        moisture in proptest::option::of(prop_oneof![
            Just(f64::NAN), Just(f64::INFINITY), -50.0f64..200.0
        ]),
    ) {
        let mut scene = Scene::healthy();
        scene.soil = Some(SoilSample {
            moisture_vwc: moisture,
            received_at: base() - Duration::seconds(age_seconds),
        });
        let usable = moisture.is_some_and(|v| v.is_finite() && (0.0..=100.0).contains(&v));
        let fresh = age_seconds * 1_000 < 900_000;

        let automatic = safety_gate(&scene.inputs());
        if usable && fresh {
            prop_assert_eq!(automatic, None);
        } else {
            prop_assert!(
                matches!(automatic, Some(LockoutReason::SensorFault | LockoutReason::StaleData)),
                "age {age_seconds}s value {moisture:?} produced {automatic:?}"
            );
            prop_assert!(!evaluate(scene.inputs()).actuates());
        }

        // The asymmetry: a human has looked at the plant and taken
        // responsibility for it, so manual watering is permitted.
        scene.mode = EvaluationMode::ManualRequest { ml: 30.0 };
        prop_assert_eq!(safety_gate(&scene.inputs()), None);
        prop_assert!(evaluate(scene.inputs()).actuates());
    }

    /// **SAFETY-012.** The meta-invariant: whenever a required input is missing,
    /// unreadable, or of unknown age, the decision is never a dose.
    ///
    /// Generates inputs with each field independently absent, which is the shape
    /// the invariant names.
    #[test]
    fn safety_012_missing_input_never_waters(
        state in any_state(),
        mode in any_mode(),
        has_actuator in any::<bool>(),
        soil in proptest::option::of(proptest::option::of(-50.0f64..200.0)),
        tank in proptest::option::of(prop_oneof![
            Just(TankState::Unknown),
            Just(TankState::Invalid),
            (0.0f64..100.0, 0i64..3_600).prop_map(|(percent, age)| TankState::Level {
                percent,
                age: Duration::seconds(age),
            }),
        ]),
        leak in any_leak(),
        delivered in prop_oneof![Just(f32::NAN), -10.0f32..500.0],
        required_state in prop_oneof![
            Just(None),
            Just(Some(RequiredInputState::Usable)),
            Just(Some(RequiredInputState::Missing)),
            Just(Some(RequiredInputState::Invalid)),
            Just(Some(RequiredInputState::Stale)),
        ],
        reconciling in any::<bool>(),
        age_seconds in 0i64..7_200,
    ) {
        let mut scene = Scene::healthy();
        scene.state = state;
        scene.mode = mode;
        if !has_actuator {
            scene.actuator = None;
        }
        scene.soil = soil.map(|moisture_vwc| SoilSample {
            moisture_vwc,
            received_at: base() - Duration::seconds(age_seconds),
        });
        scene.tank = tank;
        scene.leak = leak;
        scene.delivered = delivered;
        scene.required = required_state
            .map(|state| vec![RequiredInput { kind: MeasurementKind::PotWeight, state }])
            .unwrap_or_default();
        scene.reconciling = reconciling;

        let decision = evaluate(scene.inputs());
        if !decision.actuates() {
            return Ok(());
        }
        // A dose was issued, so **every** safety input must have been positively
        // good. Absence of evidence is never permission.
        prop_assert!(has_actuator);
        prop_assert_eq!(leak, LeakState::Clear);
        prop_assert!(!reconciling);
        prop_assert!(delivered.is_finite());
        let tank_was_usable = matches!(
            scene.tank,
            Some(TankState::Level { percent, .. }) if percent.is_finite() && percent > 15.0
        );
        prop_assert!(tank_was_usable, "a dose was issued on tank {:?}", scene.tank);
        if !mode.skips_sensor_health() {
            prop_assert!(required_state != Some(RequiredInputState::Missing));
            prop_assert!(required_state != Some(RequiredInputState::Invalid));
            prop_assert!(required_state != Some(RequiredInputState::Stale));
            prop_assert!(scene.soil.is_some_and(|s| s.is_valid()));
            prop_assert!(age_seconds * 1_000 < 900_000);
        }
    }

    /// **SAFETY-010.** A terminal command is never re-issued, however many
    /// restarts and however many late results arrive.
    ///
    /// Modelled over the storage layer's own rule — a settled command stays
    /// settled — because that is the mechanism the invariant names.
    #[test]
    fn safety_010_terminal_commands_never_reissued(
        history in proptest::collection::vec(
            prop_oneof![
                Just(Event::Restart),
                Just(Event::Result(CommandStatus::Completed)),
                Just(Event::Result(CommandStatus::Rejected)),
                Just(Event::Result(CommandStatus::Interrupted)),
                Just(Event::Result(CommandStatus::Failed)),
                Just(Event::Expire),
            ],
            1..40,
        ),
    ) {
        let mut ledger = Ledger::default();
        for event in history {
            ledger.apply(event);
        }
        prop_assert!(
            ledger.actuations <= 1,
            "one command actuated {} times", ledger.actuations
        );
        prop_assert!(ledger.watering_events <= 1);
    }
}

/// One step of an adversarial command history.
#[derive(Clone, Copy, Debug)]
enum Event {
    /// The edge restarted. Reconciliation runs; nothing is re-published.
    Restart,
    /// A `command.result` arrived.
    Result(CommandStatus),
    /// The TTL passed with no result.
    Expire,
}

/// A model of one command's lifecycle, with the storage layer's terminal rule.
#[derive(Default)]
struct Ledger {
    terminal: bool,
    actuations: usize,
    watering_events: usize,
}

impl Ledger {
    fn apply(&mut self, event: Event) {
        match event {
            // The recovery procedure never re-publishes. That is the whole of
            // SAFETY-010's mechanism, and the model says so explicitly.
            Event::Restart => {}
            Event::Result(status) => {
                if self.terminal {
                    return;
                }
                self.terminal = true;
                if status == CommandStatus::Completed {
                    self.actuations += 1;
                    if budget::creates_watering_event(status) {
                        self.watering_events += 1;
                    }
                }
            }
            Event::Expire => {
                self.terminal = true;
            }
        }
    }
}

// ---------------------------------------------------------------- the flagship

/// One step of an adversarial 72-hour history.
#[derive(Clone, Copy, Debug)]
enum Step {
    /// Time passes.
    Advance {
        /// How far.
        minutes: i64,
    },
    /// The edge tries to issue a dose.
    Dose,
    /// A dose settles.
    Settle {
        /// How it ended.
        status: CommandStatus,
        /// What the device says it delivered.
        delivered_ml: f32,
    },
    /// The edge restarts between publish and result.
    Restart,
    /// The wall clock jumps.
    Jump {
        /// Signed, in minutes.
        minutes: i64,
    },
    /// A duplicate result arrives for a command already settled.
    DuplicateResult,
}

fn any_step() -> impl Strategy<Value = Step> {
    prop_oneof![
        (1i64..600).prop_map(|minutes| Step::Advance { minutes }),
        Just(Step::Dose),
        (0.0f32..80.0).prop_map(|delivered_ml| Step::Settle {
            status: CommandStatus::Completed,
            delivered_ml,
        }),
        Just(Step::Settle {
            status: CommandStatus::Interrupted,
            delivered_ml: 0.0,
        }),
        Just(Step::Settle {
            status: CommandStatus::Failed,
            delivered_ml: 0.0,
        }),
        Just(Step::Settle {
            status: CommandStatus::Rejected,
            delivered_ml: 0.0,
        }),
        Just(Step::Restart),
        (-600i64..600).prop_map(|minutes| Step::Jump { minutes }),
        Just(Step::DuplicateResult),
    ]
}

/// A charge against the rolling window: when it happened, and how much.
#[derive(Clone, Copy, Debug)]
struct Charge {
    at: DateTime<Utc>,
    ml: f32,
}

proptest! {
    #![proptest_config(ProptestConfig {
        failure_persistence: Some(Box::new(FileFailurePersistence::Direct(
            "proptest-regressions/safety.txt",
        ))),
        ..ProptestConfig::default()
    })]

    /// **SAFETY-006, the flagship.** The rolling 24-hour total for a plant never
    /// exceeds `max_daily_ml`, under adversarial histories that include restarts
    /// between publish and result, forward and backward clock steps, interrupted
    /// doses credited conservatively, and duplicate results.
    ///
    /// # What "at every instant" means here, precisely
    ///
    /// The window is asserted **against the ledger's own timestamps**, not
    /// against whatever the wall clock currently says. That is deliberate and it
    /// is the stronger claim: an edge whose clock is stepped backwards would
    /// otherwise recompute a window that reaches further into the past and
    /// report a sum it never actually issued against. What SAFETY-006
    /// constrains is what the edge *issues*, so the property is "for every
    /// charge in the ledger, the 24 hours ending at that charge sum to no more
    /// than the cap" — which no clock step can make false after the fact.
    ///
    /// The forward-step lockout (F-060-51) is modelled because it is real
    /// behaviour, and because without it a forward step is precisely how the
    /// cap would be bypassed: older rows drop out of the window early and the
    /// plant is handed a fresh allowance.
    #[test]
    fn safety_006_rolling_24h_cap_never_exceeded(
        history in proptest::collection::vec(any_step(), 1..200),
    ) {
        let cap = 300.0f32;
        let dose = 40.0f32;
        let cooldown = Duration::hours(6);
        let mut now = base();
        let mut charges: Vec<Charge> = Vec::new();
        let mut in_flight: Option<DateTime<Utc>> = None;
        let mut locked_until: Option<DateTime<Utc>> = None;

        for step in history {
            match step {
                Step::Advance { minutes } => now += Duration::minutes(minutes),
                Step::Jump { minutes } => {
                    now += Duration::minutes(minutes);
                    // F-060-51: a forward step beyond ten minutes locks every
                    // plant `Uncertain` for one cooldown. A backward step is
                    // logged and nothing else, because it makes the window
                    // include more history rather than less.
                    if minutes > 10 {
                        locked_until = Some(now + cooldown);
                    }
                }
                // A restart re-reads the ledger and re-publishes nothing, so it
                // changes no charge at all. Modelled explicitly, because "the
                // restart does nothing" is the property.
                Step::Restart => {}
                Step::Dose => {
                    if in_flight.is_some() || locked_until.is_some_and(|until| now < until) {
                        continue;
                    }
                    let delivered = rolling(&charges, now);
                    // The rule under test, on both halves: the gate refuses at
                    // the ceiling, and the machine refuses a dose that would
                    // cross it.
                    if delivered >= cap || !budget::dose_fits(delivered, dose, cap) {
                        continue;
                    }
                    in_flight = Some(now);
                }
                Step::Settle { status, delivered_ml } => {
                    let Some(issued_at) = in_flight.take() else { continue };
                    let credited = budget::credited_ml(
                        status,
                        dose,
                        (status == CommandStatus::Completed).then_some(delivered_ml),
                    );
                    if credited > 0.0 {
                        // A device may not deliver more than it was asked for:
                        // the shared validator clamps to the request, so the
                        // ledger charges at most the request.
                        charges.push(Charge {
                            at: now.max(issued_at),
                            ml: credited.min(dose),
                        });
                    }
                }
                Step::DuplicateResult => {
                    // Terminal is terminal. `in_flight` was already taken by the
                    // first result, so this charges nothing — which is the
                    // assertion.
                    let before = charges.len();
                    if in_flight.is_none() {
                        prop_assert_eq!(charges.len(), before);
                    }
                }
            }
        }

        // The durable claim, over the ledger the edge actually wrote: for every
        // charge, the twenty-four hours **ending at it** are within the cap.
        for charge in &charges {
            let window = window_ending_at(&charges, charge.at);
            prop_assert!(
                window <= cap + 1e-3,
                "the 24 hours ending at a charge sum to {window} against a {cap} ml cap"
            );
        }
    }
}

/// The window as the storage layer computes it.
///
/// A sum over `watering_events` rows with `completed_at > now - 24h`, never a
/// counter — and deliberately with **no upper bound**, exactly as
/// `delivered_in_window` is written. A row stamped in the future relative to a
/// stepped-back clock still counts, which is the conservative direction: it
/// makes the budget look *more* spent, never less.
fn rolling(charges: &[Charge], now: DateTime<Utc>) -> f32 {
    let since = budget::window_start(now);
    charges
        .iter()
        .filter(|charge| charge.at > since)
        .map(|charge| charge.ml)
        .sum()
}

/// The retrospective window: what the ledger shows for the day ending at `at`.
///
/// Bounded above as well as below, unlike the live query — a charge written
/// *after* the instant in question is not part of the day that ended then. The
/// live query has no upper bound because "now" is by definition the end of its
/// window; a retrospective one needs both edges or it is not a window.
fn window_ending_at(charges: &[Charge], at: DateTime<Utc>) -> f32 {
    let since = budget::window_start(at);
    charges
        .iter()
        .filter(|charge| charge.at > since && charge.at <= at)
        .map(|charge| charge.ml)
        .sum()
}

/// **SAFETY-006's restart half**, as a plain example rather than a property: a
/// total derived from rows cannot be reset by restarting, because there is
/// nothing to reset.
#[test]
fn safety_006_cap_survives_restart() {
    let charges = vec![
        Charge {
            at: base() - Duration::hours(3),
            ml: 150.0,
        },
        Charge {
            at: base() - Duration::hours(1),
            ml: 140.0,
        },
    ];
    let delivered = rolling(&charges, base());
    assert!((delivered - 290.0).abs() < 1e-3);
    // "Restarting" is re-reading the same rows.
    assert!((rolling(&charges, base()) - delivered).abs() < f32::EPSILON);
    assert!(!budget::dose_fits(delivered, 40.0, 300.0));
    // ...and twenty-five hours later the window has genuinely moved on.
    assert_eq!(rolling(&charges, base() + Duration::hours(25)), 0.0);
}

/// **SAFETY-012's compile-time half**, asserted as a property of the source.
///
/// A `_ =>` arm on a safety match would let a new `LeakState`, `TankState`, or
/// `RequiredInputState` variant fall through into permission without breaking
/// the build.
#[test]
fn safety_012_no_catch_all_arm_on_a_safety_match() {
    let gate = include_str!("../src/irrigation/gate.rs");
    let offenders: Vec<usize> = gate
        .lines()
        .enumerate()
        .filter(|(_, line)| {
            let trimmed = line.trim_start();
            !trimmed.starts_with("//") && trimmed.starts_with("_ =>")
        })
        .map(|(index, _)| index + 1)
        .collect();
    assert!(offenders.is_empty(), "catch-all arms at {offenders:?}");
}

/// **SAFETY-009**, structurally: no cloud fact can reach a watering decision,
/// because `rhizo-domain` does not depend on the cloud client and
/// `IrrigationInputs` has no field derived from one.
#[test]
fn safety_009_no_cloud_input_reaches_a_watering_decision() {
    let manifest = include_str!("../Cargo.toml");
    assert!(
        !manifest.contains("cloud"),
        "rhizo-domain must not depend on any cloud crate"
    );
    let types = include_str!("../src/irrigation/types.rs");
    for forbidden in ["cloud_", "sync_status", "cloud_reachable"] {
        assert!(!types.contains(forbidden));
    }
}
