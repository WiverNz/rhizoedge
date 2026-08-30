//! The complete input and decision vocabulary of the watering decision
//! (M6-001, [PRD 060](../../../../docs/prd/060-irrigation-control-and-safety.md)).
//!
//! # Why this file is where SAFETY-009 is enforced structurally
//!
//! [`IrrigationInputs`] is *everything* a watering decision may consider. It has
//! no cloud-derived field, and it never will: adding one would be a visible edit
//! to a type the safety invariants name by hand, in a crate that does not depend
//! on `rhizo-cloud-client` and must not start. A cloud outage therefore cannot
//! change a watering answer, because there is nowhere for a cloud fact to enter.
//!
//! # Why `Option`, and why named tri-states
//!
//! Every absent-able input is an `Option` or an explicit tri-state, and there is
//! no `Default` on any of them. `unwrap_or_default()` on a safety input silently
//! converts "we do not know" into "it is fine", which is exactly the SAFETY-012
//! failure. [`LeakState`] is a three-variant enum rather than `Option<bool>` for
//! the same reason: `Option<bool>` invites `unwrap_or(false)`, while a named
//! `Unknown` variant forces the gate to classify it and fails to compile if a
//! future variant is not classified.
//!
//! Nothing here performs I/O, reads a clock, or mutates anything. These are pure
//! data; the rules live in [`super::gate`] and [`super::machine`].

use chrono::{DateTime, Duration, Utc};

use crate::plant::{ActuatorBinding, AutomationPolicy, MeasurementPolicy, SensorBinding};
use crate::profile::SoilSample;
use crate::recommend::Reason;
use crate::state::{IrrigationState, LockoutReason};

/// A pot-scale reading, dated by the **edge**'s receipt time.
///
/// The counterpart of [`SoilSample`] for the second delivery signal. Weight is
/// what makes no-delivery detection trustworthy near field capacity, where soil
/// moisture may not rise even though water arrived (M6-017).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WeightSample {
    /// Pot mass in grams, or `None` for a failed read.
    pub grams: Option<f64>,
    /// Edge-authoritative receipt time (SAFETY-005).
    pub received_at: DateTime<Utc>,
}

impl WeightSample {
    /// Physical plausibility only: finite and not negative.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.grams.is_some_and(|v| v.is_finite() && v >= 0.0)
    }

    /// Age against the **edge** clock; stale at the exact boundary.
    #[must_use]
    pub fn is_stale(&self, now: DateTime<Utc>, max_age: Duration) -> bool {
        now.signed_duration_since(self.received_at) >= max_age
    }
}

/// The leak signal as the edge currently understands it.
///
/// Three variants, never `Option<bool>`. `Unknown` is a first-class answer and
/// is **not** `Clear`: a plant whose tray sensor has never reported is a plant
/// whose floor nobody has looked at.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LeakState {
    /// The sensor reported no water present, recently enough to believe.
    Clear,
    /// Water is present.
    Detected,
    /// Absent, unreadable, or too old to act on.
    Unknown,
}

impl From<rhizo_mqtt_contract::safety::LeakState> for LeakState {
    /// The device's own tri-state maps one-for-one, so the two halves of
    /// SAFETY-003 cannot drift apart. Exhaustive on purpose.
    fn from(value: rhizo_mqtt_contract::safety::LeakState) -> Self {
        match value {
            rhizo_mqtt_contract::safety::LeakState::Clear => Self::Clear,
            rhizo_mqtt_contract::safety::LeakState::Detected => Self::Detected,
            rhizo_mqtt_contract::safety::LeakState::Unknown => Self::Unknown,
        }
    }
}

/// The reservoir as the edge currently understands it.
///
/// `Level` carries the age alongside the number so the gate can decide freshness
/// without a second lookup, and so a reading that is *low* is refused before a
/// reading that is merely *old* — a low tank is a measured fact and deserves the
/// more specific answer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TankState {
    /// A finite percentage, with its age against the edge clock.
    Level {
        /// Reservoir level, 0-100.
        percent: f64,
        /// Age at the moment of evaluation.
        age: Duration,
    },
    /// A reading exists but is not a usable number.
    Invalid,
    /// No reading at all. Absence of evidence is not a full tank.
    Unknown,
}

