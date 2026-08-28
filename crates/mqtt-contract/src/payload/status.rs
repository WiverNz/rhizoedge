//! Device status, capabilities, LWT, and configuration.
use super::{ActuatorKind, MeasurementKind, MeasurementPoint, SensorId};
use alloc::{boxed::Box, collections::BTreeMap, string::String, vec::Vec};
use serde::{Deserialize, Serialize};
/// Online state. Unknown states fail decoding rather than imply health.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceStatusValue {
    Online,
    Offline,
}
/// Device-declared power mode. Unknown values resolve conservatively to always-on.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PowerMode {
    /// The device is expected to remain reachable.
    #[default]
    AlwaysOn,
    /// The device intentionally sleeps between wake cycles.
    Battery,
    /// A future mode, treated as [`PowerMode::AlwaysOn`].
    #[serde(other)]
    Unknown,
}
impl PowerMode {
    /// Resolves an unknown declaration toward continued reachability.
    pub const fn effective(self) -> Self {
        match self {
            Self::Battery => Self::Battery,
            Self::AlwaysOn | Self::Unknown => Self::AlwaysOn,
        }
    }
}
/// Advisory power information reported with status.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PowerStatus {
    /// Actually applied mode.
    pub mode: PowerMode,
    /// Relative interval until the next wake. Edge receipt time remains authoritative.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wake_interval_seconds: Option<u32>,
    /// Device-clock diagnostic only; never a liveness or clock source.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_wake_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Reset/wake diagnostic.
    pub wake_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Supply voltage where measurable.
    pub battery_mv: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /// Elapsed awake time in this cycle.
    pub awake_ms: Option<u64>,
}
/// Connectivity as seen by the device.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectivityMode {
    Connected,
    Isolated,
}
/// Connectivity detail.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Connectivity {
    /** Mode. */
    pub mode: ConnectivityMode,
    /** Isolation duration. */
    pub isolated_ms: u64,
}
/// One declared sensor.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SensorCapability {
    /** Stable id. */
    pub sensor_id: SensorId,
    /** Default point. */
    pub point: MeasurementPoint,
    /** Produced kinds. */
    pub kinds: Vec<MeasurementKind>,
    /** Physically present. */
    pub present: bool,
    /** Health. */
    pub healthy: bool,
    /** Error count. */
    pub errors: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /** Calibration applicability/state. */
    pub calibrated: Option<bool>,
}
/// One declared actuator.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ActuatorCapability {
    /** Stable id. */
    pub actuator_id: SensorId,
    /** Strongly typed kind. */
    pub kind: ActuatorKind,
    /** Physically present. */
    pub present: bool,
    /** Health. */
    pub healthy: bool,
}
/// Device capability declaration. Empty actuators is valid.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DeviceCapabilities {
    #[serde(default)]
    /** Sensors. */
    pub sensors: Vec<SensorCapability>,
    #[serde(default)]
    /** Actuators. */
    pub actuators: Vec<ActuatorCapability>,
}
/// Compile-time limits reported read-only.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReportedLimits {
    /** Run seconds. */
    pub max_run_seconds: u32,
    /** Per-run ml. */
    pub max_ml_per_run: f32,
    /** Daily ml. */
    pub max_daily_ml: f32,
}
/// Retained device status data.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DeviceStatus {
    /** Monotonic boot generation persisted by the device. */
    pub boot_generation: u64,
    /** Online/offline. */
    pub status: DeviceStatusValue,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /** LWT/shutdown reason. */
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /** Firmware version. */
    pub firmware_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /** Protocol version. */
    pub protocol_version: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /** Config version. */
    pub applied_config_version: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /** Uptime. */
    pub uptime_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /** Free heap. */
    pub free_heap_bytes: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /** Wi-Fi RSSI. */
    pub rssi_dbm: Option<i16>,
    #[serde(default)]
    /** Active policies per plant. */
    pub applied_policy_versions: BTreeMap<String, u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /** Device connectivity. */
    pub connectivity: Option<Connectivity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /** Advisory power declaration. Absence and unknown modes mean always-on. */
    pub power: Option<Box<PowerStatus>>,
    #[serde(default)]
    /** Declared capabilities. */
    pub capabilities: DeviceCapabilities,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /** Read-only hard limits. */
    pub limits: Option<ReportedLimits>,
}
/// Semantic status validation failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StatusError {
    /// An announced sleep did not carry a valid relative wake interval.
    SleepWakeInterval,
}
/// Shortest relative wake interval a sleep announcement may declare.
pub const SLEEP_WAKE_INTERVAL_MIN_SECONDS: u32 = 60;
/// Longest relative wake interval a sleep announcement may declare.
pub const SLEEP_WAKE_INTERVAL_MAX_SECONDS: u32 = 86_400;
impl DeviceStatus {
    /// Whether this status *claims* to be an intentional sleep.
    ///
    /// The claim alone opens nothing. Only
    /// [`Self::announced_sleep_interval_seconds`] decides whether a wake window
    /// may be opened, which is why the two are separate.
    pub fn announces_sleep(&self) -> bool {
        self.status == DeviceStatusValue::Offline
            && matches!(self.reason.as_deref(), Some("sleeping"))
    }
    /// The one place the sleep-announcement rule lives.
    ///
    /// A wake window may be opened only by an offline status that says
    /// `sleeping`, carries a battery-mode `power` block, and declares a relative
    /// interval inside
    /// `SLEEP_WAKE_INTERVAL_MIN_SECONDS..=SLEEP_WAKE_INTERVAL_MAX_SECONDS`.
    /// Every consumer -- the decoder, the edge registry, and later the firmware
    /// -- asks this function instead of re-deriving the rule, for the same
    /// reason there is one `validate_water_command`: a second copy that drifted
    /// would change which absences count as expected without anything turning
    /// red.
    ///
    /// The interval is **relative**. No device timestamp is consulted here and
    /// `expected_wake_ms` is never read, so a device with a wrong clock cannot
    /// widen its own window (SAFETY-021).
    pub fn announced_sleep_interval_seconds(&self) -> Option<u32> {
        if !self.announces_sleep() {
            return None;
        }
        let power = self.power.as_deref()?;
        if power.mode.effective() != PowerMode::Battery {
            return None;
        }
        power.wake_interval_seconds.filter(|seconds| {
            (SLEEP_WAKE_INTERVAL_MIN_SECONDS..=SLEEP_WAKE_INTERVAL_MAX_SECONDS).contains(seconds)
        })
    }
    /// The power mode this status actually declares, resolved conservatively.
    ///
    /// `None` means the status carried no `power` block at all -- a v1 payload
    /// written before ADR-018, which declares nothing and must therefore change
    /// nothing. An unrecognised mode is *not* `None`: it is an explicit
    /// declaration that resolves to [`PowerMode::AlwaysOn`], because sleeping is
    /// the branch that makes a device unreachable (SAFETY-012).
    pub fn declared_power_mode(&self) -> Option<PowerMode> {
        self.power.as_deref().map(|power| power.mode.effective())
    }
    /// Validates the bounded sleep announcement without consulting device time.
    pub fn validate(&self) -> Result<(), StatusError> {
        if self.announces_sleep() && self.announced_sleep_interval_seconds().is_none() {
            return Err(StatusError::SleepWakeInterval);
        }
        Ok(())
    }
}
/// Pump tuning, never safety limits.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PumpConfig {
    /** Calibration factor. */
    pub ml_per_second: f32,
    /** Operational enable. */
    pub enabled: bool,
}
/// Tank threshold tuning.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct TankConfig {
    /** Minimum level. */
    pub min_percent: f32,
}
/// Legacy physical sensor enable switches.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SensorConfig {
    #[serde(default)]
    /** Soil. */
    pub soil: bool,
    #[serde(default)]
    /** Weight. */
    pub weight: bool,
    #[serde(default)]
    /** Tank. */
    pub tank: bool,
    #[serde(default)]
    /** Leak. */
    pub leak: bool,
}
/// Retained desired device configuration.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct DeviceConfig {
    /** Monotonic edge-owned version. */
    pub config_version: u32,
    /** Sampling interval. */
    pub telemetry_interval_seconds: u32,
    /** Pump tuning. */
    pub pump: PumpConfig,
    /** Tank tuning. */
    pub tank: TankConfig,
    #[serde(default)]
    /** Sensor switches. */
    pub sensors: SensorConfig,
}
/// Config validation failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfigError {
    TelemetryInterval,
    PumpRate,
    TankMinimum,
}
impl DeviceConfig {
    /** Rejects, never clamps, invalid configuration. */
    pub fn validate(&self) -> Result<(), ConfigError> {
        if !(10..=3600).contains(&self.telemetry_interval_seconds) {
            return Err(ConfigError::TelemetryInterval);
        }
        if !self.pump.ml_per_second.is_finite() || !(0.1..=100.0).contains(&self.pump.ml_per_second)
        {
            return Err(ConfigError::PumpRate);
        }
        if !self.tank.min_percent.is_finite() || !(0.0..=100.0).contains(&self.tank.min_percent) {
            return Err(ConfigError::TankMinimum);
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn config_rejects_each_range_and_ignores_smuggling() {
        let json = r#"{"config_version":1,"telemetry_interval_seconds":300,"pump":{"ml_per_second":8.2,"enabled":true},"tank":{"min_percent":15},"sensors":{},"ntp_server":"bad","max_ml_per_run":9999}"#;
        let c: DeviceConfig = serde_json::from_str(json).unwrap();
        assert!(c.validate().is_ok());
        let mut x = c;
        x.telemetry_interval_seconds = 9;
        assert_eq!(x.validate(), Err(ConfigError::TelemetryInterval));
        let mut x = c;
        x.pump.ml_per_second = 0.;
        assert_eq!(x.validate(), Err(ConfigError::PumpRate));
        let mut x = c;
        x.tank.min_percent = 101.;
        assert_eq!(x.validate(), Err(ConfigError::TankMinimum));
    }
    #[test]
    fn status_without_actuators_is_normal() {
        let s = DeviceStatus {
            boot_generation: 1,
            status: DeviceStatusValue::Online,
            reason: None,
            firmware_version: None,
            protocol_version: Some(1),
            applied_config_version: None,
            uptime_ms: None,
            free_heap_bytes: None,
            rssi_dbm: None,
            applied_policy_versions: BTreeMap::new(),
            connectivity: None,
            power: None,
            capabilities: DeviceCapabilities::default(),
            limits: None,
        };
        let value = serde_json::to_string(&s).unwrap();
        assert!(
            serde_json::from_str::<DeviceStatus>(&value)
                .unwrap()
                .capabilities
                .actuators
                .is_empty()
        );
    }
    #[test]
    fn power_mode_unknown_is_always_on_and_sleep_is_bounded() {
        let mode: PowerMode = serde_json::from_str("\"future_mode\"").unwrap();
        assert_eq!(mode.effective(), PowerMode::AlwaysOn);
        let power = PowerStatus {
            mode: PowerMode::Battery,
            wake_interval_seconds: Some(900),
            expected_wake_ms: Some(u64::MAX),
            wake_reason: Some("timer".into()),
            battery_mv: Some(3_280),
            awake_ms: Some(4_120),
        };
        let mut status = DeviceStatus {
            boot_generation: 1,
            status: DeviceStatusValue::Offline,
            reason: Some("sleeping".into()),
            firmware_version: None,
            protocol_version: Some(1),
            applied_config_version: None,
            uptime_ms: None,
            free_heap_bytes: None,
            rssi_dbm: None,
            applied_policy_versions: BTreeMap::new(),
            connectivity: None,
            power: Some(Box::new(power)),
            capabilities: DeviceCapabilities::default(),
            limits: None,
        };
        assert_eq!(status.validate(), Ok(()));
        assert_eq!(status.announced_sleep_interval_seconds(), Some(900));
        assert_eq!(status.declared_power_mode(), Some(PowerMode::Battery));
        status.power.as_mut().unwrap().wake_interval_seconds = Some(59);
        assert_eq!(status.validate(), Err(StatusError::SleepWakeInterval));
        assert_eq!(status.announced_sleep_interval_seconds(), None);
        status.power.as_mut().unwrap().wake_interval_seconds = Some(86_401);
        assert_eq!(status.validate(), Err(StatusError::SleepWakeInterval));
        status.power = None;
        assert_eq!(status.validate(), Err(StatusError::SleepWakeInterval));
        assert_eq!(status.declared_power_mode(), None);
    }
    /// Negative controls for the two directions the rule must never take:
    /// a sleep claim from a non-battery declaration, and an unknown mode that
    /// resolves to an explicit always-on rather than to "declared nothing".
    #[test]
    fn a_sleep_claim_without_a_battery_declaration_opens_no_window() {
        let mut status = DeviceStatus {
            boot_generation: 1,
            status: DeviceStatusValue::Offline,
            reason: Some("sleeping".into()),
            firmware_version: None,
            protocol_version: Some(1),
            applied_config_version: None,
            uptime_ms: None,
            free_heap_bytes: None,
            rssi_dbm: None,
            applied_policy_versions: BTreeMap::new(),
            connectivity: None,
            power: Some(Box::new(PowerStatus {
                mode: PowerMode::Unknown,
                wake_interval_seconds: Some(900),
                expected_wake_ms: None,
                wake_reason: None,
                battery_mv: None,
                awake_ms: None,
            })),
            capabilities: DeviceCapabilities::default(),
            limits: None,
        };
        assert!(status.announces_sleep());
        assert_eq!(status.announced_sleep_interval_seconds(), None);
        assert_eq!(status.declared_power_mode(), Some(PowerMode::AlwaysOn));
        assert_eq!(status.validate(), Err(StatusError::SleepWakeInterval));
        status.power.as_mut().unwrap().mode = PowerMode::AlwaysOn;
        assert_eq!(status.announced_sleep_interval_seconds(), None);
        status.reason = Some("connection_lost".into());
        assert!(!status.announces_sleep());
        assert_eq!(status.validate(), Ok(()));
    }
    /// A v1 status written before ADR-018 must still decode, declare nothing,
    /// and never look like a sleep announcement.
    #[test]
    fn a_pre_adr_018_status_declares_no_power_mode() {
        let json = r#"{"boot_generation":3,"status":"offline","reason":"connection_lost","protocol_version":1}"#;
        let status: DeviceStatus = serde_json::from_str(json).unwrap();
        assert_eq!(status.declared_power_mode(), None);
        assert_eq!(status.announced_sleep_interval_seconds(), None);
        assert!(!status.announces_sleep());
        assert_eq!(status.validate(), Ok(()));
        assert!(!serde_json::to_string(&status).unwrap().contains("power"));
    }
}
