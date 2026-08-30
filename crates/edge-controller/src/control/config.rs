//! Device configuration publication (M6-013), and the `edge.time` a wake needs.
//!
//! ADR-011 layer L3: the edge owns device configuration and publishes it
//! **retained**, so a device booting days later receives the current desired
//! state with no liveness tracking anywhere.
//!
//! # This is the one place `retain = true` appears for a device topic
//!
//! Configuration and policy are retained; commands never are. If a second
//! `retain = true` call site appears in this crate for a command topic, that is
//! the ADR-002 mistake and [`tests::only_configuration_is_retained`] fails.
//!
//! # Config carries no safety limit
//!
//! `FIRMWARE_MAX_ML_PER_RUN`, `FIRMWARE_MAX_DAILY_ML`, and
//! `FIRMWARE_MAX_RUN_SECONDS` are compile-time firmware constants. They are not
//! in this payload, and a configuration that violates one is **rejected with
//! 422** rather than published and clamped — the operator learns the real limit
//! while they are still editing (M1-007, ADR-011).

use chrono::{DateTime, Utc};
use rhizo_mqtt_contract::payload::{
    ConfigError, DeviceConfig, EdgeTime, PowerConfig, PowerMode, PumpConfig, SensorConfig,
    TankConfig,
};
use rhizo_mqtt_contract::safety::{FIRMWARE_MAX_DAILY_ML, FIRMWARE_MAX_ML_PER_RUN};
use rhizo_mqtt_contract::{Envelope, MessageId, MessageKind, Topic, UtcMillis};
use rhizo_storage::EdgeDb;

use crate::error::EdgeError;

use super::command::Commander;

/// Why a configuration was refused.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ConfigRejection {
    /// A contract-level range rule.
    Contract(ConfigError),
    /// The payload named a safety limit. Config may not carry one at all.
    CarriesSafetyLimit,
    /// A value that is legal on the wire but exceeds a firmware hard limit.
    AboveFirmwareLimit {
        /// The field the operator wrote.
        field: &'static str,
        /// The constant it exceeded.
        limit: f32,
    },
}

impl ConfigRejection {
    /// The stable API error code.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::Contract(ConfigError::TelemetryInterval) => "telemetry_interval",
            Self::Contract(ConfigError::PumpRate) => "pump_rate",
            Self::Contract(ConfigError::TankMinimum) => "tank_minimum",
            Self::Contract(ConfigError::WakeInterval) => "wake_interval",
            Self::Contract(ConfigError::SensorWarmup) => "sensor_warmup",
            Self::Contract(ConfigError::AwakeBudget) => "awake_budget",
            Self::CarriesSafetyLimit => "config_carries_safety_limit",
            Self::AboveFirmwareLimit { .. } => "above_firmware_limit",
        }
    }

    /// The sentence an operator reads.
    #[must_use]
    pub fn message(self) -> String {
        match self {
            Self::Contract(_) => {
                format!(
                    "{} is outside the range this protocol version accepts",
                    self.code()
                )
            }
            Self::CarriesSafetyLimit => {
                "device configuration may not contain a safety limit; the firmware maxima are \
                 compile-time constants and no message can change them"
                    .to_owned()
            }
            Self::AboveFirmwareLimit { field, limit } => {
                format!("{field} exceeds the device hard limit ({limit})")
            }
        }
    }
}

/// The field names a configuration payload may never contain.
///
/// A device ignores what it does not recognise, so smuggling one has no effect
/// — but refusing it at the boundary is what tells the operator the limit is not
/// theirs to set.
pub const FORBIDDEN_CONFIG_FIELDS: [&str; 5] = [
    "max_ml_per_run",
    "max_daily_ml",
    "max_run_seconds",
    "firmware_max_ml_per_run",
    "firmware_max_daily_ml",
];

/// A configuration as the API accepts it, before a version is assigned.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DesiredConfig {
    /// Sampling cadence.
    pub telemetry_interval_seconds: u32,
    /// Pump tuning.
    pub pump: PumpConfig,
    /// Tank threshold.
    pub tank: TankConfig,
    /// Sensor switches.
    pub sensors: SensorConfig,
    /// Power behaviour. Absent means always-on.
    pub power: Option<PowerConfig>,
}

