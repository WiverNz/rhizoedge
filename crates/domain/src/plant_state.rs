//! Deriving the operator-facing plant state (PRD 050 §State model, F-050-26).
//!
//! This state is **descriptive**. The irrigation state machine that *acts* is a
//! separate type and arrives in M6
//! ([ADR-006](../../../docs/adr/006-irrigation-state-machine-ownership.md)).
//! Keeping them distinct is what lets the UI show "needs water" without
//! implying "is about to water": `WaterRecommended` with automation disabled is
//! a normal, stable, indefinite condition, and the irrigation machine would have
//! nothing useful to say about it.
//!
//! Do not merge the two, however tempting. [`crate::state::IrrigationState`] is
//! not re-exported from here, and nothing in this module constructs one.
use crate::recommend::Recommendation;
use crate::state::{LockoutReason, PlantState};

/// Derives the operator-facing state from an evaluated recommendation.
///
/// Pure, total, and driven entirely by the recommendation's own typed reasons —
/// so the state and the explanation an operator reads can never disagree.
#[must_use]
pub fn derive(recommendation: &Recommendation) -> PlantState {
    let has = |code: &str| recommendation.reasons.iter().any(|r| r.code() == code);

    match recommendation.blocked_by {
        // A plant whose data stopped or went bad is, to an operator, a sensor
        // problem — not a safety lockout they can clear.
        Some(LockoutReason::SensorFault | LockoutReason::StaleData) => {
            return PlantState::SensorFault;
        }
        // SAFETY-018: a monitoring-only plant is a normal plant, not a locked
        // one. Presenting it as `WateringLocked` would be the "has a pump but
        // it is disabled" model ADR-016 rejected.
        Some(LockoutReason::NoActuator) | None => {}
        Some(_) => return PlantState::WateringLocked,
    }

    // Keyed on the reason *codes*, which are the same stable strings the API
    // renders, so the state and the explanation can never disagree.
    let dry = has("moisture_below_target");
    let confirmed = has("dry_for");
    let cooling_down = has("cooldown_active");

    if dry && confirmed && !cooling_down {
        PlantState::WaterRecommended
    } else if dry {
        PlantState::Drying
    } else {
        PlantState::Healthy
    }
}

/// A state change worth persisting.
///
/// Only transitions are recorded. A 30-second tick that reaches the same
/// conclusion is not news, and persisting it would write thousands of rows a day
/// saying nothing happened (ADR-010).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Transition {
    /// The state left behind, or `None` on the first evaluation of a plant.
    pub from: Option<PlantState>,
    /// The state entered.
    pub to: PlantState,
}

/// Returns the transition to persist, or `None` in steady state.
#[must_use]
pub fn transition(previous: Option<PlantState>, current: PlantState) -> Option<Transition> {
    match previous {
        Some(p) if p == current => None,
        from => Some(Transition { from, to: current }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recommend::{Decision, Reason, Recommendation};

    fn answer(
        decision: Decision,
        blocked_by: Option<LockoutReason>,
        reasons: Vec<Reason>,
    ) -> Recommendation {
        Recommendation {
            decision,
            recommended_ml: None,
            confidence: 1.0,
            reasons,
            blocked_by,
        }
    }
    fn below() -> Reason {
        Reason::MoistureBelowTarget {
            vwc: 24.0,
            target_min: 28.0,
        }
    }
    fn above() -> Reason {
        Reason::MoistureAtOrAboveTarget {
            vwc: 34.0,
            target_min: 28.0,
        }
    }
    fn dry_for() -> Reason {
        Reason::DryFor {
            minutes: 42,
            required: 30,
        }
    }

    #[test]
    fn each_state_is_derived_under_its_documented_conditions() {
        assert_eq!(
            derive(&answer(Decision::NoWater, None, vec![above()])),
            PlantState::Healthy
        );
        assert_eq!(
            derive(&answer(
                Decision::NoWater,
                None,
                vec![
                    below(),
                    Reason::NotDryLongEnough {
                        minutes: 2,
                        required: 30
                    }
                ]
            )),
            PlantState::Drying
        );
        assert_eq!(
            derive(&answer(
                Decision::Water,
                None,
                vec![below(), dry_for(), Reason::NeverWatered]
            )),
            PlantState::WaterRecommended
        );
        assert_eq!(
            derive(&answer(
                Decision::Blocked,
                Some(LockoutReason::SensorFault),
                vec![Reason::SampleInvalid]
            )),
            PlantState::SensorFault
        );
        assert_eq!(
            derive(&answer(
                Decision::Blocked,
                Some(LockoutReason::StaleData),
                vec![Reason::SampleStale {
                    age_seconds: 9_000,
                    max_age_seconds: 900
                }]
            )),
            PlantState::SensorFault
        );
        assert_eq!(
            derive(&answer(
                Decision::Blocked,
                Some(LockoutReason::Leak),
                vec![below(), dry_for()]
            )),
            PlantState::WateringLocked
        );
    }

    /// A confirmed-dry plant inside its cooldown is drying, not recommended:
    /// the machine already knows it should wait.
    #[test]
    fn a_cooling_down_plant_is_drying_rather_than_recommended() {
        assert_eq!(
            derive(&answer(
                Decision::NoWater,
                None,
                vec![
                    below(),
                    dry_for(),
                    Reason::CooldownActive {
                        hours_ago: 1.0,
                        required_hours: 6.0
                    }
                ]
            )),
            PlantState::Drying
        );
    }

    /// SAFETY-018: monitoring-only is a normal plant. It reaches
    /// `WaterRecommended` and stays there indefinitely, which is exactly what an
    /// operator with a watering can needs to see.
    #[test]
    fn a_monitoring_only_plant_is_recommended_not_locked() {
        let r = answer(
            Decision::Blocked,
            Some(LockoutReason::NoActuator),
            vec![Reason::NoActuator, below(), dry_for(), Reason::NeverWatered],
        );
        assert_eq!(derive(&r), PlantState::WaterRecommended);
    }

    #[test]
    fn only_transitions_are_persisted() {
        assert_eq!(
            transition(None, PlantState::Healthy),
            Some(Transition {
                from: None,
                to: PlantState::Healthy
            })
        );
        assert_eq!(
            transition(Some(PlantState::Healthy), PlantState::Healthy),
            None
        );
        assert_eq!(
            transition(Some(PlantState::Healthy), PlantState::Drying),
            Some(Transition {
                from: Some(PlantState::Healthy),
                to: PlantState::Drying
            })
        );
    }

    /// The two state concepts stay separate types. This does not compile if
    /// somebody merges them, which is the point.
    #[test]
    fn plant_state_and_irrigation_state_are_separate_types() {
        let plant: PlantState = PlantState::Drying;
        let irrigation: crate::state::IrrigationState = crate::state::IrrigationState::Drying;
        assert_eq!(format!("{plant:?}"), "Drying");
        assert_eq!(format!("{irrigation:?}"), "Drying");
        assert_ne!(
            std::any::TypeId::of::<PlantState>(),
            std::any::TypeId::of::<crate::state::IrrigationState>()
        );
    }
}
