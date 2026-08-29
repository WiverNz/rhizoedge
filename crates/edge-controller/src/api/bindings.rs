//! Binding endpoints (M5-013).
//!
//! A binding may only name a capability the device actually declared (M4-011).
//! The role rules, the single-`control` rule, and the leak/tank rule are all
//! `rhizo-domain`'s; this module supplies their inputs and renders the refusal.
#![allow(missing_docs)]
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use rhizo_domain::binding::{
    BindingError, validate_actuator_binding, validate_sensor_binding,
    validate_sensor_binding_removal,
};
use rhizo_domain::plant::{ActuatorBinding, SensorBinding};
use rhizo_mqtt_contract::DeviceId;
use rhizo_mqtt_contract::payload::{ActuatorKind, MeasurementPoint, SensorId};
use rhizo_storage::repo::binding as binding_repo;
use serde::Deserialize;

use super::ApiState;
use super::support::{error, error_with, storage_error};
use crate::plant;

fn refused(e: &BindingError) -> Response {
    error_with(
        StatusCode::UNPROCESSABLE_ENTITY,
        e.code(),
        &e.to_string(),
        serde_json::json!({ "rule": e.code() }),
    )
}

fn binding_json(row: &binding_repo::SensorBindingRow) -> serde_json::Value {
    serde_json::json!({
        "binding_id": row.binding_id,
        "device_id": row.device_id,
        "sensor_id": row.sensor_id,
        "point": row.point,
        "kind": row.kind,
        "role": row.role,
    })
}

pub async fn list_sensors(State(state): State<ApiState>, Path(id): Path<String>) -> Response {
    match binding_repo::sensor_bindings(&state.db, &id).await {
        Ok(rows) => Json(serde_json::json!({
            "sensor_bindings": rows.iter().map(binding_json).collect::<Vec<_>>()
        }))
        .into_response(),
        Err(_) => storage_error(),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SensorBindingBody {
    /// Supplied on an edit; generated on a create.
    #[serde(default)]
    binding_id: Option<String>,
    device_id: String,
    sensor_id: String,
    #[serde(default = "default_point")]
    point: String,
    kind: String,
    role: String,
}

fn default_point() -> String {
    "default".to_owned()
}

pub async fn put_sensor(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Json(body): Json<SensorBindingBody>,
) -> Response {
    match super::plants::exists(&state, &id).await {
        Ok(true) => {}
        Ok(false) => return error(StatusCode::NOT_FOUND, "plant_not_found", "unknown plant"),
        Err(_) => return storage_error(),
    }
    let (Ok(device_id), Ok(sensor_id), Ok(point)) = (
        DeviceId::parse(&body.device_id),
        SensorId::parse(&body.sensor_id),
        MeasurementPoint::parse(&body.point),
    ) else {
        return error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_identifier",
            "device_id, sensor_id, and point must satisfy the protocol grammar",
        );
    };
    if !matches!(body.role.as_str(), "control" | "required" | "advisory") {
        return error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_role",
            "role must be one of control, required, advisory",
        );
    }
    let binding = SensorBinding {
        device_id,
        sensor_id,
        point,
        kind: plant::kind_from_str(&body.kind),
        role: plant::role_from_str(&body.role),
    };
    let binding_id = body
        .binding_id
        .clone()
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    let (Ok(declared), Ok(existing), Ok(actuator)) = (
        plant::declared_capabilities(&state.db).await,
        binding_repo::sensor_bindings(&state.db, &id).await,
        binding_repo::actuator_binding(&state.db, &id).await,
    ) else {
        return storage_error();
    };
    // The binding being edited is excluded, so an edit re-validates cleanly
    // rather than colliding with itself.
    let others: Vec<SensorBinding> = existing
        .iter()
        .filter(|row| row.binding_id != binding_id)
        .filter_map(|row| {
            Some(SensorBinding {
                device_id: DeviceId::parse(&row.device_id).ok()?,
                sensor_id: SensorId::parse(&row.sensor_id).ok()?,
                point: MeasurementPoint::parse(&row.point).ok()?,
                kind: plant::kind_from_str(&row.kind),
                role: plant::role_from_str(&row.role),
            })
        })
        .collect();
    if let Err(e) = validate_sensor_binding(&binding, &declared, &others, actuator.is_some()) {
        return refused(&e);
    }
    let now = state.clock.now().timestamp_millis();
    let row = binding_repo::SensorBindingRow {
        binding_id: binding_id.clone(),
        plant_id: id.clone(),
        device_id: binding.device_id.to_string(),
        sensor_id: binding.sensor_id.as_str().to_owned(),
        point: binding.point.as_str().to_owned(),
        kind: binding.kind.as_str().to_owned(),
        role: plant::role_name(binding.role).to_owned(),
        created_at: now,
    };
    match binding_repo::upsert_sensor_binding(&state.db, &row).await {
        Ok(()) => (StatusCode::CREATED, Json(binding_json(&row))).into_response(),
        Err(rhizo_storage::StorageError::Constraint(message)) => error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "duplicate_control_binding",
            &message,
        ),
        Err(_) => storage_error(),
    }
}