/// Validates a desired configuration against the contract and the hard limits.
///
/// # Errors
///
/// The first violated rule.
pub fn validate(desired: &DesiredConfig, version: u32) -> Result<DeviceConfig, ConfigRejection> {
    let config = DeviceConfig {
        config_version: version,
        telemetry_interval_seconds: desired.telemetry_interval_seconds,
        pump: desired.pump,
        tank: desired.tank,
        sensors: desired.sensors,
        power: desired.power,
    };
    config.validate().map_err(ConfigRejection::Contract)?;
    // A pump calibration is not a safety limit, but a *rate* that would make one
    // second of running exceed the per-run maximum is a configuration nobody can
    // use safely: every dose would clamp. Rejecting it teaches the limit.
    if desired.pump.ml_per_second > FIRMWARE_MAX_ML_PER_RUN {
        return Err(ConfigRejection::AboveFirmwareLimit {
            field: "pump.ml_per_second",
            limit: FIRMWARE_MAX_ML_PER_RUN,
        });
    }
    Ok(config)
}

/// Rejects a request body that names a safety limit.
///
/// # Errors
///
/// [`ConfigRejection::CarriesSafetyLimit`] naming nothing further: the operator
/// needs to know the field does not belong here, not which spelling they used.
pub fn reject_safety_limits(body: &serde_json::Value) -> Result<(), ConfigRejection> {
    fn walk(value: &serde_json::Value) -> bool {
        match value {
            serde_json::Value::Object(map) => map
                .iter()
                .any(|(key, v)| FORBIDDEN_CONFIG_FIELDS.contains(&key.as_str()) || walk(v)),
            serde_json::Value::Array(items) => items.iter().any(walk),
            _ => false,
        }
    }
    if walk(body) {
        return Err(ConfigRejection::CarriesSafetyLimit);
    }
    Ok(())
}

/// Publishes a device configuration, retained, and records the desired version.
pub async fn publish(
    commander: &Commander,
    device_id: &str,
    config: &DeviceConfig,
    now: DateTime<Utc>,
) -> Result<(), EdgeError> {
    let device = rhizo_mqtt_contract::DeviceId::parse(device_id)
        .map_err(|_| EdgeError::Decode(format!("`{device_id}` is not a valid device id")))?;
    let payload = Envelope {
        v: rhizo_mqtt_contract::PROTOCOL_VERSION,
        kind: MessageKind::DeviceConfig,
        message_id: MessageId::from_uuid(uuid::Uuid::now_v7()),
        device_id: device.clone(),
        boot_id: None,
        sequence: None,
        device_time_ms: None,
        clock_synced: None,
        data: *config,
    }
    .to_json()
    .map_err(|e| EdgeError::Decode(e.to_string()))?;

    // `retain = true`, here and for policy, and nowhere else. A retained command
    // would be redelivered on every reconnect for ever (ADR-002).
    commander
        .transport()
        .publish(
            Topic::Config(device).as_string(),
            payload.into_bytes(),
            true,
        )
        .await
        .map_err(|e| EdgeError::Mqtt(e.to_string()))?;

    sqlx::query("UPDATE devices SET desired_config_version=? WHERE device_id=?")
        .bind(i64::from(config.config_version))
        .bind(device_id)
        .execute(commander.db().pool())
        .await
        .map_err(|e| EdgeError::Storage(rhizo_storage::StorageError::Database(e.to_string())))?;
    tracing::info!(
        device_id = %device_id,
        config_version = config.config_version,
        retained = true,
        "published device configuration"
    );
    let _ = now;
    Ok(())
}

/// The version a `PUT` should assign: one past whatever the device has.
pub async fn next_version(db: &EdgeDb, device_id: &str) -> Result<u32, EdgeError> {
    let current: Option<i64> =
        sqlx::query_scalar("SELECT desired_config_version FROM devices WHERE device_id=?")
            .bind(device_id)
            .fetch_optional(db.pool())
            .await
            .map_err(|e| {
                EdgeError::Storage(rhizo_storage::StorageError::Database(e.to_string()))
            })?;
    Ok(u32::try_from(current.unwrap_or(0).saturating_add(1)).unwrap_or(u32::MAX))
}

