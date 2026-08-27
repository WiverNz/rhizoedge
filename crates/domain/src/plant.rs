//! Separate per-plant bindings and policy primitives from ADR-016.
use crate::{PlantId, ProfileId};
use rhizo_mqtt_contract::{
    DeviceId,
    payload::{ActuatorKind, MeasurementKind, MeasurementPoint, SensorId},
};
/// A cared-for plant, independently configured from physical devices.
#[derive(Clone, Debug, PartialEq)]
pub struct Plant {
    /** Identity. */
    pub plant_id: PlantId,
    /** Display name. */
    pub name: String,
    /** Optional originating template. */
    pub profile_id: Option<ProfileId>,
    /** Connected-mode opt-in. */
    pub auto_watering_enabled: bool,
}
/// Safety role of a bound measurement.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BindingRole {
    Control,
    Required,
    Advisory,
}
/// Maps one physical sensor output to a plant role.
#[derive(Clone, Debug, PartialEq)]
pub struct SensorBinding {
    /** Device. */
    pub device_id: DeviceId,
    /** Sensor capability. */
    pub sensor_id: SensorId,
    /** Point. */
    pub point: MeasurementPoint,
    /** Kind. */
    pub kind: MeasurementKind,
    /** Safety role. */
    pub role: BindingRole,
}
/// Optional actuation route.
#[derive(Clone, Debug, PartialEq)]
pub struct ActuatorBinding {
    /** Device. */
    pub device_id: DeviceId,
    /** Actuator capability. */
    pub actuator_id: SensorId,
    /** Kind. */
    pub kind: ActuatorKind,
}
/// Per-plant interpretation of one measurement kind.
#[derive(Clone, Debug, PartialEq)]
pub struct MeasurementPolicy {
    /** Kind. */
    pub kind: MeasurementKind,
    /** Desired minimum. */
    pub target_min: Option<f64>,
    /** Desired maximum. */
    pub target_max: Option<f64>,
    /** Warning band. */
    pub warning_low: Option<f64>,
    /** Warning band. */
    pub warning_high: Option<f64>,
    /** Critical band. */
    pub critical_low: Option<f64>,
    /** Critical band. */
    pub critical_high: Option<f64>,
    /** Freshness requirement ms. */
    pub stale_after_ms: u32,
    /** Hysteresis. */
    pub hysteresis: Option<f64>,
    /** Confirmation duration ms. */
    pub confirm_duration_ms: Option<u32>,
}
/// Alert threshold policy marker, expanded in M5.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AlertPolicy {
    /** Alerts enabled. */
    pub enabled: bool,
}
/// Connected/offline automation ownership kept distinct.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AutomationPolicy {
    /** Connected automation enabled. */
    pub connected_enabled: bool,
    /** Offline policy version when present. */
    pub offline_policy_version: Option<u32>,
}
