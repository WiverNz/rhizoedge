//! The irrigation state machine (M6-006).
//!
//! [`evaluate`] is **the only public decision function in this crate**, it is
//! **pure** — no `self`, no mutation, no I/O, no clock — and it is **total**:
//! every (state, input) pair yields a defined [`IrrigationDecision`], including
//! inputs that are absent, which resolve to a lockout through the gate
//! (F-060-10, F-060-11, ADR-006).
//!
//! Purity is what makes the safety argument affordable. The caller loads state
//! from SQLite, calls this, and persists the result; ten thousand adversarial
//! property cases therefore cost milliseconds rather than needing a database and
//! a broker.
//!
//! The transition table in
//! [PRD 060](../../../../docs/prd/060-irrigation-control-and-safety.md)
//! §"State model" is normative, and [`next_state`] is its "To" column. The two
//! functions are separate on purpose: `evaluate` answers *what to do*, which is
//! what the PRD fixes as the return type, and `next_state` answers *where that
//! leaves the plant*, which the caller persists in the same transaction as the
//! side effect.

use chrono::{DateTime, Duration, Utc};

use crate::recommend::Reason;
use crate::state::{IrrigationState, LockoutReason};

use super::budget;
use super::gate::safety_gate;
use super::no_delivery::{DeliveryEvidence, no_delivery_detected};
use super::types::{EvaluationMode, IrrigationDecision, IrrigationInputs};

/// Evaluates one tick, or one operator request.
///
/// The gate is the first statement, unconditionally, and `Locked` is therefore
/// reachable from every state on every call: a leak does not wait for a
/// convenient moment (F-060-01, F-060-12).
#[must_use]
pub fn evaluate(inputs: IrrigationInputs<'_>) -> IrrigationDecision {
    // ---------------------------------------------------------------- the gate
    if let Some(reason) = safety_gate(&inputs) {
        return IrrigationDecision::Lock { reason };
    }

    match inputs.mode {
        EvaluationMode::ManualRequest { ml } | EvaluationMode::RecommendedRequest { ml } => {
            operator_request(&inputs, ml)
        }
        EvaluationMode::Automatic => automatic(&inputs),
    }
}

/// An operator-initiated dose, after the gate has already passed.
fn operator_request(inputs: &IrrigationInputs<'_>, ml: f32) -> IrrigationDecision {
    // A volume the edge cannot interpret is not a volume it may command
    // (SAFETY-012). The device would refuse it too; refusing here means nothing
    // is persisted or published at all.
    if !ml.is_finite() || ml <= 0.0 {
        return IrrigationDecision::Lock {
            reason: LockoutReason::Uncertain,
        };
    }
    // One command in flight at a time. A second dose issued while the first has
    // neither settled nor expired is the shape SAFETY-001 exists to prevent, and
    // there is no reason to create it deliberately.
    if *inputs.state == IrrigationState::DoseIssued {
        return IrrigationDecision::Wait {
            until: inputs.now + inputs.automation.command_ttl,
        };
    }
    // SAFETY-006's second check, with the dose included.
    if !budget::dose_fits(
        inputs.delivered_last_24h_ml,
        ml,
        inputs.automation.max_daily_ml,
    ) {
        return IrrigationDecision::Lock {
            reason: LockoutReason::DailyLimit,
        };
    }
    IrrigationDecision::IssueDose {
        ml,
        reasons: operator_reasons(inputs, ml),
    }
}

/// The automatic cycle: PRD 060's transition table, row by row.
fn automatic(inputs: &IrrigationInputs<'_>) -> IrrigationDecision {
    match effective_state(inputs) {
        // `Locked` cannot survive here: the gate returned `None`, so whatever
        // held the plant has resolved. `effective_state` maps it to `Normal` and
        // the plant re-enters the cycle from the top.
        IrrigationState::Locked | IrrigationState::Normal | IrrigationState::Drying => {
            drying_cycle(inputs)
        }
        IrrigationState::DryConfirmed => drying_cycle(inputs),
        // A command is on the wire. Its result, or its expiry, moves the plant
        // on — the machine's job here is to not issue a second one.
        IrrigationState::DoseIssued => IrrigationDecision::Wait {
            until: inputs.now + inputs.automation.command_ttl,
        },
        IrrigationState::WaitForAbsorption => match inputs.wait_until {
            Some(until) if inputs.now < until => IrrigationDecision::Wait { until },
            // A `WaitForAbsorption` with no deadline is an inconsistent row.
            // Waiting a full absorption period is the conservative reading; the
            // alternative would shorten an absorption wait on a corrupt value.
            None => IrrigationDecision::Wait {
                until: inputs.now + inputs.automation.absorption,
            },
            Some(_) => IrrigationDecision::Idle,
        },
        IrrigationState::Recheck => recheck(inputs),
    }
}

