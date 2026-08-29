//! The rule-based recommendation engine (PRD 050 §Rule, F-050-20…F-050-25).
//!
//! ```text
//! recommend water WHEN
//!       latest sample valid AND fresh
//!   AND moisture < profile.target_min
//!   AND dry_duration >= profile.dry_confirm_minutes
//!   AND time_since_last_watering >= profile.cooldown_hours
//!   AND safety gate passes
//! ```
//!
//! **Every failing conjunct contributes a reason**, so a `no_water` answer is
//! exactly as explainable as a `water` one. That symmetry is the point: an
//! operator will not enable automation they do not understand, and "no" is the
//! answer they will see most.
//!
//! Reasons are typed enum values rather than prose strings. The boilerplate buys
//! assertable tests and a renderable UI, and prose is produced in exactly one
//! place — the API layer.
//!
//! # `confidence` is advisory and decides nothing
//!
//! It is reported for operator intuition and is **not** an input to any safety
//! decision. It conflates sparsity, missing sensors, and an absent trend into
//! one scalar, which PRD 050 §Open questions 1 records as a deliberate
//! simplification — contained precisely because nothing gates on it. A future
//! contributor tempted to gate a dose on it should read this paragraph first;
//! [`tests::confidence_is_reported_and_never_decides`] fails if they do.
use chrono::Duration;

use crate::state::LockoutReason;
use crate::trend::{MIN_SAMPLES, TrendVwcPerHour};

/// What the engine advises.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    /// Every conjunct passed.
    Water,
    /// At least one conjunct failed, and nothing is locked out.
    NoWater,
    /// A lockout applies. `blocked_by` names it.
    Blocked,
}

impl Decision {
    /// The stable wire name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Water => "water",
            Self::NoWater => "no_water",
            Self::Blocked => "blocked",
        }
    }
}

/// One structured reason. Data is carried so the UI can render a sentence and a
/// test can assert on the numbers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Reason {
    /// Moisture is below the plant's target minimum.
    MoistureBelowTarget {
        /// Latest reading.
        vwc: f64,
        /// Configured minimum.
        target_min: f64,
    },
    /// Moisture is at or above the target, so the plant does not need water.
    MoistureAtOrAboveTarget {
        /// Latest reading.
        vwc: f64,
        /// Configured minimum.
        target_min: f64,
    },
    /// The dryness debounce has elapsed.
    DryFor {
        /// Observed continuous dryness.
        minutes: i64,
        /// Configured debounce.
        required: i64,
    },
    /// The dryness debounce has not elapsed yet.
    NotDryLongEnough {
        /// Observed continuous dryness.
        minutes: i64,
        /// Configured debounce.
        required: i64,
    },
    /// The cooldown since the last watering has elapsed.
    LastWatering {
        /// Hours since the last watering of any mode.
        hours_ago: f64,
    },
    /// The plant was watered recently.
    CooldownActive {
        /// Hours since the last watering of any mode.
        hours_ago: f64,
        /// Configured cooldown.
        required_hours: f64,
    },
    /// No watering has ever been recorded, so no cooldown applies.
    NeverWatered,
    /// There is no usable moisture reading at all.
    SampleMissing,
    /// The latest reading failed validation.
    SampleInvalid,
    /// The latest reading is older than the control-freshness threshold.
    SampleStale {
        /// Age of the latest reading.
        age_seconds: i64,
        /// `max_sample_age` for this plant's cadence.
        max_age_seconds: i64,
    },
    /// A bound sensor is unhealthy.
    SensorUnhealthy,
    /// The plant has no actuator, so watering is not a thing it can do.
    NoActuator,
    /// A lockout is in force.
    LockedOut {
        /// The lockout.
        reason: LockoutReason,
    },
    /// No trend could be fitted. Advisory: it gates nothing.
    TrendUnavailable,
    /// The fitted trend, for display.
    Trend {
        /// %VWC per hour.
        vwc_per_hour: f64,
    },
}