/// The observed condition of one `required`-role measurement.
///
/// Four named states rather than a `bool`: "the probe is unplugged" and "the
/// probe has not reported for an hour" need different operator responses and
/// map to different lockout reasons, and collapsing them would make the UI
/// unhelpful (M6-004).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RequiredInputState {
    /// Present, valid, and inside its freshness limit.
    Usable,
    /// No reading at all from the exact bound stream.
    Missing,
    /// A reading exists but is out of range, non-finite, or wrongly typed.
    Invalid,
    /// A valid reading, older than its limit against the **edge** clock.
    Stale,
}

/// One `required`-role binding and what the edge currently knows about it.
///
/// Leak and tank have their own dedicated gate inputs, because they are hard
/// vetoes with their own refusal reasons. Every *other* required kind arrives
/// here (SAFETY-017's edge counterpart), so a plant that requires pot weight
/// blocks when its scale is silent, and a plant that never bound one is
/// unaffected by the absence of a scale.
#[derive(Clone, Debug, PartialEq)]
pub struct RequiredInput {
    /// The bound kind.
    pub kind: rhizo_mqtt_contract::payload::MeasurementKind,
    /// What the edge knows about it.
    pub state: RequiredInputState,
}

/// Who asked, and what privilege that carries.
///
/// The asymmetry is deliberate and is F-060-05: `manual` skips **only** the
/// `SensorFault` and `StaleData` checks, because a human has looked at the plant
/// and taken responsibility for it. It skips nothing else — leak, tank, the
/// rolling cap, and the firmware hard limits all still apply.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EvaluationMode {
    /// The control loop's own evaluation.
    Automatic,
    /// `mode: "manual"` — an operator asking for `ml` having looked at the plant.
    ManualRequest {
        /// Requested volume.
        ml: f32,
    },
    /// `mode: "recommended"` — an operator accepting the engine's advice.
    ///
    /// Distinct from `manual` because the advice was computed from data that
    /// must still be fresh and valid; accepting a recommendation is not a claim
    /// to have inspected the plant (`http-api-boundaries.md` §2.6).
    RecommendedRequest {
        /// Requested volume.
        ml: f32,
    },
}

impl EvaluationMode {
    /// The requested volume, for the operator-initiated modes.
    #[must_use]
    pub const fn requested_ml(self) -> Option<f32> {
        match self {
            Self::Automatic => None,
            Self::ManualRequest { ml } | Self::RecommendedRequest { ml } => Some(ml),
        }
    }

    /// Whether this mode carries the F-060-05 privilege.
    ///
    /// Exhaustive with no catch-all: a mode added later cannot silently inherit
    /// the exemption.
    #[must_use]
    pub const fn skips_sensor_health(self) -> bool {
        match self {
            Self::ManualRequest { .. } => true,
            Self::Automatic | Self::RecommendedRequest { .. } => false,
        }
    }

    /// The stable ledger name written to `commands.mode` and
    /// `watering_events.mode`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Automatic => "automatic",
            Self::ManualRequest { .. } => "manual",
            Self::RecommendedRequest { .. } => "recommended",
        }
    }
}