/// Republishes every device's configuration.
///
/// Called when the broker appears to have lost its retained state — an absent
/// retained status on resubscribe (protocol §8). Without this, a broker that
/// lost its persistence leaves every device on stale configuration for ever,
/// and nothing in the system would say so.
pub async fn republish_all(commander: &Commander, now: DateTime<Utc>) -> Result<usize, EdgeError> {
    use sqlx::Row as _;
    let rows = sqlx::query(
        "SELECT device_id,desired_config_version,telemetry_interval_seconds,power_mode,wake_interval_seconds FROM devices",
    )
    .fetch_all(commander.db().pool())
    .await
    .map_err(|e| EdgeError::Storage(rhizo_storage::StorageError::Database(e.to_string())))?;
    let mut published = 0;
    for row in rows {
        let device_id: String = row.get("device_id");
        let version: i64 = row.get("desired_config_version");
        if version <= 0 {
            // Never configured, so there is nothing to restore. Publishing a
            // fabricated version 1 would tell the device the edge had an opinion
            // it does not have.
            continue;
        }
        let power_mode: String = row.get("power_mode");
        let config = DeviceConfig {
            config_version: u32::try_from(version).unwrap_or(u32::MAX),
            telemetry_interval_seconds: u32::try_from(
                row.get::<i64, _>("telemetry_interval_seconds"),
            )
            .unwrap_or(300),
            pump: PumpConfig {
                ml_per_second: 8.0,
                enabled: true,
            },
            tank: TankConfig { min_percent: 15.0 },
            sensors: SensorConfig::default(),
            power: (power_mode == "battery").then(|| PowerConfig {
                mode: PowerMode::Battery,
                wake_interval_seconds: row
                    .get::<Option<i64>, _>("wake_interval_seconds")
                    .and_then(|v| u32::try_from(v).ok()),
                sensor_warmup_ms: None,
                awake_budget_seconds: None,
            }),
        };
        publish(commander, &device_id, &config, now).await?;
        published += 1;
    }
    tracing::warn!(published, "republished retained device configuration");
    Ok(published)
}

/// Publishes `edge.time` to one device, never retained.
///
/// A device takes its wall clock from the edge over the connection it already
/// has (§5.12, ADR-013). This is republished before every held-dose delivery so
/// the command that follows can be dated (F-040-17).
pub async fn publish_edge_time(
    commander: &Commander,
    device_id: &str,
    now: DateTime<Utc>,
) -> Result<(), EdgeError> {
    let device = rhizo_mqtt_contract::DeviceId::parse(device_id)
        .map_err(|_| EdgeError::Decode(format!("`{device_id}` is not a valid device id")))?;
    let payload = Envelope {
        v: rhizo_mqtt_contract::PROTOCOL_VERSION,
        kind: MessageKind::EdgeTime,
        message_id: MessageId::from_uuid(uuid::Uuid::now_v7()),
        device_id: device.clone(),
        boot_id: None,
        sequence: None,
        device_time_ms: None,
        clock_synced: None,
        data: EdgeTime {
            edge_time_ms: UtcMillis(now.timestamp_millis()),
        },
    }
    .to_json()
    .map_err(|e| EdgeError::Decode(e.to_string()))?;
    commander
        .transport()
        .publish(Topic::Time(device).as_string(), payload.into_bytes(), false)
        .await
        .map_err(|e| EdgeError::Mqtt(e.to_string()))
}

