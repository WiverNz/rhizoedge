//! `PUT /devices/{id}/config` (M6-013), `http-api-boundaries.md` §2.3.
//!
//! Configuration is validated, versioned, and published **retained**, so a
//! device that boots days later receives the current desired state with no
//! liveness tracking anywhere (ADR-011 layer L3).
//!
//! A configuration that violates a firmware hard limit is **rejected with 422**,
//! never clamped and published: an operator who is told "200 ml exceeds the
//! device limit of 80 ml" learns the real limit while they are still paying
//! attention, and one whose value was silently reduced believes something false
//! about their system until an incident.

#![allow(missing_docs)]
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use rhizo_mqtt_contract::payload::{PowerConfig, PowerMode, PumpConfig, SensorConfig, TankConfig};
use serde::Deserialize;

use super::ApiState;
use super::support::{error, error_with, storage_error};
use crate::control::config::{self, DesiredConfig};

/// The configuration body. `deny_unknown_fields` is doing real work here: a
/// smuggled `max_ml_per_run` is a 422 rather than a field the device ignores.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigBody {
    pub telemetry_interval_seconds: u32,
    pub pump: PumpBody,
    pub tank: TankBody,
    #[serde(default)]
    pub sensors: SensorBody,
    #[serde(default)]
    pub power: Option<PowerBody>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PumpBody {
    pub ml_per_second: f64,
    pub enabled: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TankBody {
    pub min_percent: f64,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct SensorBody {
    #[serde(default)]
    pub soil: bool,
    #[serde(default)]
    pub weight: bool,
    #[serde(default)]
    pub tank: bool,
    #[serde(default)]
    pub leak: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PowerBody {
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub wake_interval_seconds: Option<u32>,
    #[serde(default)]
    pub sensor_warmup_ms: Option<u32>,
    #[serde(default)]
    pub awake_budget_seconds: Option<u32>,
}

impl ConfigBody {
    fn desired(&self) -> DesiredConfig {
        DesiredConfig {
            telemetry_interval_seconds: self.telemetry_interval_seconds,
            pump: PumpConfig {
                ml_per_second: self.pump.ml_per_second as f32,
                enabled: self.pump.enabled,
            },
            tank: TankConfig {
                min_percent: self.tank.min_percent as f32,
            },
            sensors: SensorConfig {
                soil: self.sensors.soil,
                weight: self.sensors.weight,
                tank: self.sensors.tank,
                leak: self.sensors.leak,
            },
            power: self.power.as_ref().map(|power| PowerConfig {
                // An unrecognised mode resolves to always-on, never to battery:
                // sleeping is the branch that makes a device unreachable, and
                // uncertainty must not take it (§5.7, SAFETY-012).
                mode: match power.mode.as_deref() {
                    Some("battery") => PowerMode::Battery,
                    Some("always_on") | None => PowerMode::AlwaysOn,
                    Some(_) => PowerMode::Unknown,
                },
                wake_interval_seconds: power.wake_interval_seconds,
                sensor_warmup_ms: power.sensor_warmup_ms,
                awake_budget_seconds: power.awake_budget_seconds,
            }),
        }
    }
}

pub async fn put(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    // The safety-limit check runs against the *raw* body, before typing, so a
    // field a future contract version might add cannot slip past by not being
    // in the struct.
    if let Err(rejection) = config::reject_safety_limits(&body) {
        return error_with(
            StatusCode::UNPROCESSABLE_ENTITY,
            rejection.code(),
            &rejection.message(),
            serde_json::json!({ "rule": rejection.code() }),
        );
    }
    let typed: ConfigBody = match serde_json::from_value(body) {
        Ok(value) => value,
        Err(e) => {
            return error(
                StatusCode::UNPROCESSABLE_ENTITY,
                "invalid_config",
                &e.to_string(),
            );
        }
    };
    let known: Option<String> =
        match sqlx::query_scalar("SELECT device_id FROM devices WHERE device_id=?")
            .bind(&id)
            .fetch_optional(state.db.pool())
            .await
        {
            Ok(value) => value,
            Err(_) => return storage_error(),
        };
    if known.is_none() {
        return error(StatusCode::NOT_FOUND, "device_not_found", "unknown device");
    }

    let version = match config::next_version(&state.db, &id).await {
        Ok(version) => version,
        Err(_) => return storage_error(),
    };
    let config = match config::validate(&typed.desired(), version) {
        Ok(config) => config,
        Err(rejection) => {
            return error_with(
                StatusCode::UNPROCESSABLE_ENTITY,
                rejection.code(),
                &rejection.message(),
                serde_json::json!({ "rule": rejection.code() }),
            );
        }
    };
    let now = state.clock.now();
    match config::publish(&state.commander, &id, &config, now).await {
        Ok(()) => (
            StatusCode::ACCEPTED,
            Json(serde_json::json!({
                "device_id": id,
                "config_version": config.config_version,
                "retained": true,
            })),
        )
            .into_response(),
        Err(crate::error::EdgeError::Mqtt(_)) => error(
            StatusCode::SERVICE_UNAVAILABLE,
            "publish_failed",
            "the configuration could not be published to the broker",
        ),
        Err(_) => storage_error(),
    }
}