/// `Normal` → `Drying` → `DryConfirmed` → `DoseIssued`.
fn drying_cycle(inputs: &IrrigationInputs<'_>) -> IrrigationDecision {
    // The gate has already refused an absent, invalid, or stale sample in
    // automatic mode, so a reading is present here. Should that ever stop being
    // true, `None` means "no evidence of dryness", which is `Idle`.
    let Some(vwc) = inputs.latest_soil.and_then(|s| s.moisture_vwc) else {
        return IrrigationDecision::Idle;
    };
    if vwc >= inputs.automation.target_min_vwc {
        return IrrigationDecision::Idle;
    }
    if inputs.dry_duration < inputs.automation.dry_confirm {
        return IrrigationDecision::Idle;
    }

    let reasons = dry_reasons(inputs, vwc);

    // `auto_watering_enabled` defaults to `false`, and a plant may sit here
    // indefinitely. Telling a person is the whole answer.
    if !inputs.auto_watering_enabled || !inputs.automation.connected_enabled {
        return IrrigationDecision::Recommend {
            ml: inputs.automation.dose_ml,
            reasons,
        };
    }
    if let Some(until) = cooldown_until(inputs) {
        return IrrigationDecision::Wait { until };
    }
    if u16::from(inputs.doses_this_cycle) >= inputs.automation.max_doses_per_cycle {
        return IrrigationDecision::Lock {
            reason: LockoutReason::MaxDosesReached,
        };
    }
    // An unreachable pump is not a safety lockout, it is nothing to do. The
    // operator-facing path routes a request for a *sleeping* device to a durable
    // intent instead (M6-022); the automatic loop simply waits for the device.
    if !inputs.device_online {
        return IrrigationDecision::Idle;
    }
    if !budget::dose_fits(
        inputs.delivered_last_24h_ml,
        inputs.automation.dose_ml,
        inputs.automation.max_daily_ml,
    ) {
        return IrrigationDecision::Lock {
            reason: LockoutReason::DailyLimit,
        };
    }
    IrrigationDecision::IssueDose {
        ml: inputs.automation.dose_ml,
        reasons,
    }
}

/// `Recheck`: did the dose work, and may the cycle continue?
fn recheck(inputs: &IrrigationInputs<'_>) -> IrrigationDecision {
    let evidence = delivery_evidence(inputs);
    if evidence.moisture_responded() {
        return IrrigationDecision::CycleComplete;
    }
    // Ordered before the dose-count rule so escalation stops on the *reason it
    // is dangerous*, not merely when the cycle happens to run out of doses.
    if no_delivery_detected(&evidence) {
        return IrrigationDecision::Lock {
            reason: LockoutReason::NoDeliveryDetected,
        };
    }
    if u16::from(inputs.doses_this_cycle) >= inputs.automation.max_doses_per_cycle {
        return IrrigationDecision::Lock {
            reason: LockoutReason::MaxDosesReached,
        };
    }
    // Still dry with doses left: back to `DryConfirmed`, where the cooldown, the
    // cap, and the gate are all consulted again before the next dose.
    IrrigationDecision::Idle
}

/// The evidence M6-017 judges, assembled from the inputs.
#[must_use]
pub fn delivery_evidence(inputs: &IrrigationInputs<'_>) -> DeliveryEvidence {
    DeliveryEvidence {
        pre_dose_vwc: inputs.pre_dose_soil.and_then(|s| s.moisture_vwc),
        latest_vwc: inputs.latest_soil.and_then(|s| s.moisture_vwc),
        pre_dose_grams: inputs.pre_dose_weight.and_then(|s| s.grams),
        latest_grams: inputs.latest_weight.and_then(|s| s.grams),
        has_weight_sensor: inputs.latest_weight.is_some() || inputs.pre_dose_weight.is_some(),
        recovery_delta_vwc: inputs.automation.recovery_delta_vwc,
        doses_this_cycle: inputs.doses_this_cycle,
    }
}