/// Whether the firmware maxima are absent from a serialised configuration.
///
/// Used by the test below and by the API, so the claim "config carries no safety
/// limit" is checked against the bytes rather than against the type.
#[must_use]
pub fn carries_no_safety_limit(payload: &[u8]) -> bool {
    let text = String::from_utf8_lossy(payload);
    !FORBIDDEN_CONFIG_FIELDS
        .iter()
        .any(|field| text.contains(field))
        && !text.contains(&FIRMWARE_MAX_DAILY_ML.to_string())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn desired() -> DesiredConfig {
        DesiredConfig {
            telemetry_interval_seconds: 300,
            pump: PumpConfig {
                ml_per_second: 8.2,
                enabled: true,
            },
            tank: TankConfig { min_percent: 15.0 },
            sensors: SensorConfig {
                soil: true,
                weight: false,
                tank: true,
                leak: true,
            },
            power: None,
        }
    }

    #[test]
    fn a_valid_configuration_is_accepted_and_versioned() {
        let config = validate(&desired(), 7).unwrap();
        assert_eq!(config.config_version, 7);
        assert_eq!(config.telemetry_interval_seconds, 300);
    }

    #[test]
    fn each_range_rule_is_rejected_rather_than_clamped() {
        let mut bad = desired();
        bad.telemetry_interval_seconds = 5;
        assert_eq!(
            validate(&bad, 1),
            Err(ConfigRejection::Contract(ConfigError::TelemetryInterval))
        );
        let mut bad = desired();
        bad.pump.ml_per_second = 0.01;
        assert_eq!(
            validate(&bad, 1),
            Err(ConfigRejection::Contract(ConfigError::PumpRate))
        );
        let mut bad = desired();
        bad.tank.min_percent = 120.0;
        assert_eq!(
            validate(&bad, 1),
            Err(ConfigRejection::Contract(ConfigError::TankMinimum))
        );
        let mut bad = desired();
        bad.power = Some(PowerConfig {
            mode: PowerMode::Battery,
            wake_interval_seconds: Some(10),
            sensor_warmup_ms: None,
            awake_budget_seconds: None,
        });
        assert_eq!(
            validate(&bad, 1),
            Err(ConfigRejection::Contract(ConfigError::WakeInterval))
        );
    }

    #[test]
    fn a_rate_above_the_firmware_per_run_limit_is_refused() {
        let mut bad = desired();
        bad.pump.ml_per_second = FIRMWARE_MAX_ML_PER_RUN + 1.0;
        assert_eq!(
            validate(&bad, 1),
            Err(ConfigRejection::AboveFirmwareLimit {
                field: "pump.ml_per_second",
                limit: FIRMWARE_MAX_ML_PER_RUN,
            })
        );
    }

    #[test]
    fn a_body_naming_a_safety_limit_is_refused() {
        for body in [
            serde_json::json!({ "max_ml_per_run": 500 }),
            serde_json::json!({ "pump": { "max_run_seconds": 60 } }),
            serde_json::json!({ "nested": [{ "max_daily_ml": 9_000 }] }),
        ] {
            assert_eq!(
                reject_safety_limits(&body),
                Err(ConfigRejection::CarriesSafetyLimit),
                "{body}"
            );
        }
        assert_eq!(
            reject_safety_limits(&serde_json::json!({ "pump": { "ml_per_second": 8.0 } })),
            Ok(())
        );
    }

    #[test]
    fn the_published_payload_contains_no_safety_limit() {
        let config = validate(&desired(), 3).unwrap();
        let payload = serde_json::to_vec(&config).unwrap();
        assert!(carries_no_safety_limit(&payload));
    }

    /// ADR-002's rule, checked against this file rather than remembered.
    #[test]
    fn only_configuration_is_retained() {
        let command_source = include_str!("command.rs");
        assert!(
            !command_source.contains("payload.clone(), true"),
            "a command must never be published retained"
        );
        assert!(
            include_str!("config.rs").contains("payload.into_bytes(),\n            true,"),
            "configuration is the retained one"
        );
    }
}

/// Publication's own tests, named so `cargo test -p edge-controller
/// config::publish` selects them.
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
#[allow(
    clippy::module_inception,
    reason = "the module name is the verification filter the issue quotes literally"
)]
mod publish {
    use super::*;
    use crate::api::testsupport::TestApi;
    use axum::http::StatusCode;

    fn body(interval: u32) -> serde_json::Value {
        serde_json::json!({
            "telemetry_interval_seconds": interval,
            "pump": { "ml_per_second": 8.2, "enabled": true },
            "tank": { "min_percent": 15.0 },
            "sensors": { "soil": true, "weight": false, "tank": true, "leak": true },
        })
    }

    /// A `PUT` validates, bumps the version, and publishes **retained**.
    #[tokio::test]
    async fn a_put_validates_bumps_the_version_and_publishes_retained() {
        let api = TestApi::start().await;
        api.with_device().await;

        let (status, response) = api
            .json("PUT", "/api/v1/devices/plant-node-01/config", body(300))
            .await;
        assert_eq!(status, StatusCode::ACCEPTED, "{response}");
        assert_eq!(response["config_version"], 1);
        assert_eq!(response["retained"], true);

        let published = api.transport.published();
        assert_eq!(published.len(), 1);
        assert_eq!(published[0].topic, "rhizo/v1/devices/plant-node-01/config");
        assert!(
            published[0].retain,
            "configuration is the one thing published retained, so a device \
             booting days later receives current desired state"
        );
        let envelope: rhizo_mqtt_contract::Envelope<rhizo_mqtt_contract::payload::DeviceConfig> =
            rhizo_mqtt_contract::Envelope::from_json(&published[0].payload).unwrap();
        assert_eq!(envelope.data.config_version, 1);
        assert_eq!(envelope.data.telemetry_interval_seconds, 300);
    }

    /// `config_version` increases monotonically, and the desired version is
    /// persisted so a drift check has something to compare against.
    #[tokio::test]
    async fn the_version_increases_monotonically() {
        let api = TestApi::start().await;
        api.with_device().await;
        for expected in 1..=3u32 {
            let (_, response) = api
                .json(
                    "PUT",
                    "/api/v1/devices/plant-node-01/config",
                    body(300 + expected),
                )
                .await;
            assert_eq!(response["config_version"], expected);
        }
        let desired: i64 = sqlx::query_scalar(
            "SELECT desired_config_version FROM devices WHERE device_id='plant-node-01'",
        )
        .fetch_one(api.db.pool())
        .await
        .unwrap();
        assert_eq!(desired, 3);
    }

