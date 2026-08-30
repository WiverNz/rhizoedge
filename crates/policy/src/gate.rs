//! The offline gate: the ordered veto an isolated device runs before any
//! irrigation logic (M6-019,
//! [offline-autonomy.md](../../../docs/architecture/offline-autonomy.md) §4).
//!
//! The same discipline as the Edge's gate, in a third of the code and with no
//! allocation: `Option` and tri-state for every absent-able input, exhaustive
//! matches, **no catch-all arm**. Adding a `LeakState` variant fails to compile
//! here too, which is the point of writing it twice rather than sharing a
//! function that would need `std`.
//!
//! ```text
//! 1. enabled == false                     -> Refuse(PolicyDisabled)
//! 2. policy invalid / no actuator         -> Refuse(PolicyInvalid | NoActuator)
//! 3. leak detected or unknown             -> Refuse(LeakDetected | LeakUnknown)
//! 4. tank below minimum or unknown        -> Refuse(TankLow | TankUnknown)
//! 5. pump faulted or unknown              -> Refuse(PumpUnhealthy | PumpUnknown)
//! 6. a required measurement missing/stale -> Refuse(Required*)
//! 7. control measurement invalid          -> Refuse(Control*)
//! 8. rolling budget exhausted             -> Refuse(BudgetExhausted)
//!      -- only then is irrigation logic evaluated --
//! ```
//!
//! # SAFETY-013: absence of a policy is not permission
//!
//! A device with no policy never reaches this function at all — the caller has
//! nothing to pass — and a device whose policy is disabled or invalid is refused
//! at steps 1 and 2. A device that invents a threshold because it has none is
//! more dangerous than a device that does nothing, because nobody authorised
//! what it does and nobody can predict it.
//!
//! # SAFETY-017: requirements are declared, not inferred
//!
//! A plant whose policy requires tank level will not water when the tank sensor
//! is silent. A plant whose policy does not require pot weight is **unaffected**
//! by the absence of a scale. Both halves matter, and the second is the one that
//! is easy to forget.

use rhizo_mqtt_contract::payload::{MeasurementValue, Quality};
use rhizo_mqtt_contract::payload::{OfflinePolicy, RequiredMeasurement};
use rhizo_mqtt_contract::safety::LeakState;

use crate::types::{MonotonicMillis, OfflineInputs, OfflineSample, RefuseReason};

/// Runs the ordered veto. `None` means every check passed.
///
/// Takes the persisted state as well as the inputs, because the last two steps
/// — the rolling budget and the per-cycle dose count — are facts about what this
/// device has already done, not about what it can currently see.
#[must_use]
pub fn offline_gate(
    policy: &OfflinePolicy,
    state: &crate::types::OfflineState,
    inputs: &OfflineInputs,
) -> Option<RefuseReason> {
    // 1. Offline autonomy is opted into per plant, by a human. `enabled`
    //    defaults to false, which is SAFETY-012 applied to provisioning.
    if !policy.enabled {
        return Some(RefuseReason::PolicyDisabled);
    }
    // 2. The Edge validated it, and the device re-validates it. A policy that
    //    fails here is not repaired, guessed at, or partially applied.
    if policy.validate().is_err() {
        return Some(RefuseReason::PolicyInvalid);
    }
    let Some(actuator) = policy.actuator.as_ref() else {
        return Some(RefuseReason::NoActuator);
    };

    // 3. Leak. `require_leak_clear` is the policy's declaration that this plant
    //    has a tray sensor; when it is set, `Unknown` refuses like `Detected`.
    if policy.safety.require_leak_clear {
        match inputs.leak {
            None => return Some(RefuseReason::LeakUnknown),
            Some(LeakState::Detected) => return Some(RefuseReason::LeakDetected),
            Some(LeakState::Unknown) => return Some(RefuseReason::LeakUnknown),
            Some(LeakState::Clear) => {}
        }
    }

    // 4. Tank. An unreadable level is `TankUnknown`, never `TankLow`: the latter
    //    is a *measured* condition, exactly as protocol §5.8 step 7 has it.
    match inputs.tank_percent {
        None => return Some(RefuseReason::TankUnknown),
        Some(percent) if !percent.is_finite() => return Some(RefuseReason::TankUnknown),
        Some(percent) => {
            if percent <= policy.safety.require_tank_above_percent {
                return Some(RefuseReason::TankLow);
            }
        }
    }

    // 5. Pump.
    if policy.safety.require_pump_healthy {
        match inputs.pump_healthy {
            None => return Some(RefuseReason::PumpUnknown),
            Some(false) => return Some(RefuseReason::PumpUnhealthy),
            Some(true) => {}
        }
    }

    // 6. Required measurements, by declaration.
    for required in &policy.required_measurements {
        if let Some(reason) = check_required(required, inputs) {
            return Some(reason);
        }
    }

    // 7. The control measurement.
    if let Some(reason) = check_control(policy, inputs) {
        return Some(reason);
    }

    // 8. The rolling budget, checked **with the dose included**, so a dose that
    //    would cross the window cap is never delivered at all (SAFETY-014).
    if !actuator.dose_ml.is_finite() || !policy.limits.max_volume_per_window_ml.is_finite() {
        return Some(RefuseReason::PolicyInvalid);
    }
    if budget_exhausted(policy, state.budget_used_ml) {
        return Some(RefuseReason::BudgetExhausted);
    }
    if doses_exhausted(policy, state.dose_count) {
        return Some(RefuseReason::MaxDosesReached);
    }
    None
}