/// When the cooldown expires, or `None` if it already has.
fn cooldown_until(inputs: &IrrigationInputs<'_>) -> Option<DateTime<Utc>> {
    let completed = inputs.last_cycle_completed_at?;
    let until = completed + inputs.automation.cooldown;
    (inputs.now < until).then_some(until)
}

/// `Locked` folded away once the gate has cleared it.
///
/// Spelled out rather than written as `s => s` so that a state added later has
/// to be classified here too.
#[must_use]
pub const fn effective_state(inputs: &IrrigationInputs<'_>) -> IrrigationState {
    match *inputs.state {
        IrrigationState::Locked => IrrigationState::Normal,
        IrrigationState::Normal => IrrigationState::Normal,
        IrrigationState::Drying => IrrigationState::Drying,
        IrrigationState::DryConfirmed => IrrigationState::DryConfirmed,
        IrrigationState::DoseIssued => IrrigationState::DoseIssued,
        IrrigationState::WaitForAbsorption => IrrigationState::WaitForAbsorption,
        IrrigationState::Recheck => IrrigationState::Recheck,
    }
}

/// The "To" column of PRD 060's transition table.
///
/// Pure and total, exhaustive over both the decision and the state it came from.
/// The caller persists this together with the decision's side effect, in one
/// transaction (F-060-14).
#[must_use]
pub fn next_state(inputs: &IrrigationInputs<'_>, decision: &IrrigationDecision) -> IrrigationState {
    match decision {
        IrrigationDecision::Lock { .. } => IrrigationState::Locked,
        IrrigationDecision::IssueDose { .. } => IrrigationState::DoseIssued,
        IrrigationDecision::CycleComplete => IrrigationState::Normal,
        IrrigationDecision::Recommend { .. } => IrrigationState::DryConfirmed,
        // "Wait" always means "stay where you are": awaiting a result, waiting
        // out an absorption, or waiting out a cooldown in `DryConfirmed`.
        IrrigationDecision::Wait { .. } => effective_state(inputs),
        IrrigationDecision::Idle => match effective_state(inputs) {
            IrrigationState::WaitForAbsorption => IrrigationState::Recheck,
            IrrigationState::Recheck => IrrigationState::DryConfirmed,
            IrrigationState::Normal
            | IrrigationState::Drying
            | IrrigationState::DryConfirmed
            | IrrigationState::Locked => dry_phase(inputs),
            IrrigationState::DoseIssued => IrrigationState::DoseIssued,
        },
    }
}

/// Which of `Normal` / `Drying` / `DryConfirmed` the current reading implies.
fn dry_phase(inputs: &IrrigationInputs<'_>) -> IrrigationState {
    match inputs.latest_soil.and_then(|s| s.moisture_vwc) {
        Some(vwc) if vwc < inputs.automation.target_min_vwc => {
            if inputs.dry_duration >= inputs.automation.dry_confirm {
                IrrigationState::DryConfirmed
            } else {
                IrrigationState::Drying
            }
        }
        Some(_) | None => IrrigationState::Normal,
    }
}

/// The absorption deadline a completed dose sets.
#[must_use]
pub fn absorption_until(now: DateTime<Utc>, absorption: Duration) -> DateTime<Utc> {
    now + absorption
}

fn dry_reasons(inputs: &IrrigationInputs<'_>, vwc: f64) -> Vec<Reason> {
    let mut reasons = vec![
        Reason::MoistureBelowTarget {
            vwc,
            target_min: inputs.automation.target_min_vwc,
        },
        Reason::DryFor {
            minutes: inputs.dry_duration.num_minutes(),
            required: inputs.automation.dry_confirm.num_minutes(),
        },
    ];
    reasons.push(match inputs.last_cycle_completed_at {
        None => Reason::NeverWatered,
        Some(at) => Reason::LastWatering {
            hours_ago: hours(inputs.now.signed_duration_since(at)),
        },
    });
    reasons
}