impl Reason {
    /// The stable machine-readable code, as `http-api-boundaries.md` §2.5
    /// renders it.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::MoistureBelowTarget { .. } => "moisture_below_target",
            Self::MoistureAtOrAboveTarget { .. } => "moisture_at_or_above_target",
            Self::DryFor { .. } => "dry_for",
            Self::NotDryLongEnough { .. } => "not_dry_long_enough",
            Self::LastWatering { .. } => "last_watering",
            Self::CooldownActive { .. } => "cooldown_active",
            Self::NeverWatered => "never_watered",
            Self::SampleMissing => "sample_missing",
            Self::SampleInvalid => "sample_invalid",
            Self::SampleStale { .. } => "sample_stale",
            Self::SensorUnhealthy => "sensor_unhealthy",
            Self::NoActuator => "no_actuator",
            Self::LockedOut { .. } => "locked_out",
            Self::TrendUnavailable => "trend_unavailable",
            Self::Trend { .. } => "trend",
        }
    }
}

/// Everything the rule consumes. Every absent-able input is an `Option`, so
/// "missing" cannot be spelled as a default (SAFETY-012).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RecommendationInputs {
    /// Latest **validated** moisture reading. `None` means missing or invalid.
    pub moisture_vwc: Option<f64>,
    /// Whether a reading exists at all but failed validation.
    pub latest_sample_invalid: bool,
    /// Age of the latest reading against the **edge** clock. `None` means there
    /// has never been one.
    pub sample_age: Option<Duration>,
    /// SAFETY-005's control-freshness threshold for this plant's cadence.
    pub max_sample_age: Duration,
    /// The plant's target minimum.
    pub target_min: f64,
    /// Observed continuous dryness.
    pub dry_duration: Duration,
    /// The configured debounce.
    pub dry_confirm: Duration,
    /// Time since the last watering of **any** mode, including `detected`
    /// (F-050-13). `None` means the plant has never been watered.
    pub time_since_last_watering: Option<Duration>,
    /// The configured cooldown.
    pub cooldown: Duration,
    /// The dose the profile prescribes. Never a computed volume (F-050-23).
    pub dose_ml: f32,
    /// Whether the plant has an `ActuatorBinding` at all (SAFETY-018).
    pub has_actuator: bool,
    /// Whether every `required`-role sensor is healthy.
    pub required_sensors_healthy: bool,
    /// A lockout the safety gate would apply. M6 fills this; in M5 it carries
    /// only the conditions the plant model itself can see.
    pub lockout: Option<LockoutReason>,
    /// The fitted moisture trend, if one could be fitted.
    pub trend: Option<TrendVwcPerHour>,
    /// How many valid samples were available inside the trend window.
    pub samples_in_window: usize,
    /// Whether a pot scale contributes to this plant.
    pub has_weight_sensor: bool,
}

/// The engine's answer.
#[derive(Clone, Debug, PartialEq)]
pub struct Recommendation {
    /// Water, no water, or blocked.
    pub decision: Decision,
    /// The profile dose, present only for [`Decision::Water`].
    pub recommended_ml: Option<f32>,
    /// Advisory only. Decides nothing.
    pub confidence: f32,
    /// Every conjunct that mattered, passing or failing.
    pub reasons: Vec<Reason>,
    /// The lockout, when the decision is [`Decision::Blocked`].
    pub blocked_by: Option<LockoutReason>,
}

impl Recommendation {
    /// Whether two answers are the same *decision*, which is what M5-012 keys
    /// change-only persistence on.
    #[must_use]
    pub fn same_answer_as(&self, other: &Self) -> bool {
        self.decision == other.decision
            && self.blocked_by == other.blocked_by
            && self
                .reasons
                .iter()
                .map(Reason::code)
                .eq(other.reasons.iter().map(Reason::code))
    }
}