/// The budget half of step 8, separated because it needs the persisted state.
#[must_use]
pub fn budget_exhausted(policy: &OfflinePolicy, used_ml: f32) -> bool {
    let Some(actuator) = policy.actuator.as_ref() else {
        return true;
    };
    // A device that cannot prove it is under budget assumes it is not, exactly
    // as protocol §5.8 step 11 requires.
    if !used_ml.is_finite() || !actuator.dose_ml.is_finite() {
        return true;
    }
    used_ml + actuator.dose_ml > policy.limits.max_volume_per_window_ml
}

/// Whether the cycle has already delivered every dose it may.
#[must_use]
pub fn doses_exhausted(policy: &OfflinePolicy, dose_count: u16) -> bool {
    policy
        .actuator
        .as_ref()
        .is_none_or(|actuator| dose_count >= actuator.max_doses_per_cycle)
}

fn check_required(required: &RequiredMeasurement, inputs: &OfflineInputs) -> Option<RefuseReason> {
    // A required kind the device has never sampled is absent from `inputs`
    // entirely, and absence is the case that must refuse (SAFETY-017).
    let Some(sample) = inputs
        .required
        .iter()
        .find(|sample| sample.kind == required.kind)
    else {
        return Some(RefuseReason::RequiredMissing);
    };
    if sample.value.is_none() {
        return Some(RefuseReason::RequiredMissing);
    }
    // `Uncalibrated`, `Suspect`, and `Fault` are all unusable. Only `Ok` is a
    // reading the gate may act on.
    match sample.quality {
        Quality::Ok => {}
        Quality::Uncalibrated | Quality::Suspect | Quality::Fault => {
            return Some(RefuseReason::RequiredQuality);
        }
    }
    if sample.age >= MonotonicMillis(u64::from(required.max_age_ms)) {
        return Some(RefuseReason::RequiredStale);
    }
    None
}

fn check_control(policy: &OfflinePolicy, inputs: &OfflineInputs) -> Option<RefuseReason> {
    let Some(sample) = inputs.control.as_ref() else {
        return Some(RefuseReason::ControlMissing);
    };
    if !policy.control_measurement.kind.is_known() {
        return Some(RefuseReason::ControlKindUnknown);
    }
    match sample.quality {
        Quality::Ok => {}
        Quality::Uncalibrated | Quality::Suspect | Quality::Fault => {
            return Some(RefuseReason::ControlQuality);
        }
    }
    if sample.age >= MonotonicMillis(u64::from(policy.control_measurement.max_age_ms)) {
        return Some(RefuseReason::ControlStale);
    }
    match control_value(sample) {
        None => Some(RefuseReason::ControlMissing),
        Some(value) if !value.is_finite() => Some(RefuseReason::ControlMissing),
        Some(_) => None,
    }
}

/// The scalar the control rule compares.
///
/// A boolean reading has no meaningful hysteresis, so it is not a control value
/// at all; the policy validator refuses a boolean control kind, and this refuses
/// a boolean *reading* of a scalar kind for the same reason.
#[must_use]
pub fn control_value(sample: &OfflineSample) -> Option<f64> {
    match sample.value {
        Some(MeasurementValue::Scalar(value)) => Some(value),
        Some(MeasurementValue::Boolean(_)) | None => None,
    }
}