fn operator_reasons(inputs: &IrrigationInputs<'_>, ml: f32) -> Vec<Reason> {
    let mut reasons = Vec::new();
    if let Some(vwc) = inputs.latest_soil.and_then(|s| s.moisture_vwc) {
        if vwc < inputs.automation.target_min_vwc {
            reasons.push(Reason::MoistureBelowTarget {
                vwc,
                target_min: inputs.automation.target_min_vwc,
            });
        } else {
            reasons.push(Reason::MoistureAtOrAboveTarget {
                vwc,
                target_min: inputs.automation.target_min_vwc,
            });
        }
    } else {
        // A manual dose is permitted with no reading at all (F-060-05), and the
        // ledger records that this is what happened.
        reasons.push(Reason::SampleMissing);
    }
    reasons.push(match inputs.last_cycle_completed_at {
        None => Reason::NeverWatered,
        Some(at) => Reason::LastWatering {
            hours_ago: hours(inputs.now.signed_duration_since(at)),
        },
    });
    debug_assert!(ml > 0.0, "the caller refuses a non-positive request");
    reasons
}

fn hours(d: Duration) -> f64 {
    d.num_milliseconds() as f64 / 3_600_000.0
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
#[allow(
    clippy::module_inception,
    reason = "the module name is the verification filter the issue quotes literally"
)]
mod machine {
    use super::super::gate::fixture::{Scene, now};
    use super::super::types::{LeakState, TankState};
    use super::*;
    use crate::profile::SoilSample;
    use chrono::Duration;
    use proptest::prelude::*;

    /// A plant that is dry, confirmed, automated, and permitted.
    fn ready() -> Scene {
        let mut scene = Scene::default();
        scene.automation.connected_enabled = true;
        scene.soil = SoilSample {
            moisture_vwc: Some(20.0),
            received_at: now(),
        };
        scene.state = IrrigationState::DryConfirmed;
        scene
    }

    // ------------------------------------------------- transition table rows

