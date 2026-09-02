//! Offline-policy endpoints (M5-016).
//!
//! Authoring derives a candidate from the plant's own bindings and measurement
//! policies and validates it with the shared validator — the same rules the
//! device will apply. The assembled Edge publishes the resulting per-device
//! policy set retained after each successful authoring or activation change.
//!
//! `enabled` defaults to `false`, and enabling is a separate call. Creating a
//! policy is not the same act as authorising a device to water unsupervised.
#![allow(missing_docs)]
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use rhizo_domain::offline_policy::{AuthoringInputs, author, missing_safety_bindings};
use rhizo_mqtt_contract::payload::{OfflinePolicy, OfflinePolicySet};
use rhizo_mqtt_contract::{DeviceId, Envelope, MessageId, MessageKind, Topic};
use rhizo_storage::repo::binding as binding_repo;
use sqlx::Row as _;

use super::ApiState;
use super::support::{error, error_with, storage_error, timestamp};
use crate::plant;

/// Why a policy set could not be published.
///
/// The two are answered differently and must not be collapsed: a stored
/// document this build cannot decode is a 500 the operator can do nothing
/// about, while an unreachable broker is a 503 that the same request will fix
/// when retried. Mapping both to "stored but not published" would tell an
/// operator to retry a request that can never succeed.
enum PublishError {
    /// A read or a write against the local database failed.
    Storage,
    /// The policy set could not be put on the wire.
    Transport,
}

/// Republishes the whole retained policy set for the plant's actuator device.
///
/// The topic is retained and carries **every** policy bound to that device, so
/// it is rebuilt from storage rather than patched: a device that reconnects
/// must find one document describing all of its plants, not the last one
/// edited.
///
/// `published_at` is stamped *after* the publish, in one statement covering the
/// plants that were in the document. Recording it first would claim a
/// publication that had not happened.
async fn publish_for_plant(state: &ApiState, plant_id: &str, now: i64) -> Result<(), PublishError> {
    let device_id: Option<String> =
        sqlx::query_scalar("SELECT device_id FROM actuator_bindings WHERE plant_id=?")
            .bind(plant_id)
            .fetch_optional(state.db.pool())
            .await
            .map_err(|_| PublishError::Storage)?;
    // A monitoring plant has no actuator and therefore no device to provision.
    // That is a normal plant, not a failure (SAFETY-018).
    let Some(device_id) = device_id else {
        return Ok(());
    };
    let rows = sqlx::query(
        "SELECT p.plant_id,p.policy_json FROM offline_policies p \
         JOIN actuator_bindings a ON a.plant_id=p.plant_id \
         WHERE a.device_id=? ORDER BY p.plant_id",
    )
    .bind(&device_id)
    .fetch_all(state.db.pool())
    .await
    .map_err(|_| PublishError::Storage)?;
    let mut policies = Vec::with_capacity(rows.len());
    for row in rows {
        let document: String = row.get("policy_json");
        policies.push(
            serde_json::from_str::<OfflinePolicy>(&document).map_err(|_| PublishError::Storage)?,
        );
    }
    let device = DeviceId::parse(&device_id).map_err(|_| PublishError::Storage)?;
    let payload = Envelope {
        v: rhizo_mqtt_contract::PROTOCOL_VERSION,
        kind: MessageKind::DevicePolicy,
        message_id: MessageId::from_uuid(uuid::Uuid::now_v7()),
        device_id: device.clone(),
        boot_id: None,
        sequence: None,
        device_time_ms: None,
        clock_synced: None,
        data: OfflinePolicySet { policies },
    }
    .to_json()
    .map_err(|_| PublishError::Storage)?;
    state
        .commander
        .transport()
        .publish(
            Topic::Policy(device).as_string(),
            payload.into_bytes(),
            true,
        )
        .await
        .map_err(|_| PublishError::Transport)?;
    // One statement, so a failure part-way cannot leave some plants of the same
    // retained document claiming publication and others not.
    sqlx::query(
        "UPDATE offline_policies SET published_at=? WHERE plant_id IN \
         (SELECT plant_id FROM actuator_bindings WHERE device_id=?)",
    )
    .bind(now)
    .bind(&device_id)
    .execute(state.db.pool())
    .await
    .map_err(|_| PublishError::Storage)?;
    Ok(())
}

/// The response a failed publication produces.
fn publish_error(failure: &PublishError) -> Response {
    match failure {
        PublishError::Storage => storage_error(),
        PublishError::Transport => error(
            StatusCode::SERVICE_UNAVAILABLE,
            "policy_publish_failed",
            "the policy was stored but could not be published to the device",
        ),
    }
}