/// Evaluates the rule.
///
/// Pure: no I/O, and no clock — every interval arrives as a parameter, which is
/// what makes the whole rule reproducible in a test and is why `Utc::now` is
/// banned in this crate (ADR-013).
#[must_use]
pub fn recommend(inputs: &RecommendationInputs) -> Recommendation {
    let mut reasons = Vec::new();
    let mut passes = true;

    // Conjunct 0 (SAFETY-018): a plant with no actuator has no actuation path,
    // so the answer is not "no water yet", it is "there is nothing to water
    // with". It is a blocking condition rather than a failed conjunct.
    let mut lockout = inputs.lockout;
    if !inputs.has_actuator {
        reasons.push(Reason::NoActuator);
        lockout = lockout.or(Some(LockoutReason::NoActuator));
        passes = false;
    }

    // Conjunct 1: the latest sample is present, valid, and fresh.
    match (inputs.moisture_vwc, inputs.sample_age) {
        (None, None) => {
            reasons.push(Reason::SampleMissing);
            lockout = lockout.or(Some(LockoutReason::SensorFault));
            passes = false;
        }
        (None, Some(_)) => {
            reasons.push(if inputs.latest_sample_invalid {
                Reason::SampleInvalid
            } else {
                Reason::SampleMissing
            });
            lockout = lockout.or(Some(LockoutReason::SensorFault));
            passes = false;
        }
        (Some(_), None) => {
            // A reading with no age is a reading the edge cannot date. Unknown
            // freshness is not freshness (SAFETY-005, SAFETY-012).
            reasons.push(Reason::SampleMissing);
            lockout = lockout.or(Some(LockoutReason::StaleData));
            passes = false;
        }
        (Some(_), Some(age)) if age >= inputs.max_sample_age => {
            reasons.push(Reason::SampleStale {
                age_seconds: age.num_seconds(),
                max_age_seconds: inputs.max_sample_age.num_seconds(),
            });
            lockout = lockout.or(Some(LockoutReason::StaleData));
            passes = false;
        }
        (Some(_), Some(_)) => {}
    }

    if !inputs.required_sensors_healthy {
        reasons.push(Reason::SensorUnhealthy);
        lockout = lockout.or(Some(LockoutReason::SensorFault));
        passes = false;
    }

    // Conjunct 2: moisture below the target minimum.
    if let Some(vwc) = inputs.moisture_vwc {
        if vwc < inputs.target_min {
            reasons.push(Reason::MoistureBelowTarget {
                vwc,
                target_min: inputs.target_min,
            });
        } else {
            reasons.push(Reason::MoistureAtOrAboveTarget {
                vwc,
                target_min: inputs.target_min,
            });
            passes = false;
        }
    }

    // Conjunct 3: the dryness debounce has elapsed.
    let dry_minutes = inputs.dry_duration.num_minutes();
    let required_minutes = inputs.dry_confirm.num_minutes();
    if inputs.dry_duration >= inputs.dry_confirm {
        reasons.push(Reason::DryFor {
            minutes: dry_minutes,
            required: required_minutes,
        });
    } else {
        reasons.push(Reason::NotDryLongEnough {
            minutes: dry_minutes,
            required: required_minutes,
        });
        passes = false;
    }

    // Conjunct 4: the cooldown has elapsed. A plant that has never been watered
    // has no cooldown to wait out.
    match inputs.time_since_last_watering {
        None => reasons.push(Reason::NeverWatered),
        Some(since) if since >= inputs.cooldown => reasons.push(Reason::LastWatering {
            hours_ago: hours(since),
        }),
        Some(since) => {
            reasons.push(Reason::CooldownActive {
                hours_ago: hours(since),
                required_hours: hours(inputs.cooldown),
            });
            passes = false;
        }
    }

    // Conjunct 5: the safety gate. In M5 nothing here can issue a command, so
    // this is the gate's *view*, not its execution.
    if let Some(reason) = lockout {
        reasons.push(Reason::LockedOut { reason });
        passes = false;
    }

    match inputs.trend {
        Some(TrendVwcPerHour(slope)) => reasons.push(Reason::Trend {
            vwc_per_hour: slope,
        }),
        None => reasons.push(Reason::TrendUnavailable),
    }

    let decision = if lockout.is_some() {
        Decision::Blocked
    } else if passes {
        Decision::Water
    } else {
        Decision::NoWater
    };
    Recommendation {
        decision,
        // F-050-23: the profile dose, never an unbounded computation.
        recommended_ml: (decision == Decision::Water).then_some(inputs.dose_ml),
        confidence: confidence(inputs),
        reasons,
        blocked_by: lockout,
    }
}

