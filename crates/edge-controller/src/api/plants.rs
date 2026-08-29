//! Plant endpoints (M5-002, M5-018), `http-api-boundaries.md` §2.4.
//!
//! Plants are the operator's primary object. Everything here reads or writes the
//! ADR-016 model directly; nothing publishes.
#![allow(missing_docs)]
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use rhizo_domain::state::LockoutReason;
use rhizo_storage::repo::{plant as plant_repo, query};
use serde::Deserialize;

use super::ApiState;
use super::support::{error, error_with, parse_timestamp, storage_error, timestamp};
use crate::plant;

/// The raw-resolution cap. A year-long request would otherwise exhaust memory,
/// and truncating silently would hand the caller an incomplete series they had
/// no way to know was incomplete.
pub const MAX_RAW_POINTS: i64 = 5_000;

/// The parts of a plant response that are computed rather than stored.
struct Derived {
    state: Option<String>,
    latest: serde_json::Value,
    bindings: serde_json::Value,
    thresholds: serde_json::Value,
    budget: serde_json::Value,
    last_watering: serde_json::Value,
    has_actuator: bool,
}

fn plant_json(plant: &plant_repo::PlantRow, derived: &Derived) -> serde_json::Value {
    let Derived {
        state,
        latest,
        bindings,
        thresholds,
        budget,
        last_watering,
        has_actuator,
    } = derived;
    serde_json::json!({
        "plant_id": plant.plant_id,
        "name": plant.name,
        "species": plant.species,
        "profile_id": plant.profile_id,
        "pot_volume_ml": plant.pot_volume_ml,
        "soil_type": plant.soil_type,
        "state": state,
        // The irrigation state machine is M6's and is a *separate* concept: a
        // plant can sit in `water_recommended` indefinitely with automation off,
        // and the machine would have nothing to say about it.
        "irrigation_state": serde_json::Value::Null,
        "auto_watering_enabled": plant.auto_watering_enabled,
        "has_actuator": has_actuator,
        "lockout": plant.lockout_reason.as_ref().map(|reason| serde_json::json!({
            "reason": reason,
            "since": plant.lockout_since.and_then(timestamp),
            "clearable": false,
            "message": "watering is locked out",
        })),
        "latest": latest,
        "bindings": bindings,
        "thresholds": thresholds,
        "water_budget": budget,
        "last_watering": last_watering,
        "applied_preset_id": plant.applied_preset_id,
        "applied_catalogue_version": plant.applied_catalogue_version,
        "created_at": timestamp(plant.created_at),
    })
}

async fn one(
    state: &ApiState,
    plant_id: &str,
) -> Result<Option<serde_json::Value>, rhizo_storage::StorageError> {
    let Some(loaded) = plant::load(&state.db, plant_id).await? else {
        return Ok(None);
    };
    let now_ms = state.clock.now().timestamp_millis();
    let mut latest = serde_json::Map::new();
    for bound in &loaded.sensors {
        let device = bound.binding.device_id.to_string();
        let Some(row) = query::latest_measurement(
            &state.db,
            &device,
            bound.binding.point.as_str(),
            bound.binding.kind.as_str(),
        )
        .await?
        else {
            continue;
        };
        latest.insert(
            bound.binding.kind.as_str().to_owned(),
            serde_json::json!({
                "value": row.value_num.or_else(|| row.value_bool.map(|v| f64::from(v != 0))),
                "unit": row.unit,
                "quality": row.quality,
                "measured_at": timestamp(row.received_at),
                "age_seconds": (now_ms.saturating_sub(row.received_at).max(0)) / 1000,
            }),
        );
    }
    let bindings = serde_json::Value::Array(
        loaded
            .sensors
            .iter()
            .map(|b| {
                serde_json::json!({
                    "binding_id": b.binding_id,
                    "device_id": b.binding.device_id.to_string(),
                    "sensor_id": b.binding.sensor_id.as_str(),
                    "point": b.binding.point.as_str(),
                    "kind": b.binding.kind.as_str(),
                    "role": plant::role_name(b.binding.role),
                })
            })
            .collect(),
    );
    let thresholds = serde_json::Value::Object(
        plant_repo::threshold_states(&state.db, plant_id)
            .await?
            .into_iter()
            .map(|(kind, severity)| (kind, serde_json::Value::String(severity)))
            .collect(),
    );
    let window_start = now_ms - 24 * 60 * 60 * 1_000;
    let delivered = plant_repo::delivered_since(&state.db, plant_id, window_start).await?;
    let budget = serde_json::json!({
        "delivered_last_24h_ml": delivered,
        "max_daily_ml": loaded.profile.max_daily_ml,
        "remaining_ml": (f64::from(loaded.profile.max_daily_ml) - delivered).max(0.0),
    });
    let last = plant_repo::watering_events(&state.db, plant_id, None, None, 1).await?;
    let last_watering = last.first().map_or(serde_json::Value::Null, |row| {
        serde_json::json!({
            "completed_at": row.completed_at.and_then(timestamp),
            "mode": row.mode,
            "delivered_ml": row.delivered_ml,
        })
    });
    let plant_state = plant_repo::plant_state(&state.db, plant_id).await?;
    Ok(Some(plant_json(
        &loaded.plant,
        &Derived {
            state: plant_state,
            latest: serde_json::Value::Object(latest),
            bindings,
            thresholds,
            budget,
            last_watering,
            has_actuator: loaded.actuator.is_some(),
        },
    )))
}

