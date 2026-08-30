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
///
/// M6 gives this type the numbers the irrigation machine runs on. They arrive
/// here rather than as a `profile` field on `IrrigationInputs` for the reason
/// ADR-016 gives: a [`crate::profile::PlantProfile`] is a **template** that
/// seeds a plant once, and a machine that read one at evaluation time would make
/// editing a template silently rewrite twelve plants' watering rules. What the
/// machine sees is the plant's own automation configuration, whatever seeded it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AutomationPolicy {
    /** Connected automation enabled. */
    pub connected_enabled: bool,
    /** Offline policy version when present. */
    pub offline_policy_version: Option<u32>,
    /** The fixed dose. Never a computed volume (F-060-23). */
    pub dose_ml: f32,
    /** Doses allowed inside one drying cycle. */
    pub max_doses_per_cycle: u16,
    /** The rolling 24-hour automatic ceiling (SAFETY-006). */
    pub max_daily_ml: f32,
    /** Moisture at or above which the plant is not dry. */
    pub target_min_vwc: f64,
    /** Continuous dryness before `Drying` becomes `DryConfirmed`. */
    pub dry_confirm: chrono::Duration,
    /** Minimum spacing between completed cycles. */
    pub cooldown: chrono::Duration,
    /** How long a dose is given to reach the probe. */
    pub absorption: chrono::Duration,
    /** Rise above the pre-dose reading that counts as a response (F-060-32). */
    pub recovery_delta_vwc: f64,
    /** Reservoir level at or below which watering is refused (SAFETY-004). */
    pub tank_min_percent: f64,
    /** The wire TTL stamped on every command (F-060-22). */
    pub command_ttl: chrono::Duration,
}

impl Default for AutomationPolicy {
    /// The built-in template's numbers, with automation **off**.
    ///
    /// `connected_enabled: false` is the SAFETY-012 default applied to
    /// provisioning: automatic watering is something an operator turns on.
    fn default() -> Self {
        Self::from_profile(
            &crate::profile::PlantProfile::default_seed(crate::ProfileId::from_uuid(
                uuid::Uuid::nil(),
            )),
            false,
            None,
        )
    }
}

impl AutomationPolicy {
    /// Seeds the automation configuration from a plant's template.
    #[must_use]
    pub fn from_profile(
        profile: &crate::profile::PlantProfile,
        connected_enabled: bool,
        offline_policy_version: Option<u32>,
    ) -> Self {
        Self {
            connected_enabled,
            offline_policy_version,
            dose_ml: profile.dose_ml,
            max_doses_per_cycle: profile.max_doses_per_cycle,
            max_daily_ml: profile.max_daily_ml,
            target_min_vwc: profile.target_min_vwc,
            dry_confirm: chrono::Duration::minutes(i64::from(profile.dry_confirm_minutes)),
            cooldown: chrono::Duration::milliseconds(
                (profile.cooldown_hours * 3_600_000.0).max(0.0) as i64,
            ),
            absorption: chrono::Duration::minutes(i64::from(profile.absorption_minutes)),
            recovery_delta_vwc: profile.recovery_delta_vwc,
            tank_min_percent: profile.tank_min_percent,
            command_ttl: chrono::Duration::seconds(i64::from(profile.command_ttl_seconds)),
        }
    }
}