fn row_json(row: &binding_repo::OfflinePolicyRow) -> serde_json::Value {
    serde_json::json!({
        "plant_id": row.plant_id,
        "policy_version": row.policy_version,
        "enabled": row.enabled,
        "policy": serde_json::from_str::<serde_json::Value>(&row.policy_json)
            .unwrap_or(serde_json::Value::Null),
        "published_at": row.published_at.and_then(timestamp),
        "applied_version": row.applied_version,
        "updated_at": timestamp(row.updated_at),
    })
}

pub async fn get(State(state): State<ApiState>, Path(id): Path<String>) -> Response {
    match binding_repo::offline_policy(&state.db, &id).await {
        Ok(Some(row)) => Json(row_json(&row)).into_response(),
        Ok(None) => Json(serde_json::json!({ "plant_id": id, "policy": null })).into_response(),
        Err(_) => storage_error(),
    }
}

/// Re-derives the policy from the plant's current configuration and stores it at
/// the next version.
///
/// There is no request body: an offline policy is not a separate set of numbers
/// an operator types, it is the plant's own configuration expressed in the form
/// a device can act on. A body would be a second place to configure the same
/// plant, and the two would drift.
pub async fn put(State(state): State<ApiState>, Path(id): Path<String>) -> Response {
    let loaded = match plant::load(&state.db, &id).await {
        Ok(Some(loaded)) => loaded,
        Ok(None) => return error(StatusCode::NOT_FOUND, "plant_not_found", "unknown plant"),
        Err(_) => return storage_error(),
    };
    let Ok(version) = binding_repo::next_policy_version(&state.db, &id).await else {
        return storage_error();
    };
    let bindings = loaded.bindings();
    let inputs = AuthoringInputs {
        plant_id: &id,
        bindings: &bindings,
        actuator: loaded.actuator.as_ref(),
        measurement_policies: &loaded.policies,
        profile: &loaded.profile,
        policy_version: u32::try_from(version).unwrap_or(u32::MAX),
    };
    let policy = match author(&inputs) {
        Ok(policy) => policy,
        Err(e) => {
            // SAFETY-018: a plant with no actuator is refused here rather than
            // being allowed to configure autonomy that could never run.
            return error_with(
                StatusCode::UNPROCESSABLE_ENTITY,
                e.code(),
                &e.to_string(),
                serde_json::json!({ "rule": e.code() }),
            );
        }
    };
    let Ok(document) = serde_json::to_string(&policy) else {
        return storage_error();
    };
    let now = state.clock.now().timestamp_millis();
    if binding_repo::upsert_offline_policy(&state.db, &id, version, false, &document, now)
        .await
        .is_err()
    {
        return storage_error();
    }
    if let Err(failure) = publish_for_plant(&state, &id, now).await {
        return publish_error(&failure);
    }
    let missing: Vec<String> = missing_safety_bindings(&bindings)
        .iter()
        .map(|k| k.as_str().to_owned())
        .collect();
    match binding_repo::offline_policy(&state.db, &id).await {
        Ok(Some(row)) => {
            let mut value = row_json(&row);
            if let Some(object) = value.as_object_mut() {
                // Advisory: the gate refuses at runtime anyway, but telling an
                // operator now that their policy can never fire beats letting
                // them discover it during a heatwave.
                object.insert(
                    "missing_required_bindings".to_owned(),
                    serde_json::json!(missing),
                );
            }
            (StatusCode::CREATED, Json(value)).into_response()
        }
        Ok(None) | Err(_) => storage_error(),
    }
}

async fn set_enabled(state: &ApiState, plant_id: &str, enabled: bool) -> Response {
    let Ok(Some(row)) = binding_repo::offline_policy(&state.db, plant_id).await else {
        return error(
            StatusCode::NOT_FOUND,
            "offline_policy_not_found",
            "author a policy before enabling it",
        );
    };
    let Ok(mut policy) =
        serde_json::from_str::<rhizo_mqtt_contract::payload::OfflinePolicy>(&row.policy_json)
    else {
        return storage_error();
    };
    // Enabling re-runs the shared validator against the stored numbers: a policy
    // authored before a firmware limit tightened must not slip through on the
    // strength of having been valid once.
    policy.enabled = enabled;
    policy.policy_version = u32::try_from(row.policy_version.saturating_add(1)).unwrap_or(u32::MAX);
    if enabled && let Err(e) = rhizo_policy::validate_authored(&policy) {
        return error_with(
            StatusCode::UNPROCESSABLE_ENTITY,
            "policy_rejected",
            &format!("the shared policy validator refused it: {e:?}"),
            serde_json::json!({}),
        );
    }
    let Ok(document) = serde_json::to_string(&policy) else {
        return storage_error();
    };
    let now = state.clock.now().timestamp_millis();
    if binding_repo::upsert_offline_policy(
        &state.db,
        plant_id,
        i64::from(policy.policy_version),
        enabled,
        &document,
        now,
    )
    .await
    .is_err()
    {
        return storage_error();
    }
    if let Err(failure) = publish_for_plant(state, plant_id, now).await {
        return publish_error(&failure);
    }
    match binding_repo::offline_policy(&state.db, plant_id).await {
        Ok(Some(row)) => Json(row_json(&row)).into_response(),
        Ok(None) | Err(_) => storage_error(),
    }
}