#[derive(Deserialize)]
pub struct ListQuery {
    cursor: Option<String>,
    limit: Option<i64>,
}

pub async fn list(State(state): State<ApiState>, Query(q): Query<ListQuery>) -> Response {
    let limit = q.limit.unwrap_or(100).clamp(1, 500);
    let rows = match plant_repo::list(&state.db, q.cursor.as_deref(), limit).await {
        Ok(rows) => rows,
        Err(_) => return storage_error(),
    };
    let next = rows.last().map(|r| r.plant_id.clone());
    let mut plants = Vec::new();
    for row in rows {
        match one(&state, &row.plant_id).await {
            Ok(Some(value)) => plants.push(value),
            Ok(None) => {}
            Err(_) => return storage_error(),
        }
    }
    Json(serde_json::json!({
        "plants": plants,
        "next_cursor": if plants.len() as i64 == limit { next } else { None },
    }))
    .into_response()
}

pub async fn get(State(state): State<ApiState>, Path(id): Path<String>) -> Response {
    match one(&state, &id).await {
        Ok(Some(value)) => Json(value).into_response(),
        Ok(None) => error(StatusCode::NOT_FOUND, "plant_not_found", "unknown plant"),
        Err(_) => storage_error(),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreatePlant {
    plant_id: String,
    name: String,
    #[serde(default)]
    species: Option<String>,
    #[serde(default)]
    profile_id: Option<String>,
    #[serde(default)]
    pot_volume_ml: Option<f64>,
    #[serde(default)]
    soil_type: Option<String>,
    /// Optional starting configuration (M5-018). Absent behaves exactly as it
    /// did before presets existed: the manual path is not a fallback, it is the
    /// same first-class path it always was.
    #[serde(default)]
    preset_id: Option<String>,
}

pub async fn create(State(state): State<ApiState>, Json(body): Json<CreatePlant>) -> Response {
    if body.plant_id.trim().is_empty() || body.name.trim().is_empty() {
        return error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_plant",
            "plant_id and name are required",
        );
    }
    if let Some(preset_id) = body.preset_id.as_deref()
        && rhizo_domain::preset::catalogue::get(preset_id).is_none()
    {
        return error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "unknown_preset",
            &format!("{preset_id} is not in the embedded catalogue"),
        );
    }
    let now = state.clock.now().timestamp_millis();
    let created = plant_repo::create(
        &state.db,
        &plant_repo::NewPlant {
            plant_id: body.plant_id.clone(),
            name: body.name,
            species: body.species,
            profile_id: body.profile_id,
            pot_volume_ml: body.pot_volume_ml,
            soil_type: body.soil_type,
        },
        now,
    )
    .await;
    match created {
        Ok(_) => {}
        Err(rhizo_storage::StorageError::Constraint(message)) => {
            return error(StatusCode::CONFLICT, "plant_conflict", &message);
        }
        Err(_) => return storage_error(),
    }
    // A preset applied at creation resolves against the bindings the plant has,
    // which at this moment is none. That is not a failure: the kinds are
    // reported as skipped, and applying again after binding sensors configures
    // them. The alternative -- a preset that created bindings -- would have the
    // catalogue guessing which probe is in which pot.
    if let Some(preset_id) = body.preset_id.as_deref() {
        match plant::load(&state.db, &body.plant_id).await {
            Ok(Some(loaded)) => {
                if let Err(e) = plant::preset::apply(&state.db, &loaded, preset_id, true, now).await
                {
                    return match e {
                        plant::preset::ApplyError::Preset(error) => error_with(
                            StatusCode::UNPROCESSABLE_ENTITY,
                            error.code(),
                            &error.to_string(),
                            serde_json::json!({ "preset_id": preset_id }),
                        ),
                        plant::preset::ApplyError::Storage(_) => storage_error(),
                    };
                }
            }
            Ok(None) | Err(_) => return storage_error(),
        }
    }
    match one(&state, &body.plant_id).await {
        Ok(Some(value)) => (StatusCode::CREATED, Json(value)).into_response(),
        Ok(None) | Err(_) => storage_error(),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatchPlant {
    #[serde(default)]
    name: Option<String>,
    #[serde(default, with = "serde_with_option")]
    species: Option<Option<String>>,
    #[serde(default, with = "serde_with_option")]
    soil_type: Option<Option<String>>,
    #[serde(default)]
    profile_id: Option<String>,
    #[serde(default)]
    pot_volume_ml: Option<f64>,
    #[serde(default)]
    auto_watering_enabled: Option<bool>,
}

/// `null` clears a field; an absent key leaves it alone. `serde` collapses the
/// two by default, and the difference is the whole point of a PATCH.
mod serde_with_option {
    use serde::{Deserialize, Deserializer};
    pub fn deserialize<'de, D, T>(d: D) -> Result<Option<Option<T>>, D::Error>
    where
        D: Deserializer<'de>,
        T: Deserialize<'de>,
    {
        Option::<T>::deserialize(d).map(Some)
    }
}

pub async fn patch(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Json(body): Json<PatchPlant>,
) -> Response {
    let patch = plant_repo::PlantPatch {
        name: body.name,
        species: body.species,
        soil_type: body.soil_type,
        profile_id: body.profile_id.map(Some),
        pot_volume_ml: body.pot_volume_ml.map(Some),
        auto_watering_enabled: body.auto_watering_enabled,
    };
    match plant_repo::update(&state.db, &id, &patch).await {
        Ok(Some(_)) => get(State(state), Path(id)).await,
        Ok(None) => error(StatusCode::NOT_FOUND, "plant_not_found", "unknown plant"),
        Err(rhizo_storage::StorageError::Constraint(message)) => {
            error(StatusCode::UNPROCESSABLE_ENTITY, "invalid_plant", &message)
        }
        Err(_) => storage_error(),
    }
}

pub async fn delete(State(state): State<ApiState>, Path(id): Path<String>) -> Response {
    let now = state.clock.now().timestamp_millis();
    match plant_repo::delete(&state.db, &id, now).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => error(StatusCode::NOT_FOUND, "plant_not_found", "unknown plant"),
        Err(_) => storage_error(),
    }
}

#[derive(Deserialize)]
pub struct MeasurementQuery {
    from: Option<String>,
    to: Option<String>,
    resolution: Option<String>,
}

pub async fn measurements(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Query(q): Query<MeasurementQuery>,
) -> Response {
    let Ok(Some(loaded)) = plant::load(&state.db, &id).await else {
        return error(StatusCode::NOT_FOUND, "plant_not_found", "unknown plant");
    };
    // `resolution` is reserved now so the API stays stable. Only `raw` is
    // implemented in M5; downsampling arrives in M13-010.
    match q.resolution.as_deref().unwrap_or("raw") {
        "raw" => {}
        r @ ("minute" | "hour" | "day") => {
            return error(
                StatusCode::NOT_IMPLEMENTED,
                "resolution_not_implemented",
                &format!("resolution={r} is reserved for M13-010; only raw is implemented"),
            );
        }
        other => {
            return error(
                StatusCode::UNPROCESSABLE_ENTITY,
                "invalid_resolution",
                &format!("{other} is not one of raw, minute, hour, day"),
            );
        }
    }
    let from = q.from.as_deref().and_then(parse_timestamp).unwrap_or(0);
    let to =
        q.to.as_deref()
            .and_then(parse_timestamp)
            .unwrap_or_else(|| state.clock.now().timestamp_millis());

    let mut total = 0;
    for bound in &loaded.sensors {
        let device = bound.binding.device_id.to_string();
        match query::count_measurements_for(
            &state.db,
            &device,
            bound.binding.point.as_str(),
            bound.binding.kind.as_str(),
            from,
            to,
        )
        .await
        {
            Ok(count) => total += count,
            Err(_) => return storage_error(),
        }
    }
    if total > MAX_RAW_POINTS {
        // Naming the cap rather than truncating: a caller handed a silently
        // shortened series has no way to know the series is incomplete.
        return error_with(
            StatusCode::UNPROCESSABLE_ENTITY,
            "too_many_points",
            &format!(
                "the requested window holds {total} points; the raw cap is {MAX_RAW_POINTS}. \
                 Narrow from/to, or wait for resolution=hour (M13-010)."
            ),
            serde_json::json!({ "points": total, "cap": MAX_RAW_POINTS }),
        );
    }
    let mut series = Vec::new();
    for bound in &loaded.sensors {
        let device = bound.binding.device_id.to_string();
        let rows = match query::measurements_for(
            &state.db,
            &device,
            bound.binding.point.as_str(),
            bound.binding.kind.as_str(),
            from,
            to,
            MAX_RAW_POINTS,
        )
        .await
        {
            Ok(rows) => rows,
            Err(_) => return storage_error(),
        };
        series.push(serde_json::json!({
            "kind": bound.binding.kind.as_str(),
            "point": bound.binding.point.as_str(),
            "device_id": device,
            "role": plant::role_name(bound.binding.role),
            "points": rows.iter().map(|r| serde_json::json!({
                "at": timestamp(r.received_at),
                "value": r.value_num.or_else(|| r.value_bool.map(|v| f64::from(v != 0))),
                "unit": r.unit,
                "quality": r.quality,
            })).collect::<Vec<_>>(),
        }));
    }
    Json(serde_json::json!({
        "plant_id": id,
        "resolution": "raw",
        "from": timestamp(from),
        "to": timestamp(to),
        "series": series,
    }))
    .into_response()
}

#[derive(Deserialize)]
pub struct EventWindow {
    from: Option<String>,
    to: Option<String>,
    limit: Option<i64>,
}

pub async fn watering_events(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Query(q): Query<EventWindow>,
) -> Response {
    match plant_repo::get(&state.db, &id).await {
        Ok(Some(_)) => {}
        Ok(None) => return error(StatusCode::NOT_FOUND, "plant_not_found", "unknown plant"),
        Err(_) => return storage_error(),
    }
    let rows = plant_repo::watering_events(
        &state.db,
        &id,
        q.from.as_deref().and_then(parse_timestamp),
        q.to.as_deref().and_then(parse_timestamp),
        q.limit.unwrap_or(100),
    )
    .await;
    match rows {
        Ok(rows) => Json(serde_json::json!({
            "watering_events": rows.iter().map(|r| serde_json::json!({
                "watering_event_id": r.watering_event_id,
                "command_id": r.command_id,
                "mode": r.mode,
                "origin": r.origin,
                "started_at": timestamp(r.started_at),
                "completed_at": r.completed_at.and_then(timestamp),
                "requested_ml": r.requested_ml,
                "delivered_ml": r.delivered_ml,
                "status": r.status,
                "detail": r.reason_json.as_deref().and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok()),
            })).collect::<Vec<_>>()
        }))
        .into_response(),
        Err(_) => storage_error(),
    }
}

