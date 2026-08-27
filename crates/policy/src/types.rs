//! State, inputs, and decisions for the bounded offline evaluator.
use alloc::vec::Vec;
use rhizo_mqtt_contract::{
    payload::{MeasurementKind, MeasurementValue, Quality},
    safety::LeakState,
};

/// Monotonic elapsed milliseconds supplied by the caller.
#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct MonotonicMillis(pub u64);
/// Persistable evaluator cycle phase.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum OfflineCycle {
    #[default]
    Idle,
    Confirming,
    WaitAbsorption,
    Cooldown,
}
/// Persistable conservative evaluator state.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct OfflineState {
    /// Cycle phase.
    pub cycle: OfflineCycle,
    /// Doses in current cycle.
    pub dose_count: u16,
    /// Used rolling budget.
    pub budget_used_ml: f32,
    /// Remaining cooldown.
    pub cooldown_remaining: MonotonicMillis,
    /// Confirmation elapsed.
    pub confirm_elapsed: MonotonicMillis,
}
/// One most-recent measurement.
#[derive(Clone, Debug, PartialEq)]
pub struct OfflineSample {
    /// Kind.
    pub kind: MeasurementKind,
    /// Typed value or failed read.
    pub value: Option<MeasurementValue>,
    /// Quality.
    pub quality: Quality,
    /// Monotonic age.
    pub age: MonotonicMillis,
}
/// Complete fail-closed inputs.
#[derive(Clone, Debug, PartialEq)]
pub struct OfflineInputs {
    /// Control measurement.
    pub control: Option<OfflineSample>,
    /// Required measurements.
    pub required: Vec<OfflineSample>,
    /// Leak veto.
    pub leak: Option<LeakState>,
    /// Tank reading.
    pub tank_percent: Option<f32>,
    /// Pump health.
    pub pump_healthy: Option<bool>,
}
/// Refusal for every offline gate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RefuseReason {
    PolicyDisabled,
    PolicyInvalid,
    NoActuator,
    ControlMissing,
    ControlStale,
    ControlQuality,
    ControlKindUnknown,
    RequiredMissing,
    RequiredStale,
    RequiredQuality,
    LeakDetected,
    LeakUnknown,
    TankUnknown,
    TankLow,
    PumpUnknown,
    PumpUnhealthy,
    CooldownActive,
    BudgetExhausted,
    MaxDosesReached,
}
/// Pure evaluator output vocabulary; evaluation logic lands in M6-019.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum OfflineDecision {
    Idle,
    Confirming,
    Dose {
        /// Fixed policy volume.
        ml: f32,
    },
    WaitAbsorption,
    Cooldown,
    Refuse(RefuseReason),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absent_safety_inputs_are_explicit() {
        let inputs = OfflineInputs {
            control: None,
            required: Vec::new(),
            leak: None,
            tank_percent: None,
            pump_healthy: None,
        };
        assert!(inputs.control.is_none());
        assert!(inputs.leak.is_none());
        assert!(inputs.tank_percent.is_none());
        assert!(inputs.pump_healthy.is_none());
    }

    #[test]
    fn state_and_decisions_construct_without_a_clock() {
        let state = OfflineState {
            cooldown_remaining: MonotonicMillis(10),
            ..OfflineState::default()
        };
        assert_eq!(state.cooldown_remaining, MonotonicMillis(10));
        assert_eq!(
            OfflineDecision::Refuse(RefuseReason::PolicyDisabled),
            OfflineDecision::Refuse(RefuseReason::PolicyDisabled)
        );
    }
}