fn hours(d: Duration) -> f64 {
    d.num_milliseconds() as f64 / 3_600_000.0
}

/// Reported for operator intuition. **Not an input to any decision.**
fn confidence(inputs: &RecommendationInputs) -> f32 {
    let mut c: f32 = 1.0;
    if inputs.trend.is_none() {
        c *= 0.80;
    }
    if inputs.samples_in_window < MIN_SAMPLES {
        c *= 0.70;
    }
    if !inputs.has_weight_sensor {
        // Moisture alone gives a poorer estimate of what actually happened to
        // the pot (PRD 050 §Failure modes).
        c *= 0.90;
    }
    if let (Some(age), max) = (inputs.sample_age, inputs.max_sample_age)
        && max > Duration::zero()
        && age.num_milliseconds() * 2 > max.num_milliseconds()
    {
        c *= 0.90;
    }
    if inputs.moisture_vwc.is_none() {
        c *= 0.50;
    }
    c.clamp(0.05, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn codes(r: &Recommendation) -> Vec<&'static str> {
        r.reasons.iter().map(Reason::code).collect()
    }

    /// A dry, fresh, past-cooldown plant with a healthy actuator.
    fn passing() -> RecommendationInputs {
        RecommendationInputs {
            moisture_vwc: Some(24.1),
            latest_sample_invalid: false,
            sample_age: Some(Duration::seconds(42)),
            max_sample_age: Duration::minutes(15),
            target_min: 28.0,
            dry_duration: Duration::minutes(42),
            dry_confirm: Duration::minutes(30),
            time_since_last_watering: Some(Duration::hours(148)),
            cooldown: Duration::hours(6),
            dose_ml: 40.0,
            has_actuator: true,
            required_sensors_healthy: true,
            lockout: None,
            trend: Some(TrendVwcPerHour(-0.8)),
            samples_in_window: 12,
            has_weight_sensor: true,
        }
    }

    #[test]
    fn a_dry_fresh_past_cooldown_plant_recommends_water_with_reasons() {
        let r = recommend(&passing());
        assert_eq!(r.decision, Decision::Water);
        assert_eq!(r.recommended_ml, Some(40.0));
        assert_eq!(r.blocked_by, None);
        assert_eq!(
            codes(&r),
            vec!["moisture_below_target", "dry_for", "last_watering", "trend"]
        );
    }

    /// Each conjunct failing in isolation, asserting the exact reason set.
    #[test]
    fn each_failing_conjunct_produces_its_own_reason() {
        let mut i = passing();
        i.moisture_vwc = Some(33.0);
        let r = recommend(&i);
        assert_eq!(r.decision, Decision::NoWater);
        assert_eq!(r.recommended_ml, None);
        assert!(codes(&r).contains(&"moisture_at_or_above_target"));

        let mut i = passing();
        i.dry_duration = Duration::minutes(5);
        let r = recommend(&i);
        assert_eq!(r.decision, Decision::NoWater);
        assert!(codes(&r).contains(&"not_dry_long_enough"));
        assert!(r.reasons.contains(&Reason::NotDryLongEnough {
            minutes: 5,
            required: 30
        }));

        let mut i = passing();
        i.time_since_last_watering = Some(Duration::hours(1));
        let r = recommend(&i);
        assert_eq!(r.decision, Decision::NoWater);
        assert!(codes(&r).contains(&"cooldown_active"));

        let mut i = passing();
        i.sample_age = Some(Duration::minutes(15));
        let r = recommend(&i);
        assert_eq!(r.decision, Decision::Blocked);
        assert_eq!(r.blocked_by, Some(LockoutReason::StaleData));
        assert!(codes(&r).contains(&"sample_stale"));

        let mut i = passing();
        i.moisture_vwc = None;
        i.latest_sample_invalid = true;
        let r = recommend(&i);
        assert_eq!(r.decision, Decision::Blocked);
        assert_eq!(r.blocked_by, Some(LockoutReason::SensorFault));
        assert!(codes(&r).contains(&"sample_invalid"));

        let mut i = passing();
        i.required_sensors_healthy = false;
        let r = recommend(&i);
        assert_eq!(r.blocked_by, Some(LockoutReason::SensorFault));

        let mut i = passing();
        i.lockout = Some(LockoutReason::Leak);
        let r = recommend(&i);
        assert_eq!(r.decision, Decision::Blocked);
        assert_eq!(r.blocked_by, Some(LockoutReason::Leak));
        assert!(r.reasons.contains(&Reason::LockedOut {
            reason: LockoutReason::Leak
        }));
    }

    /// SAFETY-018 in the engine: a monitoring-only plant is never advised to
    /// water, and the answer names the absent actuator rather than a safety
    /// refusal.
    #[test]
    fn safety_018_a_plant_with_no_actuator_is_never_advised_to_water() {
        let mut i = passing();
        i.has_actuator = false;
        let r = recommend(&i);
        assert_eq!(r.decision, Decision::Blocked);
        assert_eq!(r.blocked_by, Some(LockoutReason::NoActuator));
        assert_eq!(r.recommended_ml, None);
        assert!(codes(&r).contains(&"no_actuator"));
    }

    /// `no_water` must be as explainable as `water`: the passing conjuncts are
    /// reported too, so the operator can see what was already true.
    #[test]
    fn a_no_water_answer_carries_reasons_for_every_conjunct() {
        let mut i = passing();
        i.dry_duration = Duration::minutes(1);
        let r = recommend(&i);
        assert_eq!(r.decision, Decision::NoWater);
        assert_eq!(
            codes(&r),
            vec![
                "moisture_below_target",
                "not_dry_long_enough",
                "last_watering",
                "trend"
            ]
        );
    }

    #[test]
    fn a_plant_that_has_never_been_watered_waits_for_no_cooldown() {
        let mut i = passing();
        i.time_since_last_watering = None;
        let r = recommend(&i);
        assert_eq!(r.decision, Decision::Water);
        assert!(codes(&r).contains(&"never_watered"));
    }

    /// Confidence drops with sparse, noisy, or partially missing inputs — and
    /// changes no decision, which is the property that keeps the simplification
    /// contained (PRD 050 §Open questions 1).
    #[test]
    fn confidence_is_reported_and_never_decides() {
        let full = recommend(&passing());
        assert!((full.confidence - 1.0).abs() < 1e-6);

        let mut sparse = passing();
        sparse.samples_in_window = 3;
        sparse.trend = None;
        sparse.has_weight_sensor = false;
        let degraded = recommend(&sparse);
        assert!(
            degraded.confidence < full.confidence,
            "{} vs {}",
            degraded.confidence,
            full.confidence
        );
        assert_eq!(
            degraded.decision, full.decision,
            "confidence must not change the answer"
        );
        assert_eq!(degraded.recommended_ml, full.recommended_ml);
        assert!(degraded.confidence >= 0.05);
    }

    /// `recommended_ml` is the profile dose and nothing else (F-050-23).
    #[test]
    fn the_recommended_volume_is_always_the_profile_dose() {
        for dose in [1.0f32, 40.0, 80.0] {
            let mut i = passing();
            i.dose_ml = dose;
            assert_eq!(recommend(&i).recommended_ml, Some(dose));
        }
    }

    #[test]
    fn the_same_answer_comparison_ignores_values_but_not_codes() {
        let a = recommend(&passing());
        let mut moved = passing();
        moved.moisture_vwc = Some(23.0);
        assert!(a.same_answer_as(&recommend(&moved)));
        let mut different = passing();
        different.dry_duration = Duration::minutes(1);
        assert!(!a.same_answer_as(&recommend(&different)));
    }
}