pub async fn events(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Query(q): Query<EventWindow>,
) -> Response {
    match plant_repo::plant_events(&state.db, &id, q.limit.unwrap_or(100)).await {
        Ok(rows) => Json(serde_json::json!({
            "events": rows.iter().map(|(kind, severity, detail, at)| serde_json::json!({
                "kind": kind,
                "severity": severity,
                "detail": detail.as_deref().and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok()),
                "occurred_at": timestamp(*at),
            })).collect::<Vec<_>>()
        }))
        .into_response(),
        Err(_) => storage_error(),
    }
}

/// The documented request body, accepted so a client written against M6 gets a
/// meaningful refusal rather than a deserialisation failure. The fields are
/// deliberately unread in M5: nothing here can act on them, and there is no
/// override, force, or bypass field for them to hide behind.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(
    dead_code,
    reason = "the body is validated in M5 and acted on from M6-016"
)]
pub struct WaterRequest {
    #[serde(default)]
    ml: Option<f64>,
    #[serde(default)]
    mode: Option<String>,
}

/// The actuation endpoint, present in M5 **only so that it can refuse**.
///
/// SAFETY-018 needs the refusal to be *distinguishable*: a monitoring-only plant
/// answers **422** `no_actuator_bound`, which is "there is nothing to water
/// with", not 409, which means "refused by safety", and not 404, which would be
/// indistinguishable from an unknown plant.
///
/// A plant that *does* have an actuator answers 501. M5 issues no commands, and
/// saying so plainly is better than a route that does not exist — a 404 there
/// would tell an operator their pump is unknown when it is merely not yet
/// wired up. M6-016 replaces this arm with the gate and the command lifecycle.
/// There is no override, force, or bypass parameter here, and there never will
/// be.
pub async fn water(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Json(_body): Json<WaterRequest>,
) -> Response {
    let loaded = match plant::load(&state.db, &id).await {
        Ok(Some(loaded)) => loaded,
        Ok(None) => return error(StatusCode::NOT_FOUND, "plant_not_found", "unknown plant"),
        Err(_) => return storage_error(),
    };
    if loaded.actuator.is_none() {
        return error_with(
            StatusCode::UNPROCESSABLE_ENTITY,
            "no_actuator_bound",
            "this plant has no actuator, so there is no watering path to refuse or allow",
            serde_json::json!({ "lockout": plant::lockout_name(LockoutReason::NoActuator) }),
        );
    }
    error(
        StatusCode::NOT_IMPLEMENTED,
        "watering_not_implemented",
        "M5 recommends; M6 acts. This edge issues no watering commands.",
    )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApplyPreset {
    preset_id: String,
    #[serde(default)]
    overwrite: bool,
}

pub async fn apply_preset(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Json(body): Json<ApplyPreset>,
) -> Response {
    let loaded = match plant::load(&state.db, &id).await {
        Ok(Some(loaded)) => loaded,
        Ok(None) => return error(StatusCode::NOT_FOUND, "plant_not_found", "unknown plant"),
        Err(_) => return storage_error(),
    };
    let now = state.clock.now().timestamp_millis();
    match plant::preset::apply(&state.db, &loaded, &body.preset_id, body.overwrite, now).await {
        Ok(applied) => Json(serde_json::json!({
            "plant_id": id,
            "preset_id": applied.preset_id,
            "catalogue_version": applied.catalogue_version,
            "configured_kinds": applied.configured_kinds,
            "replaced_kinds": applied.replaced_kinds,
            "skipped_unbound_kinds": applied.skipped_unbound_kinds,
            "dose_ml": applied.dose_ml,
            "cooldown_hours": applied.cooldown_hours,
            "has_actuator": applied.has_actuator,
            "auto_watering_enabled": false,
        }))
        .into_response(),
        Err(plant::preset::ApplyError::Preset(e)) => {
            let status = if matches!(e, plant::preset::PresetError::AlreadyConfigured { .. }) {
                StatusCode::CONFLICT
            } else {
                StatusCode::UNPROCESSABLE_ENTITY
            };
            error_with(
                status,
                e.code(),
                &e.to_string(),
                serde_json::json!({ "preset_id": body.preset_id }),
            )
        }
        Err(plant::preset::ApplyError::Storage(_)) => storage_error(),
    }
}

/// Ensures the plant exists, for handlers that only need that much.
pub(crate) async fn exists(
    state: &ApiState,
    plant_id: &str,
) -> Result<bool, rhizo_storage::StorageError> {
    Ok(plant_repo::get(&state.db, plant_id).await?.is_some())
}

#[cfg(test)]
mod tests {
    use super::super::testsupport::{TestApi, base};
    use axum::http::StatusCode;
    use chrono::Duration;

    #[tokio::test]
    async fn plants_crud_returns_the_documented_shape_and_defaults_to_off() {
        let api = TestApi::start().await;
        let created = api.plant("monstera-01").await;
        assert_eq!(created["plant_id"], "monstera-01");
        assert_eq!(
            created["auto_watering_enabled"], false,
            "F-050-01: a plant created by any path is inert until a human opts in"
        );
        assert_eq!(created["has_actuator"], false);
        assert_eq!(created["lockout"], serde_json::Value::Null);
        assert!(created["water_budget"]["max_daily_ml"].is_number());
        assert!(created["created_at"].is_string());

        let (status, fetched) = api.get("/api/v1/plants/monstera-01").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(fetched["name"], "Monstera");

        let (status, patched) = api
            .json(
                "PATCH",
                "/api/v1/plants/monstera-01",
                serde_json::json!({ "name": "Big Monstera", "species": null }),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{patched}");
        assert_eq!(patched["name"], "Big Monstera");
        assert_eq!(
            patched["species"],
            serde_json::Value::Null,
            "an explicit null clears a field; an absent key leaves it alone"
        );

        let (status, _) = api.delete("/api/v1/plants/monstera-01").await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (status, _) = api.get("/api/v1/plants/monstera-01").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn unknown_plants_are_404_everywhere() {
        let api = TestApi::start().await;
        for uri in [
            "/api/v1/plants/absent",
            "/api/v1/plants/absent/measurements",
            "/api/v1/plants/absent/watering-events",
            "/api/v1/plants/absent/recommendation",
        ] {
            let (status, body) = api.get(uri).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{uri}: {body}");
            assert_eq!(body["error"]["code"], "plant_not_found");
        }
    }

    #[tokio::test]
    async fn list_endpoints_paginate_by_cursor() {
        let api = TestApi::start().await;
        for id in ["a-plant", "b-plant", "c-plant"] {
            api.plant(id).await;
        }
        let (status, page) = api.get("/api/v1/plants?limit=2").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(page["plants"].as_array().unwrap().len(), 2);
        assert_eq!(page["next_cursor"], "b-plant");
        let (_, rest) = api.get("/api/v1/plants?limit=2&cursor=b-plant").await;
        assert_eq!(rest["plants"].as_array().unwrap().len(), 1);
        assert_eq!(rest["plants"][0]["plant_id"], "c-plant");
        assert_eq!(
            rest["next_cursor"],
            serde_json::Value::Null,
            "a short page is the last page"
        );
    }

    #[tokio::test]
    async fn measurements_respect_the_window_and_refuse_rather_than_truncate() {
        let api = TestApi::start().await;
        api.with_device().await;
        api.plant("monstera-01").await;
        api.bind_control("monstera-01").await;
        for i in 0..10 {
            api.sample(base() - Duration::minutes(i * 5), 30.0 + i as f64)
                .await;
        }
        let (status, body) = api.get("/api/v1/plants/monstera-01/measurements").await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["resolution"], "raw");
        assert_eq!(body["series"][0]["kind"], "soil_moisture");
        assert_eq!(body["series"][0]["points"].as_array().unwrap().len(), 10);

        // A narrow window returns only what is inside it.
        // `Z` rather than `+00:00`: a literal `+` in a query string decodes as
        // a space, which would silently widen the window under test.
        let from =
            (base() - Duration::minutes(12)).to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let (_, narrowed) = api
            .get(&format!(
                "/api/v1/plants/monstera-01/measurements?from={from}"
            ))
            .await;
        assert_eq!(narrowed["series"][0]["points"].as_array().unwrap().len(), 3);

        // Reserved resolutions are honest about being unimplemented.
        let (status, body) = api
            .get("/api/v1/plants/monstera-01/measurements?resolution=hour")
            .await;
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
        assert_eq!(body["error"]["code"], "resolution_not_implemented");
        let (status, _) = api
            .get("/api/v1/plants/monstera-01/measurements?resolution=fortnight")
            .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    }

    /// Exceeding the cap returns a specific error naming it, never a silently
    /// shortened series.
    #[tokio::test]
    async fn exceeding_the_raw_cap_names_the_cap() {
        let api = TestApi::start().await;
        api.with_device().await;
        api.plant("monstera-01").await;
        api.bind_control("monstera-01").await;
        // One insert per row would take minutes; the cap is about the count.
        let mut tx = api.db.begin().await.unwrap();
        for i in 0..(super::MAX_RAW_POINTS + 1) {
            sqlx::query(
                "INSERT INTO measurements(device_id,sensor_id,point,kind,value_num,unit,quality,received_at,batch_id,origin) \
                 VALUES('plant-node-01','soil-0','default','soil_moisture',30.0,'vwc_percent','ok',?,?,'live')",
            )
            .bind(base().timestamp_millis() - i)
            .bind(i.to_string())
            .execute(&mut *tx)
            .await
            .unwrap();
        }
        tx.commit().await.unwrap();

        let (status, body) = api.get("/api/v1/plants/monstera-01/measurements").await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body["error"]["code"], "too_many_points");
        assert_eq!(body["error"]["details"]["cap"], super::MAX_RAW_POINTS);
        assert!(
            body["error"]["message"]
                .as_str()
                .unwrap()
                .contains(&super::MAX_RAW_POINTS.to_string()),
            "the message must name the cap: {body}"
        );
    }

    /// SAFETY-018. The refusal is **422 and distinguishable**: not 409, which
    /// means "refused by safety", and not 404, which would be indistinguishable
    /// from an unknown plant.
    #[tokio::test]
    async fn safety_018_no_actuator_no_command() {
        let api = TestApi::start().await;
        api.with_device().await;
        api.plant("fern-01").await;
        let (status, body) = api
            .json(
                "POST",
                "/api/v1/plants/fern-01/water",
                serde_json::json!({ "ml": 30.0, "mode": "manual" }),
            )
            .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        assert_eq!(body["error"]["code"], "no_actuator_bound");
        assert_ne!(
            status,
            StatusCode::CONFLICT,
            "409 means refused by safety, which is a different thing"
        );
        assert_eq!(
            body["error"]["details"]["lockout"], "no_actuator",
            "the lockout is named so a UI can render the reason"
        );

        // And it is still a first-class monitoring plant.
        let (status, plant) = api.get("/api/v1/plants/fern-01").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(plant["has_actuator"], false);
    }

    /// There is no override, force, or bypass parameter, and adding one would
    /// fail here rather than in review.
    #[tokio::test]
    async fn safety_018_api_returns_422_not_409_and_has_no_override() {
        let api = TestApi::start().await;
        api.with_device().await;
        api.plant("fern-01").await;
        for body in [
            serde_json::json!({ "ml": 30.0, "force": true }),
            serde_json::json!({ "ml": 30.0, "override": true }),
            serde_json::json!({ "ml": 30.0, "bypass_safety": true }),
        ] {
            let (status, _) = api.json("POST", "/api/v1/plants/fern-01/water", body).await;
            assert_eq!(
                status,
                StatusCode::UNPROCESSABLE_ENTITY,
                "an unknown field must be refused, never honoured"
            );
        }
    }

    // ---------------------------------------------------------------- presets

    #[tokio::test]
    async fn the_catalogue_is_listed_and_searchable_over_http() {
        let api = TestApi::start().await;
        let (status, body) = api.get("/api/v1/presets").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["catalogue_version"], 1);
        assert!(body["presets"].as_array().unwrap().len() >= 20);

        let (_, found) = api.get("/api/v1/presets?q=swiss%20cheese").await;
        assert_eq!(found["presets"][0]["preset_id"], "monstera-deliciosa");

        let (status, detail) = api.get("/api/v1/presets/monstera-deliciosa").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(detail["provenance"]["license"], "rhizo-authored");
        let moisture = detail["measurements"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["kind"] == "soil_moisture")
            .unwrap();
        assert_eq!(
            moisture["target_low"]["provenance"], "rhizo_default",
            "an interpretation must not be presented as a measured fact"
        );
        let ph = detail["measurements"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["kind"] == "soil_ph")
            .unwrap();
        assert_eq!(ph["target_low"]["provenance"], "source_fact");
        assert_eq!(ph["target_low"]["units"], "pH");

        let (status, _) = api.get("/api/v1/presets/triffid").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    /// Materialised rows are indistinguishable from hand-configured ones, and
    /// every value stays editable afterwards.
    #[tokio::test]
    async fn preset_materialisation_equals_hand_configuration_and_stays_editable() {
        let api = TestApi::start().await;
        api.with_device().await;
        api.plant("monstera-01").await;
        api.bind_control("monstera-01").await;

        let (status, applied) = api
            .json(
                "POST",
                "/api/v1/plants/monstera-01/apply-preset",
                serde_json::json!({ "preset_id": "monstera-deliciosa" }),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{applied}");
        assert_eq!(
            applied["configured_kinds"],
            serde_json::json!(["soil_moisture"])
        );
        assert_eq!(
            applied["skipped_unbound_kinds"],
            serde_json::json!(["ambient_temperature", "soil_ph"]),
            "a preset kind with no binding creates no policy row and is reported"
        );
        assert_eq!(applied["auto_watering_enabled"], false);
        assert_eq!(applied["has_actuator"], false);

        let (_, policies) = api
            .get("/api/v1/plants/monstera-01/measurement-policies")
            .await;
        let materialised = &policies["measurement_policies"][0];
        assert_eq!(materialised["kind"], "soil_moisture");
        assert_eq!(materialised["target_min"], 28.0);
        assert_eq!(materialised["critical_low"], 16.0);

        // A hand-configured plant with the same numbers produces the same row.
        api.plant("monstera-02").await;
        api.bind_control("monstera-02").await;
        let (status, _) = api
            .json(
                "PUT",
                "/api/v1/plants/monstera-02/measurement-policies/soil_moisture",
                serde_json::json!({
                    "target_min": 28.0, "target_max": 45.0,
                    "warning_low": 22.0, "warning_high": 55.0,
                    "critical_low": 16.0, "critical_high": 65.0,
                    "stale_after_ms": 900_000,
                    "confirm_duration_ms": 1_800_000,
                }),
            )
            .await;
        assert_eq!(status, StatusCode::OK);
        let (_, hand) = api
            .get("/api/v1/plants/monstera-02/measurement-policies")
            .await;
        assert_eq!(
            materialised, &hand["measurement_policies"][0],
            "a preset row and a hand-typed row must be the same row"
        );

        // The edit survives, and nothing re-derives it.
        let (status, _) = api
            .json(
                "PUT",
                "/api/v1/plants/monstera-01/measurement-policies/soil_moisture",
                serde_json::json!({ "target_min": 33.0, "target_max": 50.0, "stale_after_ms": 900_000 }),
            )
            .await;
        assert_eq!(status, StatusCode::OK);
        api.evaluate("monstera-01").await;
        let (_, after) = api
            .get("/api/v1/plants/monstera-01/measurement-policies")
            .await;
        assert_eq!(
            after["measurement_policies"][0]["target_min"], 33.0,
            "materialisation happens once; a tick must never re-derive a value"
        );
    }

    #[tokio::test]
    async fn applying_to_a_configured_plant_needs_overwrite_and_names_what_changed() {
        let api = TestApi::start().await;
        api.with_device().await;
        api.plant("monstera-01").await;
        api.bind_control("monstera-01").await;
        api.moisture_policy("monstera-01").await;

        let (status, refused) = api
            .json(
                "POST",
                "/api/v1/plants/monstera-01/apply-preset",
                serde_json::json!({ "preset_id": "monstera-deliciosa" }),
            )
            .await;
        assert_eq!(status, StatusCode::CONFLICT, "{refused}");
        assert_eq!(refused["error"]["code"], "already_configured");

        let (status, applied) = api
            .json(
                "POST",
                "/api/v1/plants/monstera-01/apply-preset",
                serde_json::json!({ "preset_id": "monstera-deliciosa", "overwrite": true }),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{applied}");
        assert_eq!(
            applied["replaced_kinds"],
            serde_json::json!(["soil_moisture"]),
            "the response names each changed field"
        );
    }

    /// A curated catalogue is an input, not a trusted one: a dose above the
    /// firmware ceiling is rejected with 422, never clamped.
    #[tokio::test]
    async fn a_preset_value_violating_a_hard_limit_is_rejected_with_422() {
        let api = TestApi::start().await;
        api.with_device().await;
        // A pot large enough that a `generous` dose exceeds the 80 ml ceiling.
        let (status, _) = api
            .json(
                "POST",
                "/api/v1/plants",
                serde_json::json!({
                    "plant_id": "tomato-01",
                    "name": "Tomato",
                    "pot_volume_ml": 40_000.0,
                }),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED);
        api.bind_control("tomato-01").await;
        let (status, body) = api
            .json(
                "POST",
                "/api/v1/plants/tomato-01/apply-preset",
                serde_json::json!({ "preset_id": "solanum-lycopersicum" }),
            )
            .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        assert_eq!(body["error"]["code"], "dose_above_firmware_limit");
        assert!(
            body["error"]["message"]
                .as_str()
                .unwrap()
                .contains("FIRMWARE_MAX_ML_PER_RUN"),
            "{body}"
        );
    }

    /// Applying to a monitoring-only plant succeeds and creates no actuation
    /// path (SAFETY-018).
    #[tokio::test]
    async fn safety_018_preset_creates_no_actuation_path() {
        let api = TestApi::start().await;
        api.with_device().await;
        api.plant("fern-01").await;
        api.bind_control("fern-01").await;
        let (status, applied) = api
            .json(
                "POST",
                "/api/v1/plants/fern-01/apply-preset",
                serde_json::json!({ "preset_id": "nephrolepis-exaltata" }),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{applied}");
        assert_eq!(applied["has_actuator"], false);
        assert!(
            applied["dose_ml"].is_number(),
            "recorded as an inert default"
        );

        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM actuator_bindings")
                .fetch_one(api.db.pool())
                .await
                .unwrap(),
            0,
            "nothing on this path writes to actuator_bindings"
        );
        let (_, policies) = api.get("/api/v1/plants/fern-01/measurement-policies").await;
        assert_eq!(
            policies["measurement_policies"].as_array().unwrap().len(),
            1
        );

        let (status, body) = api
            .json(
                "POST",
                "/api/v1/plants/fern-01/water",
                serde_json::json!({ "ml": 10.0 }),
            )
            .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body["error"]["code"], "no_actuator_bound");
    }

    /// Applying a preset must not touch bindings: a catalogue has no idea which
    /// probe is in which pot.
    #[tokio::test]
    async fn applying_a_preset_creates_selects_or_edits_no_binding() {
        let api = TestApi::start().await;
        api.with_device().await;
        api.plant("monstera-01").await;
        let created = api.bind_control("monstera-01").await;
        let before = sqlx::query_scalar::<_, String>(
            "SELECT group_concat(binding_id||':'||kind||':'||role) FROM sensor_bindings",
        )
        .fetch_one(api.db.pool())
        .await
        .unwrap();
        api.json(
            "POST",
            "/api/v1/plants/monstera-01/apply-preset",
            serde_json::json!({ "preset_id": "monstera-deliciosa" }),
        )
        .await;
        let after = sqlx::query_scalar::<_, String>(
            "SELECT group_concat(binding_id||':'||kind||':'||role) FROM sensor_bindings",
        )
        .fetch_one(api.db.pool())
        .await
        .unwrap();
        assert_eq!(before, after);
        assert!(after.starts_with(created["binding_id"].as_str().unwrap()));
    }

    #[tokio::test]
    async fn creating_a_plant_with_no_preset_behaves_exactly_as_before() {
        let api = TestApi::start().await;
        api.with_device().await;
        let plain = api.plant("monstera-01").await;
        assert_eq!(plain["applied_preset_id"], serde_json::Value::Null);
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM measurement_policies")
                .fetch_one(api.db.pool())
                .await
                .unwrap(),
            0,
            "the manual path writes nothing a preset would have written"
        );

        let (status, body) = api
            .json(
                "POST",
                "/api/v1/plants",
                serde_json::json!({
                    "plant_id": "bad-01", "name": "Bad", "preset_id": "triffid"
                }),
            )
            .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        assert_eq!(body["error"]["code"], "unknown_preset");
    }

    #[tokio::test]
    async fn a_plant_created_with_a_preset_records_provenance_and_stays_off() {
        let api = TestApi::start().await;
        api.with_device().await;
        let (status, created) = api
            .json(
                "POST",
                "/api/v1/plants",
                serde_json::json!({
                    "plant_id": "monstera-01",
                    "name": "Monstera",
                    "pot_volume_ml": 2000.0,
                    "preset_id": "monstera-deliciosa",
                }),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{created}");
        assert_eq!(created["applied_preset_id"], "monstera-deliciosa");
        assert_eq!(created["applied_catalogue_version"], 1);
        assert_eq!(created["auto_watering_enabled"], false);
    }
}