/// Everything a watering decision may consider — and nothing else.
///
/// **No field here is derived from cloud state, and none may ever be**
/// (SAFETY-009). `rhizo-domain` does not depend on `rhizo-cloud-client`, so the
/// invariant is enforced by the dependency graph as well as by review.
#[derive(Clone, Copy, Debug)]
pub struct IrrigationInputs<'a> {
    /// The edge clock, supplied by the caller. This crate never reads one.
    pub now: DateTime<Utc>,
    /// The persisted irrigation state, loaded from SQLite this tick.
    pub state: &'a IrrigationState,
    /// Who asked.
    pub mode: EvaluationMode,
    /// The latest control-measurement reading. `None` means there is none.
    pub latest_soil: Option<&'a SoilSample>,
    /// The reading taken immediately before the current cycle's last dose.
    pub pre_dose_soil: Option<&'a SoilSample>,
    /// The latest pot-scale reading, where a scale exists.
    pub latest_weight: Option<&'a WeightSample>,
    /// The pot-scale reading taken before the current cycle's first dose.
    ///
    /// The companion of `pre_dose_soil`, and the baseline F-060-33 compares
    /// against: "no moisture **and** no weight response" needs two baselines,
    /// and PRD 060's illustrative struct supplies only one.
    pub pre_dose_weight: Option<&'a WeightSample>,
    /// The reservoir. `None` means the plant has no tank reading at all.
    pub tank: Option<TankState>,
    /// The leak signal. Not an `Option`: `Unknown` is the absent case.
    pub leak: LeakState,
    /// The plant's sensor bindings, with their roles.
    pub sensor_bindings: &'a [SensorBinding],
    /// The optional actuation route. `None` is a normal monitoring plant.
    pub actuator_binding: Option<&'a ActuatorBinding>,
    /// Per-measurement policies, as configured on this plant.
    pub measurement_policies: &'a [MeasurementPolicy],
    /// The automation configuration: doses, budgets, and durations.
    pub automation: &'a AutomationPolicy,
    /// The rolling 24-hour total, **derived from `watering_events` rows**.
    pub delivered_last_24h_ml: f32,
    /// Doses already delivered inside the current cycle.
    pub doses_this_cycle: u8,
    /// When the last cycle completed, for the cooldown.
    pub last_cycle_completed_at: Option<DateTime<Utc>>,
    /// The persisted absorption deadline, when one is in force.
    pub wait_until: Option<DateTime<Utc>>,
    /// The operator's connected-automation opt-in. Defaults to `false`.
    pub auto_watering_enabled: bool,
    /// Whether the actuator's device is reachable right now.
    pub device_online: bool,
    /// Continuous observed dryness, accumulated from samples rather than ticks.
    ///
    /// **Not in PRD 060's illustrative struct**, and required by its normative
    /// transition table: `Drying -> DryConfirmed` is defined as
    /// `dry >= dry_confirm_minutes`, and no other field carries that number.
    /// It is observed state rather than configuration, so it does not belong in
    /// [`AutomationPolicy`]; the alternative was a partial function, which
    /// F-060-11 forbids.
    pub dry_duration: Duration,
    /// Whether reconciliation of this plant's device is incomplete.
    ///
    /// A device that autonomously watered ninety seconds before reconnecting has
    /// that dose in its buffer and not yet in the budget, so acting on the
    /// budget would double-water it (SAFETY-016). Also absent from PRD 060's
    /// illustrative struct, which predates ADR-015.
    pub reconciling: bool,
    /// Every `required`-role measurement other than leak and tank.
    ///
    /// The edge half of SAFETY-017. Absent from PRD 060's illustrative struct,
    /// which predates ADR-016's binding roles; without it a plant that requires
    /// pot weight would water on a silent scale.
    pub required_inputs: &'a [RequiredInput],
    /// The lockout currently persisted against this plant, if any.
    ///
    /// Read so an **explicit-clear** lockout (F-060-41) survives the condition
    /// that raised it: a leak that dried out does not silently re-enable
    /// watering, because the burst joint has not necessarily been fixed. An
    /// auto-clearing lockout (F-060-40) is ignored here and re-derived from
    /// current inputs, which is exactly what "clears when the condition
    /// resolves" means. Absent from PRD 060's illustrative struct; without it
    /// no lockout could outlive one tick.
    pub active_lockout: Option<LockoutReason>,
    /// Until when an auto-clearing lockout is nevertheless **held**.
    ///
    /// F-060-51's only mechanism: a forward clock step of more than ten minutes
    /// locks every plant `Uncertain` for one cooldown, and `Uncertain` would
    /// otherwise clear on the very next tick because nothing about the inputs is
    /// wrong. Also absent from PRD 060's illustrative struct, which states the
    /// requirement without naming a field for it.
    pub lockout_held_until: Option<DateTime<Utc>>,
}

/// What the machine decided.
///
/// Total: `evaluate` answers one of these for every (state, input) pair,
/// including inputs that are absent — which resolve to [`Self::Lock`] through
/// the gate (F-060-11).
#[derive(Clone, Debug, PartialEq)]
pub enum IrrigationDecision {
    /// Nothing to do.
    Idle,
    /// The plant needs water and automation is off, so a human is told.
    Recommend {
        /// The profile dose. Never a computed volume.
        ml: f32,
        /// Every conjunct that mattered.
        reasons: Vec<Reason>,
    },
    /// Issue a dose of `ml`.
    IssueDose {
        /// The volume to command.
        ml: f32,
        /// Every conjunct that mattered.
        reasons: Vec<Reason>,
    },
    /// Wait until `until` before deciding again.
    Wait {
        /// The persisted deadline.
        until: DateTime<Utc>,
    },
    /// Lock the plant out.
    Lock {
        /// Why.
        reason: LockoutReason,
    },
    /// The cycle finished and the plant recovered.
    CycleComplete,
}

impl IrrigationDecision {
    /// Whether this decision moves water.
    ///
    /// Exhaustive with no catch-all arm: a decision variant added later cannot
    /// default to "harmless".
    #[must_use]
    pub const fn actuates(&self) -> bool {
        match self {
            Self::IssueDose { .. } => true,
            Self::Idle
            | Self::Recommend { .. }
            | Self::Wait { .. }
            | Self::Lock { .. }
            | Self::CycleComplete => false,
        }
    }