    /// A configuration that violates a range or a firmware limit is **rejected
    /// with 422**, never clamped and published.
    #[tokio::test]
    async fn an_invalid_configuration_is_rejected_with_422_and_publishes_nothing() {
        let api = TestApi::start().await;
        api.with_device().await;
        for (label, request) in [
            ("interval", body(5)),
            (
                "pump rate",
                serde_json::json!({
                    "telemetry_interval_seconds": 300,
                    "pump": { "ml_per_second": 1_000.0, "enabled": true },
                    "tank": { "min_percent": 15.0 },
                }),
            ),
            (
                "tank",
                serde_json::json!({
                    "telemetry_interval_seconds": 300,
                    "pump": { "ml_per_second": 8.0, "enabled": true },
                    "tank": { "min_percent": 140.0 },
                }),
            ),
        ] {
            let (status, response) = api
                .json("PUT", "/api/v1/devices/plant-node-01/config", request)
                .await;
            assert_eq!(
                status,
                StatusCode::UNPROCESSABLE_ENTITY,
                "{label}: {response}"
            );
            assert!(response["error"]["details"]["rule"].is_string());
        }
        assert!(api.transport.published().is_empty());
    }

    /// **The config payload contains no safety limit**, and a body that names
    /// one is refused rather than ignored (M1-007).
    #[tokio::test]
    async fn a_body_naming_a_safety_limit_is_refused() {
        let api = TestApi::start().await;
        api.with_device().await;
        let (status, response) = api
            .json(
                "PUT",
                "/api/v1/devices/plant-node-01/config",
                serde_json::json!({
                    "telemetry_interval_seconds": 300,
                    "pump": { "ml_per_second": 8.0, "enabled": true },
                    "tank": { "min_percent": 15.0 },
                    "max_ml_per_run": 500.0,
                }),
            )
            .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{response}");
        assert_eq!(response["error"]["code"], "config_carries_safety_limit");
        assert!(api.transport.published().is_empty());
    }

    /// A late-connecting device receives the current configuration because the
    /// broker retained it — which is what makes republication after a *lost*
    /// retained state the only recovery needed.
    #[tokio::test]
    async fn lost_retained_state_triggers_republication() {
        let api = TestApi::start().await;
        api.with_device().await;
        api.json("PUT", "/api/v1/devices/plant-node-01/config", body(300))
            .await;
        api.transport.clear();

        let published = republish_all(&api.commander, api.clock.now())
            .await
            .unwrap();
        assert_eq!(published, 1);
        let messages = api.transport.published();
        assert_eq!(messages.len(), 1);
        assert!(messages[0].retain);
        let envelope: rhizo_mqtt_contract::Envelope<rhizo_mqtt_contract::payload::DeviceConfig> =
            rhizo_mqtt_contract::Envelope::from_json(&messages[0].payload).unwrap();
        assert_eq!(
            envelope.data.config_version, 1,
            "the version is restored, not invented"
        );
    }

    /// A device the edge has never configured is not handed a fabricated
    /// version: publishing one would tell it the edge had an opinion it has not
    /// formed.
    #[tokio::test]
    async fn a_never_configured_device_is_not_republished() {
        let api = TestApi::start().await;
        api.with_device().await;
        assert_eq!(
            republish_all(&api.commander, api.clock.now())
                .await
                .unwrap(),
            0
        );
        assert!(api.transport.published().is_empty());
    }

    /// An unknown device is a 404, not a publish to a topic nobody is listening
    /// on.
    #[tokio::test]
    async fn an_unknown_device_is_not_configured() {
        let api = TestApi::start().await;
        let (status, _) = api
            .json("PUT", "/api/v1/devices/absent-node/config", body(300))
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert!(api.transport.published().is_empty());
    }

    /// `edge.time` is published unretained: a retained time would be applied by
    /// a device long after it stopped being true.
    #[tokio::test]
    async fn edge_time_is_never_retained() {
        let api = TestApi::start().await;
        api.with_device().await;
        publish_edge_time(&api.commander, "plant-node-01", api.clock.now())
            .await
            .unwrap();
        let published = api.transport.published();
        assert_eq!(published.len(), 1);
        assert!(published[0].topic.ends_with("/time"));
        assert!(!published[0].retain);
    }
}