pub async fn enable(State(state): State<ApiState>, Path(id): Path<String>) -> Response {
    set_enabled(&state, &id, true).await
}

pub async fn disable(State(state): State<ApiState>, Path(id): Path<String>) -> Response {
    set_enabled(&state, &id, false).await
}

#[cfg(test)]
mod tests {
    use super::super::testsupport::TestApi;
    use axum::http::StatusCode;
    use rhizo_mqtt_contract::Envelope;
    use rhizo_mqtt_contract::payload::OfflinePolicySet;

    async fn configured(with_actuator: bool) -> TestApi {
        let api = TestApi::start().await;
        api.with_device().await;
        api.plant("monstera-01").await;
        api.bind_control("monstera-01").await;
        api.moisture_policy("monstera-01").await;
        if with_actuator {
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
                api.json(
                    "PUT",
                    "/api/v1/plants/monstera-01/bindings/sensors",
                    serde_json::json!({
                        "device_id": "plant-node-01",
                        "sensor_id": sensor_id,
                        "point": point,
                        "kind": kind,
                        "role": "required",
                    }),
                )
                .await;
                api.json(
                    "PUT",
                    &format!("/api/v1/plants/monstera-01/measurement-policies/{kind}"),
                    serde_json::json!({ "stale_after_ms": 600_000 }),
                )
                .await;
            }
        }
        api
    }

    #[tokio::test]
    async fn a_valid_policy_is_authored_versioned_and_disabled() {
        let api = configured(true).await;
        let (status, before) = api.get("/api/v1/plants/monstera-01/offline-policy").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(before["policy"], serde_json::Value::Null);

        let (status, authored) = api
            .json(
                "PUT",
                "/api/v1/plants/monstera-01/offline-policy",
                serde_json::json!({}),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{authored}");
        assert_eq!(authored["policy_version"], 1);
        assert_eq!(
            authored["enabled"], false,
            "authoring a policy is not authorising unsupervised watering"
        );
        assert_eq!(
            authored["policy"]["control_measurement"]["trigger_below"],
            28.0
        );
        assert_eq!(
            authored["policy"]["control_measurement"]["resume_above"],
            45.0
        );
        assert_eq!(
            authored["policy"]["required_measurements"]
                .as_array()
                .unwrap()
                .len(),
            2,
            "required measurements come from required-role bindings"
        );
        assert!(
            authored["policy"]["safety"]["require_leak_clear"]
                .as_bool()
                .unwrap()
        );
        assert_eq!(authored["missing_required_bindings"], serde_json::json!([]),);

        // Re-authoring allocates a strictly higher version.
        let (status, again) = api
            .json(
                "PUT",
                "/api/v1/plants/monstera-01/offline-policy",
                serde_json::json!({}),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{again}");
        assert_eq!(again["policy_version"], 2);
    }

    /// SAFETY-018: a plant with no actuator cannot have an offline policy at
    /// all — refused at authoring time, with a message that says why.
    #[tokio::test]
    async fn safety_018_automation_rejected_without_actuator() {
        let api = configured(false).await;
        let (status, refused) = api
            .json(
                "PUT",
                "/api/v1/plants/monstera-01/offline-policy",
                serde_json::json!({}),
            )
            .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{refused}");
        assert_eq!(refused["error"]["code"], "no_actuator_bound");
        assert!(
            refused["error"]["message"]
                .as_str()
                .unwrap()
                .contains("no actuator")
        );
    }

    #[tokio::test]
    async fn a_plant_with_no_control_binding_is_refused() {
        let api = TestApi::start().await;
        api.with_device().await;
        api.plant("monstera-01").await;
        api.json(
            "PUT",
            "/api/v1/plants/monstera-01/bindings/actuator",
            serde_json::json!({ "device_id": "plant-node-01", "actuator_id": "pump-0" }),
        )
        .await;
        let (status, refused) = api
            .json(
                "PUT",
                "/api/v1/plants/monstera-01/offline-policy",
                serde_json::json!({}),
            )
            .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{refused}");
        assert_eq!(refused["error"]["code"], "no_control_binding");
    }

    /// Rejected, never clamped, and by the contract's own rule.
    #[tokio::test]
    async fn a_dose_above_the_firmware_limit_is_rejected_not_clamped() {
        let api = configured(true).await;
        api.json(
            "POST",
            "/api/v1/profiles",
            serde_json::json!({
                "profile_id": "wild", "name": "Wild",
                "target_min_vwc": 28.0, "target_max_vwc": 45.0,
                "dose_ml": 79.0, "max_doses_per_cycle": 6, "max_daily_ml": 500.0,
                "dry_confirm_minutes": 30, "cooldown_hours": 6.0, "absorption_minutes": 30,
            }),
        )
        .await;
        api.json(
            "PATCH",
            "/api/v1/plants/monstera-01",
            serde_json::json!({ "profile_id": "wild" }),
        )
        .await;
        // 79 x 6 = 474 ml, inside the 500 ml device budget: this one is valid.
        let (status, ok) = api
            .json(
                "PUT",
                "/api/v1/plants/monstera-01/offline-policy",
                serde_json::json!({}),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{ok}");

        // Now push the dose past the ceiling by editing the profile directly in
        // storage, which is the only way to get an invalid value this far.
        let document = serde_json::to_string(&rhizo_domain::profile::PlantProfile {
            profile_id: rhizo_domain::ProfileId::from_uuid(uuid::Uuid::nil()),
            name: "Wild".into(),
            target_min_vwc: 28.0,
            target_max_vwc: 45.0,
            dose_ml: 200.0,
            max_doses_per_cycle: 1,
            max_daily_ml: 500.0,
            dry_confirm_minutes: 30,
            cooldown_hours: 6.0,
            absorption_minutes: 30,
            recovery_delta_vwc: 6.0,
            tank_min_percent: 15.0,
            command_ttl_seconds: 120,
        })
        .unwrap();
        rhizo_storage::repo::profile::upsert(&api.db, "wild", "Wild", &document, 1)
            .await
            .unwrap();
        let (status, refused) = api
            .json(
                "PUT",
                "/api/v1/plants/monstera-01/offline-policy",
                serde_json::json!({}),
            )
            .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{refused}");
        assert_eq!(refused["error"]["code"], "policy_rejected");
        assert!(
            refused["error"]["message"]
                .as_str()
                .unwrap()
                .contains("DoseAboveHardLimit"),
            "the refusal is the shared validator's own value: {refused}"
        );
    }

    #[tokio::test]
    async fn enabling_and_disabling_are_separate_decisions() {
        let api = configured(true).await;
        api.json(
            "PUT",
            "/api/v1/plants/monstera-01/offline-policy",
            serde_json::json!({}),
        )
        .await;

        let (status, enabled) = api
            .json(
                "POST",
                "/api/v1/plants/monstera-01/offline-policy/enable",
                serde_json::json!({}),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{enabled}");
        assert_eq!(enabled["enabled"], true);
        assert_eq!(
            enabled["policy_version"], 2,
            "a change of activation is a new version, per protocol section 5.11"
        );

        let (status, disabled) = api
            .json(
                "POST",
                "/api/v1/plants/monstera-01/offline-policy/disable",
                serde_json::json!({}),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{disabled}");
        assert_eq!(disabled["enabled"], false);
        assert_eq!(disabled["policy_version"], 3);
    }

    #[tokio::test]
    async fn enabling_a_policy_that_was_never_authored_is_refused() {
        let api = configured(true).await;
        let (status, refused) = api
            .json(
                "POST",
                "/api/v1/plants/monstera-01/offline-policy/enable",
                serde_json::json!({}),
            )
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{refused}");
    }

    /// The assembled Edge publishes the authored policy retained; authoring is
    /// still not authorisation because the document is explicitly disabled.
    #[tokio::test]
    async fn authoring_publishes_a_disabled_policy_retained() {
        let api = configured(true).await;
        let (_, authored) = api
            .json(
                "PUT",
                "/api/v1/plants/monstera-01/offline-policy",
                serde_json::json!({}),
            )
            .await;
        assert!(authored["published_at"].is_string());
        assert_eq!(authored["applied_version"], serde_json::Value::Null);
        let published = api.transport.published();
        assert_eq!(published.len(), 1);
        assert_eq!(published[0].topic, "rhizo/v1/devices/plant-node-01/policy");
        assert!(published[0].retain);
        let envelope = Envelope::<OfflinePolicySet>::from_json(&published[0].payload).unwrap();
        assert_eq!(envelope.data.policies.len(), 1);
        assert!(!envelope.data.policies[0].enabled);
    }
}