    /// any | the gate returns a reason | Locked(r)
    #[test]
    fn row_any_state_locks_when_the_gate_refuses() {
        let mut scene = ready();
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
            let decision = evaluate(inputs);
            assert_eq!(
                decision,
                IrrigationDecision::Lock {
                    reason: LockoutReason::Leak
                },
                "{state:?}"
            );
            assert_eq!(
                next_state(&scene.inputs(), &decision),
                IrrigationState::Locked
            );
        }
    }

    /// Locked(r) | r auto-clearable and resolved | Normal
    #[test]
    fn row_a_resolved_auto_clearing_lockout_returns_to_the_cycle() {
        let mut scene = Scene::at(IrrigationState::Locked);
        scene.soil.moisture_vwc = Some(40.0);
        let mut inputs = scene.inputs();
        inputs.active_lockout = Some(LockoutReason::TankLow);
        let decision = evaluate(inputs);
        assert_eq!(decision, IrrigationDecision::Idle);
        let inputs = scene.inputs();
        assert_eq!(next_state(&inputs, &decision), IrrigationState::Normal);
    }

    /// Locked(r) | r explicit | stays locked until an operator clears it
    #[test]
    fn row_an_explicit_lockout_does_not_clear_itself() {
        let scene = Scene::at(IrrigationState::Locked);
        let mut inputs = scene.inputs();
        inputs.active_lockout = Some(LockoutReason::MaxDosesReached);
        assert_eq!(
            evaluate(inputs),
            IrrigationDecision::Lock {
                reason: LockoutReason::MaxDosesReached
            }
        );
    }

    /// Normal | moisture < target_min | Drying
    #[test]
    fn row_normal_to_drying() {
        let mut scene = Scene::at(IrrigationState::Normal);
        scene.soil.moisture_vwc = Some(20.0);
        let mut inputs = scene.inputs();
        inputs.dry_duration = Duration::minutes(5);
        let decision = evaluate(inputs);
        assert_eq!(decision, IrrigationDecision::Idle);
        let mut inputs = scene.inputs();
        inputs.dry_duration = Duration::minutes(5);
        assert_eq!(next_state(&inputs, &decision), IrrigationState::Drying);
    }

    /// Drying | moisture >= target_min | Normal
    #[test]
    fn row_drying_to_normal() {
        let mut scene = Scene::at(IrrigationState::Drying);
        scene.soil.moisture_vwc = Some(40.0);
        let decision = evaluate(scene.inputs());
        assert_eq!(decision, IrrigationDecision::Idle);
        assert_eq!(
            next_state(&scene.inputs(), &decision),
            IrrigationState::Normal
        );
    }

    /// Drying | dry >= dry_confirm_minutes | DryConfirmed
    #[test]
    fn row_drying_to_dry_confirmed() {
        let mut scene = Scene::at(IrrigationState::Drying);
        scene.soil.moisture_vwc = Some(20.0);
        let inputs = scene.inputs();
        assert_eq!(inputs.dry_duration, Duration::minutes(45));
        let decision = evaluate(inputs);
        // Automation is off in the default scene, so the answer is advice.
        assert!(matches!(decision, IrrigationDecision::Recommend { .. }));
        assert_eq!(
            next_state(&scene.inputs(), &decision),
            IrrigationState::DryConfirmed
        );
    }

    /// DryConfirmed | auto disabled | emit a recommendation only
    #[test]
    fn row_dry_confirmed_with_automation_off_only_recommends() {
        let mut scene = ready();
        let mut inputs = scene.inputs();
        inputs.auto_watering_enabled = false;
        match evaluate(inputs) {
            IrrigationDecision::Recommend { ml, reasons } => {
                assert!((ml - 40.0).abs() < f32::EPSILON);
                let codes: Vec<&str> = reasons.iter().map(Reason::code).collect();
                assert!(codes.contains(&"moisture_below_target"), "{codes:?}");
                assert!(codes.contains(&"dry_for"), "{codes:?}");
            }
            other => panic!("{other:?}"),
        }
        // ...and the plant-level opt-in is honoured independently.
        scene.automation.connected_enabled = false;
        assert!(matches!(
            evaluate(scene.inputs()),
            IrrigationDecision::Recommend { .. }
        ));
    }

    /// DryConfirmed | auto enabled, cooldown elapsed, gate passes | DoseIssued
    #[test]
    fn row_dry_confirmed_to_dose_issued() {
        let scene = ready();
        let decision = evaluate(scene.inputs());
        match &decision {
            IrrigationDecision::IssueDose { ml, .. } => {
                assert!((ml - 40.0).abs() < f32::EPSILON);
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            next_state(&scene.inputs(), &decision),
            IrrigationState::DoseIssued
        );
    }

    /// DryConfirmed | cooldown still running | Wait, and the plant stays put
    #[test]
    fn row_cooldown_is_enforced_between_cycles() {
        let scene = ready();
        let mut inputs = scene.inputs();
        inputs.last_cycle_completed_at = Some(now() - Duration::hours(1));
        let decision = evaluate(inputs);
        assert_eq!(
            decision,
            IrrigationDecision::Wait {
                until: now() + Duration::hours(5)
            }
        );
        let mut inputs = scene.inputs();
        inputs.last_cycle_completed_at = Some(now() - Duration::hours(1));
        assert_eq!(
            next_state(&inputs, &decision),
            IrrigationState::DryConfirmed
        );

        // Past the cooldown, the same plant doses.
        let mut inputs = scene.inputs();
        inputs.last_cycle_completed_at = Some(now() - Duration::hours(6));
        assert!(matches!(
            evaluate(inputs),
            IrrigationDecision::IssueDose { .. }
        ));
    }

    /// DoseIssued | awaiting a result | Wait, and never a second command
    #[test]
    fn row_dose_issued_never_issues_a_second_command() {
        let mut scene = ready();
        scene.state = IrrigationState::DoseIssued;
        let decision = evaluate(scene.inputs());
        assert_eq!(
            decision,
            IrrigationDecision::Wait {
                until: now() + Duration::seconds(120)
            }
        );
        assert_eq!(
            next_state(&scene.inputs(), &decision),
            IrrigationState::DoseIssued
        );
    }

    /// WaitForAbsorption | now < wait_until | WaitForAbsorption
    /// WaitForAbsorption | now >= wait_until | Recheck
    #[test]
    fn row_wait_for_absorption_then_recheck() {
        let mut scene = ready();
        scene.state = IrrigationState::WaitForAbsorption;
        let mut inputs = scene.inputs();
        inputs.wait_until = Some(now() + Duration::minutes(10));
        let decision = evaluate(inputs);
        assert_eq!(
            decision,
            IrrigationDecision::Wait {
                until: now() + Duration::minutes(10)
            }
        );
        let mut inputs = scene.inputs();
        inputs.wait_until = Some(now() + Duration::minutes(10));
        assert_eq!(
            next_state(&inputs, &decision),
            IrrigationState::WaitForAbsorption
        );

        let mut inputs = scene.inputs();
        inputs.wait_until = Some(now() - Duration::seconds(1));
        let decision = evaluate(inputs);
        assert_eq!(decision, IrrigationDecision::Idle);
        let mut inputs = scene.inputs();
        inputs.wait_until = Some(now() - Duration::seconds(1));
        assert_eq!(next_state(&inputs, &decision), IrrigationState::Recheck);
    }

    /// A `WaitForAbsorption` row with no deadline waits rather than shortening
    /// the absorption on a corrupt value.
    #[test]
    fn an_absorption_wait_with_no_deadline_is_conservative() {
        let mut scene = ready();
        scene.state = IrrigationState::WaitForAbsorption;
        let mut inputs = scene.inputs();
        inputs.wait_until = None;
        assert_eq!(
            evaluate(inputs),
            IrrigationDecision::Wait {
                until: now() + Duration::minutes(30)
            }
        );
    }

    /// Recheck | moisture >= pre-dose + recovery_delta | CycleComplete
    #[test]
    fn row_recheck_recovered_completes_the_cycle() {
        let mut scene = ready();
        scene.state = IrrigationState::Recheck;
        scene.pre_dose.moisture_vwc = Some(20.0);
        scene.soil.moisture_vwc = Some(27.0);
        let mut inputs = scene.inputs();
        inputs.doses_this_cycle = 1;
        let decision = evaluate(inputs);
        assert_eq!(decision, IrrigationDecision::CycleComplete);
        assert_eq!(
            next_state(&scene.inputs(), &decision),
            IrrigationState::Normal
        );
    }

    /// Recheck | still dry, doses < max | DryConfirmed
    #[test]
    fn row_recheck_still_dry_goes_back_for_another_dose() {
        let mut scene = ready();
        scene.state = IrrigationState::Recheck;
        scene.pre_dose.moisture_vwc = Some(20.0);
        scene.soil.moisture_vwc = Some(20.5);
        let mut inputs = scene.inputs();
        inputs.doses_this_cycle = 1;
        let decision = evaluate(inputs);
        assert_eq!(decision, IrrigationDecision::Idle);
        let mut inputs = scene.inputs();
        inputs.doses_this_cycle = 1;
        assert_eq!(
            next_state(&inputs, &decision),
            IrrigationState::DryConfirmed
        );
    }

    /// Recheck | still dry, doses = max | Locked(MaxDosesReached)
    #[test]
    fn row_recheck_at_the_dose_limit_locks_out() {
        let mut scene = ready();
        scene.state = IrrigationState::Recheck;
        scene.pre_dose.moisture_vwc = Some(20.0);
        scene.soil.moisture_vwc = Some(20.5);
        let mut inputs = scene.inputs();
        inputs.doses_this_cycle = 3;
        // A weight response keeps no-delivery detection out of the way, so this
        // test is about the dose limit and nothing else.
        let weight_before = super::super::types::WeightSample {
            grams: Some(1_800.0),
            received_at: now(),
        };
        let weight_after = super::super::types::WeightSample {
            grams: Some(1_900.0),
            received_at: now(),
        };
        inputs.pre_dose_weight = Some(&weight_before);
        inputs.latest_weight = Some(&weight_after);
        assert_eq!(
            evaluate(inputs),
            IrrigationDecision::Lock {
                reason: LockoutReason::MaxDosesReached
            }
        );
    }

    /// Recheck | two doses, no moisture and no weight response |
    /// Locked(NoDeliveryDetected), and it outranks the dose limit.
    #[test]
    fn row_recheck_no_delivery_stops_escalation() {
        let mut scene = ready();
        scene.state = IrrigationState::Recheck;
        scene.pre_dose.moisture_vwc = Some(20.0);
        scene.soil.moisture_vwc = Some(20.1);
        let mut inputs = scene.inputs();
        inputs.doses_this_cycle = 2;
        assert_eq!(
            evaluate(inputs),
            IrrigationDecision::Lock {
                reason: LockoutReason::NoDeliveryDetected
            }
        );
    }

    /// A single unresponsive dose is not a fault; the cycle continues.
    #[test]
    fn one_unresponsive_dose_does_not_lock_the_plant() {
        let mut scene = ready();
        scene.state = IrrigationState::Recheck;
        scene.pre_dose.moisture_vwc = Some(20.0);
        scene.soil.moisture_vwc = Some(20.1);
        let mut inputs = scene.inputs();
        inputs.doses_this_cycle = 1;
        assert_eq!(evaluate(inputs), IrrigationDecision::Idle);
    }

    // ------------------------------------------------------- the cap and modes

    /// SAFETY-006's second check: a dose that would cross the cap is not issued.
    #[test]
    fn a_dose_that_would_cross_the_cap_is_not_issued() {
        let scene = ready();
        let mut inputs = scene.inputs();
        inputs.delivered_last_24h_ml = 270.0;
        assert_eq!(
            evaluate(inputs),
            IrrigationDecision::Lock {
                reason: LockoutReason::DailyLimit
            },
            "270 + 40 crosses the 300 ml ceiling"
        );
        let mut inputs = scene.inputs();
        inputs.delivered_last_24h_ml = 260.0;
        assert!(matches!(
            evaluate(inputs),
            IrrigationDecision::IssueDose { .. }
        ));
    }

    /// An offline device is nothing to do, not a lockout.
    #[test]
    fn an_offline_device_is_idle_rather_than_locked() {
        let scene = ready();
        let mut inputs = scene.inputs();
        inputs.device_online = false;
        assert_eq!(evaluate(inputs), IrrigationDecision::Idle);
    }

    /// A manual request waters a plant whose sensor is broken, and refuses a
    /// volume nobody can interpret.
    #[test]
    fn a_manual_request_waters_under_sensor_fault_but_not_on_nonsense() {
        let mut scene = Scene::default();
        scene.soil.moisture_vwc = None;
        let mut inputs = scene.inputs();
        inputs.mode = EvaluationMode::ManualRequest { ml: 30.0 };
        match evaluate(inputs) {
            IrrigationDecision::IssueDose { ml, .. } => {
                assert!((ml - 30.0).abs() < f32::EPSILON);
            }
            other => panic!("{other:?}"),
        }
        for bad in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            let mut inputs = scene.inputs();
            inputs.mode = EvaluationMode::ManualRequest { ml: bad };
            assert_eq!(
                evaluate(inputs),
                IrrigationDecision::Lock {
                    reason: LockoutReason::Uncertain
                },
                "{bad}"
            );
        }
    }

    /// A manual request never becomes a second in-flight command.
    #[test]
    fn a_manual_request_waits_while_a_command_is_in_flight() {
        let scene = Scene::at(IrrigationState::DoseIssued);
        let mut inputs = scene.inputs();
        inputs.mode = EvaluationMode::ManualRequest { ml: 30.0 };
        assert_eq!(
            evaluate(inputs),
            IrrigationDecision::Wait {
                until: now() + Duration::seconds(120)
            }
        );
    }

    /// A manual dose obeys the cap.
    #[test]
    fn a_manual_request_obeys_the_rolling_cap() {
        let scene = Scene::default();
        let mut inputs = scene.inputs();
        inputs.mode = EvaluationMode::ManualRequest { ml: 40.0 };
        inputs.delivered_last_24h_ml = 280.0;
        assert_eq!(
            evaluate(inputs),
            IrrigationDecision::Lock {
                reason: LockoutReason::DailyLimit
            }
        );
    }

    /// A `recommended` request does **not** inherit the manual privilege.
    #[test]
    fn a_recommended_request_is_still_blocked_by_stale_data() {
        let mut scene = Scene::default();
        scene.soil.received_at = now() - Duration::hours(4);
        let mut inputs = scene.inputs();
        inputs.mode = EvaluationMode::RecommendedRequest { ml: 30.0 };
        assert_eq!(
            evaluate(inputs),
            IrrigationDecision::Lock {
                reason: LockoutReason::StaleData
            }
        );
    }

    /// A full multi-dose cycle: dose, absorb, recheck, dose, recover.
    #[test]
    fn a_multi_dose_cycle_runs_to_completion() {
        let mut scene = ready();

        // Dose one.
        assert!(matches!(
            evaluate(scene.inputs()),
            IrrigationDecision::IssueDose { .. }
        ));

        // Absorbing.
        scene.state = IrrigationState::WaitForAbsorption;
        let mut inputs = scene.inputs();
        inputs.wait_until = Some(now() + Duration::minutes(1));
        inputs.doses_this_cycle = 1;
        assert!(matches!(evaluate(inputs), IrrigationDecision::Wait { .. }));

        // Recheck, still dry.
        scene.state = IrrigationState::Recheck;
        scene.pre_dose.moisture_vwc = Some(20.0);
        scene.soil.moisture_vwc = Some(21.0);
        let mut inputs = scene.inputs();
        inputs.doses_this_cycle = 1;
        assert_eq!(evaluate(inputs), IrrigationDecision::Idle);

        // Dose two.
        scene.state = IrrigationState::DryConfirmed;
        let mut inputs = scene.inputs();
        inputs.doses_this_cycle = 1;
        assert!(matches!(
            evaluate(inputs),
            IrrigationDecision::IssueDose { .. }
        ));

        // Recheck, recovered.
        scene.state = IrrigationState::Recheck;
        scene.soil.moisture_vwc = Some(27.0);
        let mut inputs = scene.inputs();
        inputs.doses_this_cycle = 2;
        assert_eq!(evaluate(inputs), IrrigationDecision::CycleComplete);
    }

    // ----------------------------------------------------------- the property

    /// A strategy over every state crossed with adversarial inputs.
    #[allow(clippy::too_many_arguments)]
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

    proptest! {
        /// F-060-11. Every (state, input) pair yields a defined decision, and
        /// nothing panics — including on `NaN`, absent readings, and impossible
        /// combinations. A partial function would panic in production on an
        /// input nobody anticipated.
        #[test]
        fn prop_state_machine_total(
            state in any_state(),
            moisture in proptest::option::of(prop_oneof![
                Just(f64::NAN), Just(f64::INFINITY), -50.0f64..200.0
            ]),
            pre_dose in proptest::option::of(-50.0f64..200.0),
            tank_percent in proptest::option::of(prop_oneof![Just(f64::NAN), -10.0f64..150.0]),
            leak in prop_oneof![
                Just(LeakState::Clear), Just(LeakState::Detected), Just(LeakState::Unknown)
            ],
            delivered in prop_oneof![Just(f32::NAN), -10.0f32..1_000.0],
            doses in 0u8..10,
            dry_minutes in 0i64..600,
            age_seconds in 0i64..100_000,
            auto in any::<bool>(),
            online in any::<bool>(),
            reconciling in any::<bool>(),
            manual in any::<bool>(),
        ) {
            let mut scene = Scene::at(state);
            scene.automation.connected_enabled = true;
            scene.soil = SoilSample {
                moisture_vwc: moisture,
                received_at: now() - Duration::seconds(age_seconds),
            };
            scene.pre_dose = SoilSample {
                moisture_vwc: pre_dose,
                received_at: now() - Duration::minutes(60),
            };
            let mut inputs = scene.inputs();
            inputs.tank = tank_percent.map(|percent| TankState::Level {
                percent,
                age: Duration::seconds(age_seconds),
            });
            inputs.leak = leak;
            inputs.delivered_last_24h_ml = delivered;
            inputs.doses_this_cycle = doses;
            inputs.dry_duration = Duration::minutes(dry_minutes);
            inputs.auto_watering_enabled = auto;
            inputs.device_online = online;
            inputs.reconciling = reconciling;
            if manual {
                inputs.mode = EvaluationMode::ManualRequest { ml: 30.0 };
            }

            let decision = evaluate(inputs);
            // The decision is one of the six, and a next state always exists.
            let mut again = scene.inputs();
            again.tank = tank_percent.map(|percent| TankState::Level {
                percent,
                age: Duration::seconds(age_seconds),
            });
            again.leak = leak;
            again.doses_this_cycle = doses;
            again.dry_duration = Duration::minutes(dry_minutes);
            let _ = next_state(&again, &decision);
            prop_assert!(!decision.as_str().is_empty());

            // ...and a dose is never issued while anything is uncertain.
            if decision.actuates() {
                prop_assert_eq!(leak, LeakState::Clear);
                prop_assert!(!reconciling);
                prop_assert!(tank_percent.is_some_and(|p| p.is_finite() && p > 15.0));
                prop_assert!(delivered.is_finite());
            }
        }
    }
}
