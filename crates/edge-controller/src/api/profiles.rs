//! Profile endpoints (M5-004), `http-api-boundaries.md` §2.7.
//!
//! Profiles carry the values the safety gate will consume, so their editing
//! surface matters. A 422 body names the rule **and** the limit — an error at
//! edit time teaches the real limit while the operator is paying attention
//! ([ADR-011](../../../../docs/adr/011-configuration-and-secrets-model.md)).
#![allow(missing_docs)]
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use rhizo_domain::profile::PlantProfile;
use rhizo_storage::repo::profile as profile_repo;
use serde::Deserialize;

use super::ApiState;
use super::support::{error, error_with, storage_error, timestamp};

/// The id of the profile a first-run system starts with.
pub const DEFAULT_PROFILE_ID: &str = "default";

/// A profile as the API accepts it. `profile_id` comes from the path on `PUT`
/// and from the body on `POST`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileBody {
    #[serde(default)]
    profile_id: Option<String>,
    name: String,
    target_min_vwc: f64,
    target_max_vwc: f64,
    dose_ml: f32,
    max_doses_per_cycle: u16,
    max_daily_ml: f32,
    dry_confirm_minutes: u32,
    cooldown_hours: f64,
    absorption_minutes: u32,
}

impl ProfileBody {
    fn to_profile(&self) -> PlantProfile {
        PlantProfile {
            // The stored document keys on the row id, so the nil UUID here is a
            // placeholder rather than an identity.
            profile_id: rhizo_domain::ProfileId::from_uuid(uuid::Uuid::nil()),
            name: self.name.clone(),
            target_min_vwc: self.target_min_vwc,
            target_max_vwc: self.target_max_vwc,
            dose_ml: self.dose_ml,
            max_doses_per_cycle: self.max_doses_per_cycle,
            max_daily_ml: self.max_daily_ml,
            dry_confirm_minutes: self.dry_confirm_minutes,
            cooldown_hours: self.cooldown_hours,
            absorption_minutes: self.absorption_minutes,
        }
    }
}

fn profile_json(row: &profile_repo::ProfileRow) -> serde_json::Value {
    let document: serde_json::Value =
        serde_json::from_str(&row.profile_json).unwrap_or(serde_json::Value::Null);
    serde_json::json!({
        "profile_id": row.profile_id,
        "name": row.name,
        "updated_at": timestamp(row.updated_at),
        "target_min_vwc": document.get("target_min_vwc"),
        "target_max_vwc": document.get("target_max_vwc"),
        "dose_ml": document.get("dose_ml"),
        "max_doses_per_cycle": document.get("max_doses_per_cycle"),
        "max_daily_ml": document.get("max_daily_ml"),
        "dry_confirm_minutes": document.get("dry_confirm_minutes"),
        "cooldown_hours": document.get("cooldown_hours"),
        "absorption_minutes": document.get("absorption_minutes"),
    })
}

/// Renders a validation failure as the 422 an operator learns from.
fn rejected(error: rhizo_domain::profile::ProfileError) -> Response {
    error_with(
        StatusCode::UNPROCESSABLE_ENTITY,
        error.code(),
        &error.to_string(),
        serde_json::json!({ "rule": error.code() }),
    )
}

#[derive(Deserialize)]
pub struct ListQuery {
    cursor: Option<String>,
    limit: Option<i64>,
}

pub async fn list(State(state): State<ApiState>, Query(q): Query<ListQuery>) -> Response {
    let limit = q.limit.unwrap_or(100).clamp(1, 500);
    match profile_repo::list(&state.db, q.cursor.as_deref(), limit).await {
        Ok(rows) => {
            let next = rows.last().map(|r| r.profile_id.clone());
            Json(serde_json::json!({
                "profiles": rows.iter().map(profile_json).collect::<Vec<_>>(),
                "next_cursor": if rows.len() as i64 == limit { next } else { None },
            }))
            .into_response()
        }
        Err(_) => storage_error(),
    }
}

pub async fn get(State(state): State<ApiState>, Path(id): Path<String>) -> Response {
    match profile_repo::get(&state.db, &id).await {
        Ok(Some(row)) => Json(profile_json(&row)).into_response(),
        Ok(None) => error(
            StatusCode::NOT_FOUND,
            "profile_not_found",
            "unknown profile",
        ),
        Err(_) => storage_error(),
    }
}