    /// A stable name for logs, metrics, and persisted transitions.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Recommend { .. } => "recommend",
            Self::IssueDose { .. } => "issue_dose",
            Self::Wait { .. } => "wait",
            Self::Lock { .. } => "lock",
            Self::CycleComplete => "cycle_complete",
        }
    }
}

/// The wire name of an irrigation state, as `irrigation_state.state` stores it.
#[must_use]
pub const fn state_name(state: IrrigationState) -> &'static str {
    match state {
        IrrigationState::Normal => "normal",
        IrrigationState::Drying => "drying",
        IrrigationState::DryConfirmed => "dry_confirmed",
        IrrigationState::DoseIssued => "dose_issued",
        IrrigationState::WaitForAbsorption => "wait_for_absorption",
        IrrigationState::Recheck => "recheck",
        IrrigationState::Locked => "locked",
    }
}

/// Decodes a stored irrigation state.
///
/// An unrecognised name reads as [`IrrigationState::Locked`], never as
/// `Normal`: a state the edge cannot interpret is not a state it may water from
/// (SAFETY-012).
#[must_use]
pub fn state_from_str(name: &str) -> IrrigationState {
    match name {
        "normal" => IrrigationState::Normal,
        "drying" => IrrigationState::Drying,
        "dry_confirmed" => IrrigationState::DryConfirmed,
        "dose_issued" => IrrigationState::DoseIssued,
        "wait_for_absorption" => IrrigationState::WaitForAbsorption,
        "recheck" => IrrigationState::Recheck,
        _ => IrrigationState::Locked,
    }
}

/// Whether a lockout clears on its own once the condition resolves (F-060-40),
/// or needs an operator to say so (F-060-41).
///
/// Exhaustive, with `Unknown` classified explicitly as needing a human: a
/// lockout the edge cannot name is not one it may lift by itself.
#[must_use]
pub const fn is_auto_clearable(reason: LockoutReason) -> bool {
    match reason {
        LockoutReason::StaleData
        | LockoutReason::SensorFault
        | LockoutReason::TankLow
        | LockoutReason::DailyLimit
        | LockoutReason::ClockUnsynced
        | LockoutReason::NoActuator
        | LockoutReason::Uncertain => true,
        LockoutReason::Leak
        | LockoutReason::PumpFault
        | LockoutReason::NoDeliveryDetected
        | LockoutReason::MaxDosesReached
        | LockoutReason::Unknown => false,
    }
}