pub async fn delete_sensor(
    State(state): State<ApiState>,
    Path((id, binding_id)): Path<(String, String)>,
) -> Response {
    let (Ok(Some(plant)), Ok(existing)) = (
        rhizo_storage::repo::plant::get(&state.db, &id).await,
        binding_repo::sensor_bindings(&state.db, &id).await,
    ) else {
        return error(StatusCode::NOT_FOUND, "plant_not_found", "unknown plant");
    };
    let Some(target) = existing.iter().find(|row| row.binding_id == binding_id) else {
        return error(
            StatusCode::NOT_FOUND,
            "binding_not_found",
            "unknown binding",
        );
    };
    let decoded = |row: &binding_repo::SensorBindingRow| -> Option<SensorBinding> {
        Some(SensorBinding {
            device_id: DeviceId::parse(&row.device_id).ok()?,
            sensor_id: SensorId::parse(&row.sensor_id).ok()?,
            point: MeasurementPoint::parse(&row.point).ok()?,
            kind: plant::kind_from_str(&row.kind),
            role: plant::role_from_str(&row.role),
        })
    };
    let Some(removed) = decoded(target) else {
        return storage_error();
    };
    let remaining: Vec<SensorBinding> = existing
        .iter()
        .filter(|row| row.binding_id != binding_id)
        .filter_map(decoded)
        .collect();
    if let Err(e) =
        validate_sensor_binding_removal(&removed, &remaining, plant.auto_watering_enabled)
    {
        return refused(&e);
    }
    match binding_repo::delete_sensor_binding(&state.db, &binding_id).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => error(
            StatusCode::NOT_FOUND,
            "binding_not_found",
            "unknown binding",
        ),
        Err(_) => storage_error(),
    }
}

pub async fn get_actuator(State(state): State<ApiState>, Path(id): Path<String>) -> Response {
    match binding_repo::actuator_binding(&state.db, &id).await {
        // A plant with no actuator is a normal monitoring plant, not an error:
        // 200 with `null`, so the UI renders no watering controls at all rather
        // than disabled ones (SAFETY-018).
        Ok(None) => Json(serde_json::json!({ "actuator_binding": null })).into_response(),
        Ok(Some(row)) => Json(serde_json::json!({ "actuator_binding": {
            "device_id": row.device_id,
            "actuator_id": row.actuator_id,
            "kind": row.kind,
        }}))
        .into_response(),
        Err(_) => storage_error(),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActuatorBindingBody {
    device_id: String,
    actuator_id: String,
    #[serde(default = "irrigation_pump")]
    kind: String,
}

fn irrigation_pump() -> String {
    "irrigation_pump".to_owned()
}

pub async fn put_actuator(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Json(body): Json<ActuatorBindingBody>,
) -> Response {
    match super::plants::exists(&state, &id).await {
        Ok(true) => {}
        Ok(false) => return error(StatusCode::NOT_FOUND, "plant_not_found", "unknown plant"),
        Err(_) => return storage_error(),
    }
    let (Ok(device_id), Ok(actuator_id)) = (
        DeviceId::parse(&body.device_id),
        SensorId::parse(&body.actuator_id),
    ) else {
        return error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_identifier",
            "device_id and actuator_id must satisfy the protocol grammar",
        );
    };
    let kind: ActuatorKind = serde_json::from_value(serde_json::Value::String(body.kind.clone()))
        .unwrap_or(ActuatorKind::Unknown);
    let binding = ActuatorBinding {
        device_id,
        actuator_id,
        kind,
    };
    let Ok(declared) = plant::declared_capabilities(&state.db).await else {
        return storage_error();
    };
    if let Err(e) = validate_actuator_binding(&binding, &declared) {
        return refused(&e);
    }
    let existing = match plant::load(&state.db, &id).await {
        Ok(Some(loaded)) => loaded,
        Ok(None) => return error(StatusCode::NOT_FOUND, "plant_not_found", "unknown plant"),
        Err(_) => return storage_error(),
    };
    if let Some(bound) = existing.sensors.iter().find(|bound| {
        rhizo_domain::binding::is_safety_kind(&bound.binding.kind)
            && bound.binding.role != rhizo_domain::plant::BindingRole::Required
    }) {
        return refused(&BindingError::SafetyRoleMustBeRequired {
            kind: bound.binding.kind.as_str().to_owned(),
        });
    }
    let now = state.clock.now().timestamp_millis();
    let row = binding_repo::ActuatorBindingRow {
        plant_id: id,
        device_id: binding.device_id.to_string(),
        actuator_id: binding.actuator_id.as_str().to_owned(),
        kind: body.kind,
        created_at: now,
    };
    match binding_repo::upsert_actuator_binding(&state.db, &row).await {
        Ok(()) => (
            StatusCode::CREATED,
            Json(serde_json::json!({ "actuator_binding": {
                "device_id": row.device_id,
                "actuator_id": row.actuator_id,
                "kind": row.kind,
            }})),
        )
            .into_response(),
        Err(_) => storage_error(),
    }
}

pub async fn delete_actuator(State(state): State<ApiState>, Path(id): Path<String>) -> Response {
    match binding_repo::delete_actuator_binding(&state.db, &id).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => error(
            StatusCode::NOT_FOUND,
            "binding_not_found",
            "this plant has no actuator binding",
        ),
        Err(_) => storage_error(),
    }
}

