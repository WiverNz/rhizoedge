//! Autonomous watering while isolated — **the one call site** of the shared
//! evaluator (M6-019, ADR-015, ADR-008).
//!
//! M2 built the seam and stopped deliberately: it gathered inputs, persisted
//! monotonic state, and classified nothing. This module adds the one line M2 was
//! shaped around — `rhizo_policy::evaluate_offline(..)` — and the scheduling that
//! turns its answer into a dose.
//!
//! # Why there is exactly one call, and one actuation path
//!
//! A simulator-specific evaluator, even a small one, even a temporary one, would
//! make every offline safety test in M6 and every isolation scenario in M8
//! exercise rules the hardware does not follow. `tests/single_actuation_path.rs`
//! counts the call sites and fails at two.
//!
//! The dose it produces goes through [`crate::device::Device::autonomous_dose`],
//! which reaches `start_pump` through the same `begin_dose` an edge command does
//! — the same in-flight NVS write before actuation (SAFETY-011), the same dedup
//! ring, the same firmware hard limits through `bound_dose` (SAFETY-007,
//! SAFETY-014).
//!
//! # What an autonomous dose does *not* go through, and why
//!
//! `validate_water_command` steps 2 and 3 — `clock_unsynced` and `expired` — are
//! about a **command**: a decision another machine made at a wall-clock instant
//! this device must be able to compare against. An autonomous dose has no
//! issuer, no TTL, and by construction no synchronised clock; SAFETY-015 says
//! plainly that this invariant governs the autonomous path and SAFETY-002 governs
//! the commanded one. Applying the TTL check here would mean an isolated device
//! could never water at all, which is the entire feature.
//!
//! Everything that bounds *water* still applies, and applies from the same
//! shared function.
//!
//! # Autonomy is opt-in, and isolation-only
//!
//! A connected device takes its instructions from the Edge. Evaluating while
//! connected would create the second control path ADR-015 §9 is careful to say
//! it did not create.

use rhizo_mqtt_contract::payload::{EventDetail, EventKind, EventTier};
use rhizo_policy::{OfflineDecision, RefuseReason, evaluate_offline, next_offline_state};

use crate::device::Device;
use crate::envelope::Publication;

impl Device {
    /// Evaluates offline autonomy for this tick.
    ///
    /// `elapsed_ms` is the monotonic time the device actually **observed**, and
    /// is the only time the evaluator is given. A reboot credits zero, which is
    /// what stops a reboot loop earning water (SAFETY-015).
    pub fn evaluate_offline_autonomy(&mut self, elapsed_ms: u64) -> Vec<Publication> {
        // Mode C only. A connected device is told what to do.
        if self.is_connected() {
            return Vec::new();
        }
        // A device that cannot trust its stored safety history must not water,
        // whatever any evaluator would say: the budget, the cooldown, and the
        // dedup ring are exactly the state that is in doubt.
        if !self.actuation_permitted() {
            return Vec::new();
        }
        let Some(plant_id) = self.policy_plants().into_iter().next() else {
            // No activated policy is not permission (SAFETY-013). It is the
            // documented behaviour of an unprovisioned device: a data logger.
            return Vec::new();
        };
        let Some(seam) = self.offline_seam(&plant_id, elapsed_ms) else {
            return Vec::new();
        };

        // ------------------------------------------------------- the one call
        let decision = evaluate_offline(seam.policy, &seam.state, &seam.inputs, seam.elapsed);
        let next = next_offline_state(seam.policy, &seam.state, &decision, seam.elapsed);
        // `seam` borrows the active policy; the write below needs the device.
        let policy_version = seam.policy.policy_version;
        // Taken from the policy that was actually evaluated, not from the
        // `plant_id` string the lookup started with: the buffered event has to
        // name the same plant the decision was made for, in the contract's own
        // identifier type.
        let dosed_plant = seam.policy.plant_id.clone();
        drop(seam);
        self.apply_offline_decision(&next);

        match decision {
            OfflineDecision::Dose { ml } => {
                tracing::warn!(
                    plant_id = %plant_id,
                    ml,
                    policy_version,
                    "autonomous dose scheduled while isolated"
                );
                let publications = self.autonomous_dose(ml);
                // The audit record of what the machine did to a living plant.
                // Buffered rather than published, because there is nobody to
                // publish to — that is what being isolated means.
                let trigger = self.offline_control_value(&plant_id).unwrap_or(f64::NAN);
                self.record_event(
                    EventTier::Audit,
                    EventKind::WateringOfflineAutonomous,
                    EventDetail::Watering {
                        // The dose names its own subject. `plant_id` is the
                        // plant whose policy was just evaluated -- the only
                        // fact about ownership that is true at the moment the
                        // water goes into the pot. The edge would otherwise
                        // have to infer it from bindings that may have been
                        // edited while this device was alone, and charge the
                        // wrong budget in both directions at once.
                        plant_id: Some(dosed_plant),
                        policy_version,
                        delivered_ml: ml,
                        trigger_value: trigger,
                        duration_ms: 0,
                    },
                );
                self.note_offline_refusal(None);
                publications
            }
            OfflineDecision::Refuse(reason) => {
                // One event per *change* of reason. A leak that lasts a week
                // would otherwise fill the 64-slot audit ring with the same
                // sentence and evict the record of the dose that matters
                // (SAFETY-020).
                if self.note_offline_refusal(Some(reason)) {
                    tracing::warn!(
                        plant_id = %plant_id,
                        reason = ?reason,
                        "autonomous watering refused"
                    );
                    self.record_event(
                        EventTier::Audit,
                        EventKind::OfflineRefused,
                        EventDetail::Refused {
                            reason: refuse_reason_name(reason).to_owned(),
                        },
                    );
                }
                Vec::new()
            }
            OfflineDecision::Idle
            | OfflineDecision::Confirming
            | OfflineDecision::WaitAbsorption
            | OfflineDecision::Cooldown => {
                self.note_offline_refusal(None);
                Vec::new()
            }
        }
    }
}

/// The stable name a refusal is buffered under.
///
/// Exhaustive with no catch-all, so a refusal reason added to the shared crate
/// has to be named here before it can be recorded.
#[must_use]
pub const fn refuse_reason_name(reason: RefuseReason) -> &'static str {
    match reason {
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