#[cfg(test)]
#[allow(
    clippy::module_inception,
    reason = "the module name is the verification filter the issue quotes literally"
)]
mod types {
    use super::*;
    use chrono::TimeZone;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, 30, 12, 0, 0)
            .single()
            .expect("a valid fixed instant")
    }

    /// SAFETY-012: `Unknown` is a distinct answer, not a spelling of `Clear`.
    #[test]
    fn leak_unknown_is_not_leak_clear() {
        assert_ne!(LeakState::Unknown, LeakState::Clear);
        assert_ne!(LeakState::Unknown, LeakState::Detected);
        assert_eq!(
            LeakState::from(rhizo_mqtt_contract::safety::LeakState::Unknown),
            LeakState::Unknown
        );
        assert_eq!(
            LeakState::from(rhizo_mqtt_contract::safety::LeakState::Clear),
            LeakState::Clear
        );
        assert_eq!(
            LeakState::from(rhizo_mqtt_contract::safety::LeakState::Detected),
            LeakState::Detected
        );
    }

    /// The tank is a tri-state too: `Unknown` and `Invalid` are both distinct
    /// from any measured level, including zero.
    #[test]
    fn tank_unknown_is_not_a_measured_level() {
        assert_ne!(
            TankState::Unknown,
            TankState::Level {
                percent: 0.0,
                age: Duration::zero()
            }
        );
        assert_ne!(TankState::Invalid, TankState::Unknown);
    }

    /// F-060-05's privilege belongs to `manual` alone.
    #[test]
    fn only_manual_carries_the_sensor_health_privilege() {
        assert!(EvaluationMode::ManualRequest { ml: 30.0 }.skips_sensor_health());
        assert!(!EvaluationMode::Automatic.skips_sensor_health());
        assert!(!EvaluationMode::RecommendedRequest { ml: 30.0 }.skips_sensor_health());
        assert_eq!(EvaluationMode::Automatic.requested_ml(), None);
        assert_eq!(
            EvaluationMode::ManualRequest { ml: 30.0 }.requested_ml(),
            Some(30.0)
        );
        assert_eq!(EvaluationMode::Automatic.as_str(), "automatic");
        assert_eq!(
            EvaluationMode::RecommendedRequest { ml: 1.0 }.as_str(),
            "recommended"
        );
    }

    /// Every absent-able input can actually be absent, and the struct builds
    /// with nothing known at all.
    #[test]
    fn inputs_construct_with_every_absent_able_field_absent() {
        let state = IrrigationState::Normal;
        let automation = AutomationPolicy::default();
        let inputs = IrrigationInputs {
            now: now(),
            state: &state,
            mode: EvaluationMode::Automatic,
            latest_soil: None,
            pre_dose_soil: None,
            latest_weight: None,
            pre_dose_weight: None,
            tank: None,
            leak: LeakState::Unknown,
            sensor_bindings: &[],
            actuator_binding: None,
            measurement_policies: &[],
            automation: &automation,
            delivered_last_24h_ml: 0.0,
            doses_this_cycle: 0,
            last_cycle_completed_at: None,
            wait_until: None,
            auto_watering_enabled: false,
            device_online: false,
            dry_duration: Duration::zero(),
            reconciling: false,
            required_inputs: &[],
            active_lockout: None,
            lockout_held_until: None,
        };
        assert!(inputs.latest_soil.is_none());
        assert!(inputs.tank.is_none());
        assert!(inputs.actuator_binding.is_none());
        assert_eq!(inputs.leak, LeakState::Unknown);
    }

    /// Only `IssueDose` moves water.
    #[test]
    fn one_decision_actuates_and_the_rest_do_not() {
        assert!(
            IrrigationDecision::IssueDose {
                ml: 40.0,
                reasons: Vec::new()
            }
            .actuates()
        );
        for decision in [
            IrrigationDecision::Idle,
            IrrigationDecision::Recommend {
                ml: 40.0,
                reasons: Vec::new(),
            },
            IrrigationDecision::Wait { until: now() },
            IrrigationDecision::Lock {
                reason: LockoutReason::Leak,
            },
            IrrigationDecision::CycleComplete,
        ] {
            assert!(!decision.actuates(), "{decision:?}");
        }
    }

    /// An unreadable persisted state is `Locked`, never `Normal`.
    #[test]
    fn an_unknown_stored_state_reads_as_locked() {
        for state in [
            IrrigationState::Normal,
            IrrigationState::Drying,
            IrrigationState::DryConfirmed,
            IrrigationState::DoseIssued,
            IrrigationState::WaitForAbsorption,
            IrrigationState::Recheck,
            IrrigationState::Locked,
        ] {
            assert_eq!(state_from_str(state_name(state)), state);
        }
        assert_eq!(state_from_str("something_new"), IrrigationState::Locked);
        assert_eq!(state_from_str(""), IrrigationState::Locked);
    }

    /// F-060-40 and F-060-41, and the rule for a lockout nobody recognises.
    #[test]
    fn clearability_matches_the_documented_split() {
        for reason in [
            LockoutReason::StaleData,
            LockoutReason::SensorFault,
            LockoutReason::TankLow,
            LockoutReason::DailyLimit,
            LockoutReason::Uncertain,
        ] {
            assert!(is_auto_clearable(reason), "{reason:?}");
        }
        for reason in [
            LockoutReason::Leak,
            LockoutReason::PumpFault,
            LockoutReason::NoDeliveryDetected,
            LockoutReason::MaxDosesReached,
            LockoutReason::Unknown,
        ] {
            assert!(!is_auto_clearable(reason), "{reason:?}");
        }
    }

    /// A weight reading is dated by the edge and stale at the boundary.
    #[test]
    fn weight_validity_and_staleness() {
        let sample = WeightSample {
            grams: Some(1_800.0),
            received_at: now(),
        };
        assert!(sample.is_valid());
        assert!(!sample.is_stale(now(), Duration::minutes(15)));
        assert!(sample.is_stale(now() + Duration::minutes(15), Duration::minutes(15)));
        for bad in [f64::NAN, f64::INFINITY, -1.0] {
            assert!(
                !WeightSample {
                    grams: Some(bad),
                    received_at: now()
                }
                .is_valid(),
                "{bad}"
            );
        }
        assert!(
            !WeightSample {
                grams: None,
                received_at: now()
            }
            .is_valid()
        );
    }
}
