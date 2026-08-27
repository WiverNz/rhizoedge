//! Restricted offline-autonomy policy contract.
use super::{ActuatorKind, MeasurementKind, MeasurementPoint, SensorId};
use crate::safety::{FIRMWARE_MAX_DAILY_ML, FIRMWARE_MAX_ML_PER_RUN};
use alloc::vec::Vec;
use serde::{Deserialize, Serialize};
/// A retained set of policies served by one device.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OfflinePolicySet {
    /** Per-plant policies. */
    pub policies: Vec<OfflinePolicy>,
}
/// One plant's bounded offline policy.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OfflinePolicy {
    /** Plant identity. */
    pub plant_id: SensorId,
    /** Strictly monotonic version. */
    pub policy_version: u32,
    #[serde(default)]
    /** Explicit opt-in; absent is disabled. */
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /** Optional actuator relationship. */
    pub actuator: Option<OfflineActuator>,
    /** Trigger and hysteresis. */
    pub control_measurement: ControlMeasurement,
    #[serde(default)]
    /** Safety-gating measurements. */
    pub required_measurements: Vec<RequiredMeasurement>,
    #[serde(default)]
    /** Non-gating measurements. */
    pub advisory_measurements: Vec<AdvisoryMeasurement>,
    /** Cooldown/window budget. */
    pub limits: OfflineLimits,
    /** Hardware safety inputs required. */
    pub safety: OfflineSafety,
}
/// Offline actuator and dose limits.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OfflineActuator {
    /** Declared capability. */
    pub actuator_id: SensorId,
    #[serde(default = "irrigation_pump")]
    /** Must be an irrigation pump in v1. */
    pub kind: ActuatorKind,
    /** Fixed dose. */
    pub dose_ml: f32,
    /** Per-cycle bound. */
    pub max_doses_per_cycle: u16,
    /** Absorption delay. */
    pub absorption_wait_ms: u32,
}
fn irrigation_pump() -> ActuatorKind {
    ActuatorKind::IrrigationPump
}
/// Offline trigger rule.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ControlMeasurement {
    /** Known control kind. */
    pub kind: MeasurementKind,
    /** Bound point. */
    pub point: MeasurementPoint,
    /** Dry threshold. */
    pub trigger_below: f64,
    /** Hysteresis resume threshold. */
    pub resume_above: f64,
    /** Confirmation duration. */
    pub confirm_duration_ms: u32,
    /** Freshness bound. */
    pub max_age_ms: u32,
}
/// Required measurement binding.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RequiredMeasurement {
    /** Kind. */
    pub kind: MeasurementKind,
    /** Point. */
    pub point: MeasurementPoint,
    /** Freshness bound. */
    pub max_age_ms: u32,
}
/// Advisory measurement binding.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AdvisoryMeasurement {
    /** Kind. */
    pub kind: MeasurementKind,
    /** Point. */
    pub point: MeasurementPoint,
}
/// Offline budget limits.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct OfflineLimits {
    /** Cycle cooldown. */
    pub cooldown_ms: u32,
    /** Maximum volume per window. */
    pub max_volume_per_window_ml: f32,
    /** Rolling window duration. */
    pub window_ms: u32,
}
/// Required safety vetoes.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct OfflineSafety {
    /** Leak must explicitly be clear. */
    pub require_leak_clear: bool,
    /** Tank threshold. */
    pub require_tank_above_percent: f32,
    /** Pump must report healthy. */
    pub require_pump_healthy: bool,
}
/// Distinct policy validation failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PolicyError {
    DisabledActuatorPresent,
    MissingActuator,
    UnsupportedActuator,
    UnknownControlKind,
    InvalidHysteresis,
    ZeroDuration,
    DoseInvalid,
    DoseAboveHardLimit,
    ZeroDoses,
    CycleVolumeAboveWindow,
    WindowAboveHardLimit,
    TankThreshold,
}
impl OfflinePolicy {
    /** Validates against firmware hard limits; never clamps. */
    pub fn validate(&self) -> Result<(), PolicyError> {
        if !self.enabled {
            return Ok(());
        }
        let a = self.actuator.as_ref().ok_or(PolicyError::MissingActuator)?;
        if a.kind != ActuatorKind::IrrigationPump {
            return Err(PolicyError::UnsupportedActuator);
        }
        if !self.control_measurement.kind.control_eligible() {
            return Err(PolicyError::UnknownControlKind);
        }
        if self.control_measurement.resume_above <= self.control_measurement.trigger_below {
            return Err(PolicyError::InvalidHysteresis);
        }
        if self.control_measurement.confirm_duration_ms == 0
            || self.control_measurement.max_age_ms == 0
            || a.absorption_wait_ms == 0
            || self.limits.cooldown_ms == 0
            || self.limits.window_ms == 0
            || self.required_measurements.iter().any(|r| r.max_age_ms == 0)
        {
            return Err(PolicyError::ZeroDuration);
        }
        if !a.dose_ml.is_finite() || a.dose_ml <= 0.0 {
            return Err(PolicyError::DoseInvalid);
        }
        if a.dose_ml > FIRMWARE_MAX_ML_PER_RUN {
            return Err(PolicyError::DoseAboveHardLimit);
        }
        if a.max_doses_per_cycle == 0 {
            return Err(PolicyError::ZeroDoses);
        }
        if a.dose_ml * f32::from(a.max_doses_per_cycle) > self.limits.max_volume_per_window_ml {
            return Err(PolicyError::CycleVolumeAboveWindow);
        }
        if !self.limits.max_volume_per_window_ml.is_finite()
            || self.limits.max_volume_per_window_ml > FIRMWARE_MAX_DAILY_ML
        {
            return Err(PolicyError::WindowAboveHardLimit);
        }
        if !self.safety.require_tank_above_percent.is_finite()
            || !(0.0..=100.0).contains(&self.safety.require_tank_above_percent)
        {
            return Err(PolicyError::TankThreshold);
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn policy() -> OfflinePolicy {
        OfflinePolicy {
            plant_id: SensorId::parse("plant-01").unwrap(),
            policy_version: 1,
            enabled: true,
            actuator: Some(OfflineActuator {
                actuator_id: SensorId::parse("pump-0").unwrap(),
                kind: ActuatorKind::IrrigationPump,
                dose_ml: 35.,
                max_doses_per_cycle: 3,
                absorption_wait_ms: 1,
            }),
            control_measurement: ControlMeasurement {
                kind: MeasurementKind::SoilMoisture,
                point: MeasurementPoint::parse("default").unwrap(),
                trigger_below: 28.,
                resume_above: 34.,
                confirm_duration_ms: 1,
                max_age_ms: 1,
            },
            required_measurements: Vec::new(),
            advisory_measurements: Vec::new(),
            limits: OfflineLimits {
                cooldown_ms: 1,
                max_volume_per_window_ml: 300.,
                window_ms: 1,
            },
            safety: OfflineSafety {
                require_leak_clear: true,
                require_tank_above_percent: 15.,
                require_pump_healthy: true,
            },
        }
    }
    #[test]
    fn omitted_enabled_fails_closed() {
        let mut value = serde_json::to_value(policy()).unwrap();
        value.as_object_mut().unwrap().remove("enabled");
        let p: OfflinePolicy = serde_json::from_value(value).unwrap();
        assert!(!p.enabled);
    }
    #[test]
    fn each_bound_is_rejected_not_clamped() {
        let mut p = policy();
        p.control_measurement.resume_above = 28.;
        assert_eq!(p.validate(), Err(PolicyError::InvalidHysteresis));
        let mut p = policy();
        p.actuator.as_mut().unwrap().dose_ml = FIRMWARE_MAX_ML_PER_RUN + 0.1;
        assert_eq!(p.validate(), Err(PolicyError::DoseAboveHardLimit));
        let mut p = policy();
        p.limits.max_volume_per_window_ml = FIRMWARE_MAX_DAILY_ML + 0.1;
        assert_eq!(p.validate(), Err(PolicyError::WindowAboveHardLimit));
    }
}