#[cfg(test)]
mod tests {
    use super::super::testsupport::TestApi;
    use axum::http::StatusCode;

    fn sensor(sensor_id: &str, point: &str, kind: &str, role: &str) -> serde_json::Value {
        serde_json::json!({
            "device_id": "plant-node-01",
            "sensor_id": sensor_id,
            "point": point,
            "kind": kind,
            "role": role,
        })
    }

    async fn ready() -> TestApi {
        let api = TestApi::start().await;
        api.with_device().await;
        api.plant("monstera-01").await;
        api
    }

    #[tokio::test]
    async fn bindings_can_be_created_listed_and_deleted() {
        let api = ready().await;
        let created = api.bind_control("monstera-01").await;
        assert_eq!(created["kind"], "soil_moisture");
        assert_eq!(created["role"], "control");

        let (status, listed) = api.get("/api/v1/plants/monstera-01/bindings/sensors").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(listed["sensor_bindings"].as_array().unwrap().len(), 1);

        let id = created["binding_id"].as_str().unwrap();
        let (status, _) = api
            .delete(&format!("/api/v1/plants/monstera-01/bindings/sensors/{id}"))
            .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (_, empty) = api.get("/api/v1/plants/monstera-01/bindings/sensors").await;
        assert!(empty["sensor_bindings"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_binding_naming_an_undeclared_capability_is_rejected() {
        let api = ready().await;
        let (status, refused) = api
            .json(
                "PUT",
                "/api/v1/plants/monstera-01/bindings/sensors",
                sensor("soil-9", "default", "soil_moisture", "control"),
            )
            .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{refused}");
        assert_eq!(refused["error"]["code"], "undeclared_sensor");
        assert!(
            refused["error"]["message"]
                .as_str()
                .unwrap()
                .contains("soil-9")
        );

        let (status, refused) = api
            .json(
                "PUT",
                "/api/v1/plants/monstera-01/bindings/sensors",
                sensor("soil-0", "default", "illuminance", "advisory"),
            )
            .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{refused}");
        assert_eq!(refused["error"]["code"], "sensor_does_not_produce_kind");
    }

    #[tokio::test]
    async fn a_second_control_binding_is_rejected() {
        let api = ready().await;
        api.bind_control("monstera-01").await;
        let (status, refused) = api
            .json(
                "PUT",
                "/api/v1/plants/monstera-01/bindings/sensors",
                sensor("soil-0", "default", "soil_temperature", "control"),
            )
            .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{refused}");
        assert_eq!(refused["error"]["code"], "duplicate_control_binding");
    }

    #[tokio::test]
    async fn removing_the_last_control_binding_is_refused_while_automation_is_on() {
        let api = ready().await;
        let control = api.bind_control("monstera-01").await;
        api.json(
            "PATCH",
            "/api/v1/plants/monstera-01",
            serde_json::json!({ "auto_watering_enabled": true }),
        )
        .await;
        let id = control["binding_id"].as_str().unwrap();
        let (status, refused) = api
            .delete(&format!("/api/v1/plants/monstera-01/bindings/sensors/{id}"))
            .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{refused}");
        assert_eq!(
            refused["error"]["code"],
            "last_control_binding_while_automation_enabled"
        );

        api.json(
            "PATCH",
            "/api/v1/plants/monstera-01",
            serde_json::json!({ "auto_watering_enabled": false }),
        )
        .await;
        let (status, _) = api
            .delete(&format!("/api/v1/plants/monstera-01/bindings/sensors/{id}"))
            .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
    }

    /// Demoting a veto to advisory would remove it silently.
    #[tokio::test]
    async fn leak_and_tank_cannot_be_advisory_once_an_actuator_exists() {
        let api = ready().await;
        let (status, bound) = api
            .json(
                "PUT",
                "/api/v1/plants/monstera-01/bindings/actuator",
                serde_json::json!({ "device_id": "plant-node-01", "actuator_id": "pump-0" }),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{bound}");

        for (sensor_id, point, kind) in [
            ("leak-0", "tray", "leak_state"),
            ("tank-0", "reservoir", "tank_level"),
        ] {
            let (status, refused) = api
                .json(
                    "PUT",
                    "/api/v1/plants/monstera-01/bindings/sensors",
                    sensor(sensor_id, point, kind, "advisory"),
                )
                .await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{refused}");
            assert_eq!(refused["error"]["code"], "safety_role_must_be_required");

            let (status, accepted) = api
                .json(
                    "PUT",
                    "/api/v1/plants/monstera-01/bindings/sensors",
                    sensor(sensor_id, point, kind, "required"),
                )
                .await;
            assert_eq!(status, StatusCode::CREATED, "{accepted}");
        }
    }

    /// The invariant is order-independent: adding the actuator after an
    /// advisory safety binding must be refused just as the reverse order is.
    #[tokio::test]
    async fn an_actuator_cannot_be_added_over_an_advisory_safety_binding() {
        let api = ready().await;
        let (status, advisory) = api
            .json(
                "PUT",
                "/api/v1/plants/monstera-01/bindings/sensors",
                sensor("leak-0", "tray", "leak_state", "advisory"),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{advisory}");

        let (status, refused) = api
            .json(
                "PUT",
                "/api/v1/plants/monstera-01/bindings/actuator",
                serde_json::json!({ "device_id": "plant-node-01", "actuator_id": "pump-0" }),
            )
            .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{refused}");
        assert_eq!(refused["error"]["code"], "safety_role_must_be_required");
        let (_, body) = api
            .get("/api/v1/plants/monstera-01/bindings/actuator")
            .await;
        assert_eq!(body["actuator_binding"], serde_json::Value::Null);
    }

    /// SCEN-106: a monitoring-only plant is fully functional and its actuator
    /// endpoint answers `null` rather than 404 — absence is normal.
    #[tokio::test]
    async fn scen_106_a_monitoring_only_plant_is_first_class() {
        let api = ready().await;
        api.bind_control("monstera-01").await;
        let (status, actuator) = api
            .get("/api/v1/plants/monstera-01/bindings/actuator")
            .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(actuator["actuator_binding"], serde_json::Value::Null);

        let (status, plant) = api.get("/api/v1/plants/monstera-01").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(plant["has_actuator"], false);
        assert_eq!(plant["bindings"].as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn an_actuator_binding_must_name_a_declared_pump() {
        let api = ready().await;
        let (status, refused) = api
            .json(
                "PUT",
                "/api/v1/plants/monstera-01/bindings/actuator",
                serde_json::json!({ "device_id": "plant-node-01", "actuator_id": "pump-9" }),
            )
            .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{refused}");
        assert_eq!(refused["error"]["code"], "undeclared_actuator");

        let (status, refused) = api
            .json(
                "PUT",
                "/api/v1/plants/monstera-01/bindings/actuator",
                serde_json::json!({
                    "device_id": "plant-node-01", "actuator_id": "pump-0", "kind": "grow_light"
                }),
            )
            .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{refused}");
        assert_eq!(refused["error"]["code"], "unsupported_actuator_kind");
    }

    /// Replacing a probe is a binding edit: policies and history are untouched.
    #[tokio::test]
    async fn replacing_a_sensor_preserves_history_and_policies() {
        let api = ready().await;
        let control = api.bind_control("monstera-01").await;
        api.moisture_policy("monstera-01").await;
        let (_, before) = api
            .get("/api/v1/plants/monstera-01/measurement-policies")
            .await;

        let mut edit = sensor("soil-0", "default", "soil_moisture", "control");
        edit["binding_id"] = control["binding_id"].clone();
        let (status, edited) = api
            .json("PUT", "/api/v1/plants/monstera-01/bindings/sensors", edit)
            .await;
        assert_eq!(status, StatusCode::CREATED, "{edited}");
        assert_eq!(edited["binding_id"], control["binding_id"]);

        let (_, after) = api
            .get("/api/v1/plants/monstera-01/measurement-policies")
            .await;
        assert_eq!(before, after);
        let (_, listed) = api.get("/api/v1/plants/monstera-01/bindings/sensors").await;
        assert_eq!(
            listed["sensor_bindings"].as_array().unwrap().len(),
            1,
            "an edit replaces the binding rather than adding a second control"
        );
    }

    #[tokio::test]
    async fn a_control_binding_must_be_a_scalar_kind() {
        let api = ready().await;
        let (status, refused) = api
            .json(
                "PUT",
                "/api/v1/plants/monstera-01/bindings/sensors",
                sensor("leak-0", "tray", "leak_state", "control"),
            )
            .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{refused}");
        assert_eq!(refused["error"]["code"], "control_kind_not_eligible");
    }
}