pub async fn create(State(state): State<ApiState>, Json(body): Json<ProfileBody>) -> Response {
    let Some(profile_id) = body.profile_id.clone().filter(|id| !id.trim().is_empty()) else {
        return error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_profile",
            "profile_id is required",
        );
    };
    let profile = body.to_profile();
    if let Err(e) = profile.validate() {
        return rejected(e);
    }
    let Ok(document) = serde_json::to_string(&profile) else {
        return storage_error();
    };
    let now = state.clock.now().timestamp_millis();
    match profile_repo::insert_new(&state.db, &profile_id, &profile.name, &document, now).await {
        Ok(true) => match profile_repo::get(&state.db, &profile_id).await {
            Ok(Some(row)) => (StatusCode::CREATED, Json(profile_json(&row))).into_response(),
            Ok(None) | Err(_) => storage_error(),
        },
        Ok(false) => error(
            StatusCode::CONFLICT,
            "profile_conflict",
            "a profile with that id already exists",
        ),
        Err(_) => storage_error(),
    }
}

pub async fn put(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Json(body): Json<ProfileBody>,
) -> Response {
    let profile = body.to_profile();
    if let Err(e) = profile.validate() {
        return rejected(e);
    }
    let Ok(document) = serde_json::to_string(&profile) else {
        return storage_error();
    };
    let now = state.clock.now().timestamp_millis();
    // Editing a profile changes the template. It does **not** rewrite the
    // `MeasurementPolicy` rows of plants already seeded from it (ADR-016): a
    // plant's configuration is the plant's. What it does change is the dose and
    // cooldown the next evaluation reads, which is why the effect shows up on
    // the next tick rather than immediately.
    match profile_repo::upsert(&state.db, &id, &profile.name, &document, now).await {
        Ok(row) => Json(profile_json(&row)).into_response(),
        Err(_) => storage_error(),
    }
}

/// Seeds one sensible default profile so a first-run system is usable without
/// the operator inventing numbers.
///
/// Idempotent: it inserts only when the id is free, so an operator who edits the
/// default keeps their edit across restarts.
pub async fn seed_default(
    db: &rhizo_storage::EdgeDb,
    now: i64,
) -> Result<bool, rhizo_storage::StorageError> {
    let profile = rhizo_domain::profile::PlantProfile::default_seed(
        rhizo_domain::ProfileId::from_uuid(uuid::Uuid::nil()),
    );
    let document = serde_json::to_string(&profile)
        .map_err(|e| rhizo_storage::StorageError::Serialization(e.to_string()))?;
    profile_repo::insert_new(db, DEFAULT_PROFILE_ID, &profile.name, &document, now).await
}

#[cfg(test)]
mod tests {
    use super::super::testsupport::{TestApi, base};
    use axum::http::StatusCode;

    fn body(dose_ml: f64) -> serde_json::Value {
        serde_json::json!({
            "profile_id": "monstera_default",
            "name": "Monstera",
            "target_min_vwc": 28.0,
            "target_max_vwc": 45.0,
            "dose_ml": dose_ml,
            "max_doses_per_cycle": 3,
            "max_daily_ml": 300.0,
            "dry_confirm_minutes": 30,
            "cooldown_hours": 6.0,
            "absorption_minutes": 30,
        })
    }

