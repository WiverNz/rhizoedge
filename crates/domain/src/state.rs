//! Stable state vocabulary; transition logic is intentionally absent.
use serde::{Deserialize, Serialize};
/// Plant summary state.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlantState {
    Healthy,
    Drying,
    WaterRecommended,
    WaitingForResponse,
    Recovering,
    SensorFault,
    WateringLocked,
    #[serde(other)]
    Unknown,
}
/// Detailed irrigation state.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IrrigationState {
    Normal,
    Drying,
    DryConfirmed,
    DoseIssued,
    WaitForAbsorption,
    Recheck,
    Locked,
}
/// Conservative actuation lockout.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LockoutReason {
    Leak,
    TankLow,
    StaleData,
    SensorFault,
    DailyLimit,
    MaxDosesReached,
    NoDeliveryDetected,
    Uncertain,
    ClockUnsynced,
    PumpFault,
    NoActuator,
    #[serde(other)]
    Unknown,
}
/// Origin/mode of watering.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WateringMode {
    Manual,
    ConnectedAutomatic,
    OfflineAutonomous,
    #[serde(other)]
    Unknown,
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stable_snake_case() {
        assert_eq!(
            serde_json::to_string(&PlantState::WaterRecommended).unwrap(),
            "\"water_recommended\""
        );
        assert_eq!(
            serde_json::to_string(&LockoutReason::ClockUnsynced).unwrap(),
            "\"clock_unsynced\""
        );
    }
}
