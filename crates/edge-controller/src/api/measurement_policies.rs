//! Per-measurement policy endpoints (M5-014).
//!
//! Thresholds belong to the **plant**, not to the sensor: two plants sharing one
//! room probe hold their own interpretations of it, and the API is keyed
//! `(plant, kind)` for exactly that reason.
#![allow(missing_docs)]
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use rhizo_domain::measurement_policy::MeasurementPolicyRules as _;
use rhizo_domain::plant::MeasurementPolicy;
use rhizo_storage::repo::binding as binding_repo;
use serde::Deserialize;

use super::ApiState;
use super::support::{error, error_with, storage_error};
use crate::plant;

fn policy_json(policy: &MeasurementPolicy) -> serde_json::Value {
    serde_json::json!({
        "kind": policy.kind.as_str(),
        "target_min": policy.target_min,
        "target_max": policy.target_max,
        "warning_low": policy.warning_low,
        "warning_high": policy.warning_high,
        "critical_low": policy.critical_low,
        "critical_high": policy.critical_high,
        "stale_after_ms": policy.stale_after_ms,
        "hysteresis": policy.hysteresis,
        "confirm_duration_ms": policy.confirm_duration_ms,
    })
}

pub async fn list(State(state): State<ApiState>, Path(id): Path<String>) -> Response {
    match binding_repo::measurement_policies(&state.db, &id).await {
        Ok(rows) => Json(serde_json::json!({
            "measurement_policies": rows
                .iter()
                .map(|row| policy_json(&plant::policy_from_row(row)))
                .collect::<Vec<_>>()
        }))
        .into_response(),
        Err(_) => storage_error(),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyBody {
    #[serde(default)]
    target_min: Option<f64>,
    #[serde(default)]
    target_max: Option<f64>,
    #[serde(default)]
    warning_low: Option<f64>,
    #[serde(default)]
    warning_high: Option<f64>,
    #[serde(default)]
    critical_low: Option<f64>,
    #[serde(default)]
    critical_high: Option<f64>,
    /// The only required field: everything else is genuinely optional and its
    /// absence never blocks evaluation.
    stale_after_ms: u32,
    #[serde(default)]
    hysteresis: Option<f64>,
    #[serde(default)]
    confirm_duration_ms: Option<u32>,
}

pub async fn put(
    State(state): State<ApiState>,
    Path((id, kind)): Path<(String, String)>,
    Json(body): Json<PolicyBody>,
) -> Response {
    match super::plants::exists(&state, &id).await {
        Ok(true) => {}
        Ok(false) => return error(StatusCode::NOT_FOUND, "plant_not_found", "unknown plant"),
        Err(_) => return storage_error(),
    }
    let policy = MeasurementPolicy {
        kind: plant::kind_from_str(&kind),
        target_min: body.target_min,
        target_max: body.target_max,
        warning_low: body.warning_low,
        warning_high: body.warning_high,
        critical_low: body.critical_low,
        critical_high: body.critical_high,
        stale_after_ms: body.stale_after_ms,
        hysteresis: body.hysteresis,
        confirm_duration_ms: body.confirm_duration_ms,
    };
    if let Err(e) = policy.validate() {
        return error_with(
            StatusCode::UNPROCESSABLE_ENTITY,
            e.code(),
            &e.to_string(),
            serde_json::json!({ "rule": e.code(), "kind": kind }),
        );
    }
    let now = state.clock.now().timestamp_millis();
    match binding_repo::upsert_measurement_policy(
        &state.db,
        &plant::policy_to_row(&id, &policy),
        now,
    )
    .await
    {
        Ok(()) => Json(policy_json(&policy)).into_response(),
        Err(_) => storage_error(),
    }
}

pub async fn delete(
    State(state): State<ApiState>,
    Path((id, kind)): Path<(String, String)>,
) -> Response {
    let now = state.clock.now().timestamp_millis();
    match binding_repo::delete_measurement_policy(&state.db, &id, &kind, now).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => error(StatusCode::NOT_FOUND, "policy_not_found", "unknown policy"),
        Err(_) => storage_error(),
    }
}

#[cfg(test)]
mod tests {
    use super::super::testsupport::TestApi;
    use axum::http::StatusCode;

    async fn ready() -> TestApi {
        let api = TestApi::start().await;
        api.with_device().await;
        api.plant("monstera-01").await;
        api.plant("fern-01").await;
        api
    }

    fn temperature() -> serde_json::Value {
        serde_json::json!({
            "target_min": 18.0, "target_max": 27.0,
            "warning_low": 12.0, "warning_high": 30.0,
            "critical_low": 5.0, "critical_high": 35.0,
            "stale_after_ms": 900_000,
            "hysteresis": 1.0,
            "confirm_duration_ms": 600_000,
        })
    }

    #[tokio::test]
    async fn policies_are_set_per_plant_and_per_kind() {
        let api = ready().await;
        let (status, written) = api
            .json(
                "PUT",
                "/api/v1/plants/monstera-01/measurement-policies/ambient_temperature",
                temperature(),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{written}");
        assert_eq!(written["kind"], "ambient_temperature");
        assert_eq!(written["hysteresis"], 1.0);

        let (_, listed) = api
            .get("/api/v1/plants/monstera-01/measurement-policies")
            .await;
        assert_eq!(listed["measurement_policies"].as_array().unwrap().len(), 1);

        let (status, _) = api
            .delete("/api/v1/plants/monstera-01/measurement-policies/ambient_temperature")
            .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (status, _) = api
            .delete("/api/v1/plants/monstera-01/measurement-policies/ambient_temperature")
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn each_validation_rule_rejects_with_its_own_code() {
        let api = ready().await;
        let cases = [
            (serde_json::json!({"target_min": 30.0}), "target_range"),
            (
                serde_json::json!({"stale_after_ms": 0}),
                "stale_after_not_positive",
            ),
            (
                serde_json::json!({"confirm_duration_ms": 0}),
                "non_positive_duration",
            ),
            (
                serde_json::json!({"hysteresis": -1.0}),
                "hysteresis_invalid",
            ),
            (
                serde_json::json!({"critical_low": 15.0}),
                "bands_not_nested",
            ),
            (
                serde_json::json!({"critical_high": 28.0}),
                "bands_not_nested",
            ),
        ];
        for (patch, code) in cases {
            let mut candidate = temperature();
            for (key, value) in patch.as_object().unwrap() {
                candidate[key] = value.clone();
            }
            let (status, refused) = api
                .json(
                    "PUT",
                    "/api/v1/plants/monstera-01/measurement-policies/ambient_temperature",
                    candidate,
                )
                .await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{refused}");
            assert_eq!(refused["error"]["code"], code, "{refused}");
        }
    }

    #[tokio::test]
    async fn a_missing_optional_field_is_genuinely_optional() {
        let api = ready().await;
        let (status, written) = api
            .json(
                "PUT",
                "/api/v1/plants/monstera-01/measurement-policies/illuminance",
                serde_json::json!({ "stale_after_ms": 900_000 }),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{written}");
        assert_eq!(written["target_min"], serde_json::Value::Null);
        assert_eq!(written["hysteresis"], serde_json::Value::Null);
        // And evaluation runs over it without complaint.
        api.bind_control("monstera-01").await;
        api.evaluate("monstera-01").await;
    }

    /// Thresholds belong to the plant, not the sensor: two plants sharing one
    /// probe hold their own interpretations of it.
    #[tokio::test]
    async fn two_plants_hold_different_thresholds_for_one_shared_sensor() {
        let api = ready().await;
        for plant in ["monstera-01", "fern-01"] {
            api.json(
                "PUT",
                &format!("/api/v1/plants/{plant}/bindings/sensors"),
                serde_json::json!({
                    "device_id": "plant-node-01",
                    "sensor_id": "soil-0",
                    "point": "default",
                    "kind": "soil_temperature",
                    "role": "advisory",
                }),
            )
            .await;
        }
        let mut warm = temperature();
        warm["critical_low"] = serde_json::json!(15.0);
        warm["warning_low"] = serde_json::json!(16.0);
        api.json(
            "PUT",
            "/api/v1/plants/monstera-01/measurement-policies/soil_temperature",
            warm,
        )
        .await;
        let mut hardy = temperature();
        hardy["critical_low"] = serde_json::json!(0.0);
        hardy["warning_low"] = serde_json::json!(4.0);
        api.json(
            "PUT",
            "/api/v1/plants/fern-01/measurement-policies/soil_temperature",
            hardy,
        )
        .await;

        let (_, a) = api
            .get("/api/v1/plants/monstera-01/measurement-policies")
            .await;
        let (_, b) = api.get("/api/v1/plants/fern-01/measurement-policies").await;
        assert_eq!(a["measurement_policies"][0]["critical_low"], 15.0);
        assert_eq!(b["measurement_policies"][0]["critical_low"], 0.0);
        assert_eq!(
            a["measurement_policies"][0]["kind"], b["measurement_policies"][0]["kind"],
            "the same kind, from the same probe, interpreted differently"
        );
    }

    #[tokio::test]
    async fn an_unknown_plant_or_kind_is_refused() {
        let api = ready().await;
        let (status, _) = api
            .json(
                "PUT",
                "/api/v1/plants/absent/measurement-policies/ambient_temperature",
                temperature(),
            )
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        let (status, refused) = api
            .json(
                "PUT",
                "/api/v1/plants/monstera-01/measurement-policies/future_kind",
                serde_json::json!({ "stale_after_ms": 900_000 }),
            )
            .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{refused}");
        assert_eq!(refused["error"]["code"], "unknown_kind");
    }
}