    #[tokio::test]
    async fn profiles_round_trip_through_the_documented_shape() {
        let api = TestApi::start().await;
        let (status, created) = api.json("POST", "/api/v1/profiles", body(40.0)).await;
        assert_eq!(status, StatusCode::CREATED, "{created}");
        assert_eq!(created["profile_id"], "monstera_default");
        assert_eq!(created["dose_ml"], 40.0);

        let (status, fetched) = api.get("/api/v1/profiles/monstera_default").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(fetched["cooldown_hours"], 6.0);

        let (status, listed) = api.get("/api/v1/profiles").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(listed["profiles"].as_array().unwrap().len(), 1);

        let (status, conflict) = api.json("POST", "/api/v1/profiles", body(40.0)).await;
        assert_eq!(status, StatusCode::CONFLICT, "{conflict}");

        let mut edit = body(35.0);
        edit["name"] = serde_json::json!("Monstera v2");
        let (status, updated) = api
            .json("PUT", "/api/v1/profiles/monstera_default", edit)
            .await;
        assert_eq!(status, StatusCode::OK, "{updated}");
        assert_eq!(updated["name"], "Monstera v2");
        assert_eq!(updated["dose_ml"], 35.0);

        let (status, _) = api.get("/api/v1/profiles/absent").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    /// PRD 050's named case, and ADR-011's whole argument: the 422 names the
    /// rule **and** the limit, so the operator learns it while editing.
    #[tokio::test]
    async fn a_dose_of_200_is_rejected_with_422_naming_the_firmware_limit() {
        let api = TestApi::start().await;
        let (status, refused) = api.json("POST", "/api/v1/profiles", body(200.0)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{refused}");
        assert_eq!(refused["error"]["code"], "dose_above_firmware_limit");
        let message = refused["error"]["message"].as_str().unwrap();
        assert!(message.contains("FIRMWARE_MAX_ML_PER_RUN"), "{message}");
        assert!(message.contains("200"), "{message}");
        assert!(message.contains("80"), "{message}");
        assert_eq!(
            refused["error"]["details"]["rule"],
            "dose_above_firmware_limit"
        );

        // Nothing was stored: a rejected profile is not a clamped one.
        let (_, listed) = api.get("/api/v1/profiles").await;
        assert!(listed["profiles"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn each_violated_rule_reports_its_own_code() {
        let api = TestApi::start().await;
        let cases = [
            (serde_json::json!({"target_min_vwc": 50.0}), "target_range"),
            (serde_json::json!({"max_doses_per_cycle": 0}), "zero_doses"),
            (
                serde_json::json!({"dry_confirm_minutes": 0}),
                "non_positive_interval",
            ),
            (
                serde_json::json!({"max_daily_ml": 900.0}),
                "daily_above_firmware_limit",
            ),
            (
                serde_json::json!({"max_doses_per_cycle": 20}),
                "cycle_volume_above_daily_max",
            ),
        ];
        for (patch, code) in cases {
            let mut candidate = body(40.0);
            for (key, value) in patch.as_object().unwrap() {
                candidate[key] = value.clone();
            }
            let (status, refused) = api.json("POST", "/api/v1/profiles", candidate).await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{refused}");
            assert_eq!(refused["error"]["code"], code, "{refused}");
        }
    }

    #[tokio::test]
    async fn a_default_profile_is_seeded_once_and_an_edit_survives() {
        let api = TestApi::start().await;
        assert!(
            super::seed_default(&api.db, base().timestamp_millis())
                .await
                .unwrap()
        );
        let (status, seeded) = api.get("/api/v1/profiles/default").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(seeded["dose_ml"], 40.0);
        assert_eq!(seeded["target_min_vwc"], 28.0);

        let mut edit = body(25.0);
        edit["name"] = serde_json::json!("Mine");
        api.json("PUT", "/api/v1/profiles/default", edit).await;
        assert!(
            !super::seed_default(&api.db, base().timestamp_millis())
                .await
                .unwrap(),
            "seeding is idempotent"
        );
        let (_, after) = api.get("/api/v1/profiles/default").await;
        assert_eq!(
            after["dose_ml"], 25.0,
            "an operator edit is not overwritten"
        );
    }

    /// ADR-016: editing a profile does not rewrite the policies of plants
    /// already seeded from it. What it does change is what the next evaluation
    /// reads for dose and cooldown.
    #[tokio::test]
    async fn editing_a_profile_does_not_modify_existing_plants() {
        let api = TestApi::start().await;
        api.with_device().await;
        api.json("POST", "/api/v1/profiles", body(40.0)).await;
        let (status, _) = api
            .json(
                "POST",
                "/api/v1/plants",
                serde_json::json!({
                    "plant_id": "monstera-01",
                    "name": "Monstera",
                    "profile_id": "monstera_default",
                    "pot_volume_ml": 2000.0,
                }),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED);
        api.bind_control("monstera-01").await;
        api.moisture_policy("monstera-01").await;
        let (_, before) = api
            .get("/api/v1/plants/monstera-01/measurement-policies")
            .await;

        let mut edit = body(35.0);
        edit["target_min_vwc"] = serde_json::json!(5.0);
        edit["target_max_vwc"] = serde_json::json!(9.0);
        api.json("PUT", "/api/v1/profiles/monstera_default", edit)
            .await;

        let (_, after) = api
            .get("/api/v1/plants/monstera-01/measurement-policies")
            .await;
        assert_eq!(
            before, after,
            "a profile edit must not silently change an existing plant's rules"
        );
        assert_eq!(after["measurement_policies"][0]["target_min"], 28.0);
    }
}
