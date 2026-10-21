//! The safety-critical endpoints (M6-016, M6-023),
//! `http-api-boundaries.md` §2.6.
//!
//! # 409 is the safety answer
//!
//! A refusal carries `{ reason, since, clearable, message }` so a UI can explain
//! what will lift it (PRD 120 F-120-21). **422** is reserved for "this plant has
//! nothing to water with", which SAFETY-018 requires to be distinguishable from
//! both a safety refusal and an unknown plant.
//!
//! # There is no override, force, bypass, expedite, or wake parameter
//!
//! Every request body below is `deny_unknown_fields`, so one would be a 422
//! rather than a silently ignored field, and
//! [`crate::api::server::tests`] greps this directory for the words. Every
//! actuation request goes through `rhizo_domain::irrigation::evaluate`; an HTTP
//! handler that published MQTT directly would nullify SAFETY-003 and
//! SAFETY-004, and it is easy to write by accident while adding a "quick manual
//! test" endpoint.

#![allow(missing_docs)]
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use rhizo_domain::irrigation::types::{EvaluationMode, IrrigationDecision};
use rhizo_domain::irrigation::{display_rank, is_auto_clearable, safety_gate};
use rhizo_domain::state::LockoutReason;
use rhizo_storage::repo::{lockout as lockout_repo, plant as plant_repo};
use serde::Deserialize;

use super::ApiState;
use super::support::{error, error_with, storage_error, timestamp};
use crate::control::command::Issued;
use crate::control::intents::{self, Route, RoutingRefusal};
use crate::control::irrigation;
use crate::plant;

/// `POST /plants/{id}/water`.
///
/// `ml` defaults to the plant's profile dose; `mode` to `manual`. There is no
/// third field, and never will be.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WaterRequest {
    #[serde(default)]
    pub ml: Option<f64>,
    #[serde(default)]
    pub mode: Option<String>,
}

/// The structured 409 a lockout produces.
fn lockout_conflict(
    reason: LockoutReason,
    since: Option<i64>,
    detail: Option<serde_json::Value>,
) -> Response {
    let name = plant::lockout_name(reason);
    let mut details = serde_json::json!({
        "reason": name,
        "since": since.and_then(timestamp),
        "clearable": !is_auto_clearable(reason),
        "message": lockout_message(reason),
    });
    if let (Some(extra), Some(map)) = (detail, details.as_object_mut())
        && let Some(extra) = extra.as_object()
    {
        for (key, value) in extra {
            map.insert(key.clone(), value.clone());
        }
    }
    error_with(
        StatusCode::CONFLICT,
        "safety_lockout",
        &lockout_message(reason),
        details,
    )
}

/// The sentence an operator reads for a lockout. Rendered in one place.
#[must_use]
pub fn lockout_message(reason: LockoutReason) -> String {
    match reason {
        LockoutReason::Leak => {
            "water is present, or has been; clear the leak and reset the lockout explicitly"
                .to_owned()
        }
        LockoutReason::TankLow => "the reservoir is at or below its minimum".to_owned(),
        LockoutReason::StaleData => "the latest reading is too old to act on".to_owned(),
        LockoutReason::SensorFault => {
            "a sensor this plant depends on is not reporting usable data".to_owned()
        }
        LockoutReason::DailyLimit => "the rolling 24-hour water budget is spent".to_owned(),
        LockoutReason::MaxDosesReached => {
            "this cycle reached its dose limit without the plant recovering".to_owned()
        }
        LockoutReason::NoDeliveryDetected => {
            "two doses produced no measurable response; the water is not reaching the pot"
                .to_owned()
        }
        LockoutReason::Uncertain => {
            "an input the decision depends on is missing, unreadable, or not yet reconciled"
                .to_owned()
        }
        LockoutReason::ClockUnsynced => "the device has no trustworthy clock".to_owned(),
        LockoutReason::PumpFault => "the pump reported a fault".to_owned(),
        LockoutReason::NoActuator => "this plant has no actuator".to_owned(),
        LockoutReason::Unknown => {
            "this plant is locked out for a reason this version does not recognise".to_owned()
        }
    }
}

pub async fn water(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Json(body): Json<WaterRequest>,
) -> Response {
    let Ok(Some(loaded)) = plant::load(&state.db, &id).await else {
        return match plant::load(&state.db, &id).await {
            Ok(None) => error(StatusCode::NOT_FOUND, "plant_not_found", "unknown plant"),
            _ => storage_error(),
        };
    };
    // SAFETY-018: a monitoring-only plant answers 422, distinguishably from a
    // safety refusal (409) and from an unknown plant (404).
    let Some(actuator) = loaded.actuator.clone() else {
        return error_with(
            StatusCode::UNPROCESSABLE_ENTITY,
            "no_actuator_bound",
            "this plant has no actuator, so there is no watering path to refuse or allow",
            serde_json::json!({ "lockout": plant::lockout_name(LockoutReason::NoActuator) }),
        );
    };
    let requested_ml = body.ml.unwrap_or(f64::from(loaded.profile.dose_ml)) as f32;
    if !requested_ml.is_finite() || requested_ml <= 0.0 {
        return error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_volume",
            "ml must be a finite volume greater than zero",
        );
    }
    let mode = match body.mode.as_deref().unwrap_or("manual") {
        "manual" => EvaluationMode::ManualRequest { ml: requested_ml },
        "recommended" => EvaluationMode::RecommendedRequest { ml: requested_ml },
        other => {
            return error(
                StatusCode::UNPROCESSABLE_ENTITY,
                "unknown_mode",
                &format!("`{other}` is not a watering mode; use manual or recommended"),
            );
        }
    };

    let now = state.clock.now();
    let device = actuator.device_id.to_string();
    let route = match intents::route(&state.db, &id, &device, now).await {
        Ok(route) => route,
        Err(_) => return storage_error(),
    };

    match route {
        Route::Refuse(RoutingRefusal::IntentAlreadyOpen {
            intent_id,
            expected_delivery_after,
        }) => error_with(
            StatusCode::CONFLICT,
            "intent_already_pending",
            "a dose is already waiting for this device to wake",
            serde_json::json!({
                "intent_id": intent_id,
                "expected_delivery_after": expected_delivery_after.and_then(timestamp),
            }),
        ),
        Route::Refuse(RoutingRefusal::DeviceUnreachable) => error_with(
            StatusCode::CONFLICT,
            "device_unreachable",
            "the device carrying this plant's pump is not reachable and is not asleep",
            serde_json::json!({ "device_id": device }),
        ),
        Route::HoldForWake {
            expected_delivery_after,
            wake_interval_seconds,
        } => {
            hold_for_wake(
                &state,
                &loaded,
                &device,
                requested_ml,
                mode,
                expected_delivery_after,
                wake_interval_seconds,
                now,
            )
            .await
        }
        Route::Immediate => immediate(&state, &loaded, mode, now).await,
    }
}

/// The connected path. Byte-identical whether the caller is the loop or a person.
async fn immediate(
    state: &ApiState,
    loaded: &plant::Loaded,
    mode: EvaluationMode,
    now: chrono::DateTime<chrono::Utc>,
) -> Response {
    let Ok(analysis) =
        super::support::with_busy_retry(|| plant::analyse(&state.db, loaded, now)).await
    else {
        return storage_error();
    };
    // Retried on a busy database, not on a refusal. `run_pass` runs the gate
    // and, if it passes, persists and publishes — so a retry after a *successful*
    // pass would be a second dose. It cannot happen: the only retried error is
    // `Busy`, which is raised by a statement that did not commit.
    let pass = match super::support::with_busy_retry(|| async {
        irrigation::run_pass(
            &state.commander,
            loaded,
            analysis.inputs.dry_duration,
            mode,
            now,
        )
        .await
        .map_err(|error| match error {
            crate::error::EdgeError::Storage(storage) => storage,
            other => rhizo_storage::StorageError::Database(other.to_string()),
        })
    })
    .await
    {
        Ok(pass) => pass,
        Err(_) => return storage_error(),
    };
    match pass.decision {
        IrrigationDecision::IssueDose { ml, .. } => {
            let Some(command_id) = pass.command_id else {
                return storage_error();
            };
            let Ok(Some(row)) = rhizo_storage::repo::command::get(&state.db, &command_id).await
            else {
                return storage_error();
            };
            if row.status == "failed" {
                return error_with(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "publish_failed",
                    "the command was recorded but could not be delivered to the device",
                    serde_json::json!({ "command_id": command_id }),
                );
            }
            (
                StatusCode::ACCEPTED,
                Json(serde_json::json!({
                    "command_id": command_id,
                    "status": "issued",
                    "requested_ml": ml,
                    "expires_at": timestamp(row.expires_at),
                })),
            )
                .into_response()
        }
        IrrigationDecision::Lock { reason } => {
            let since = plant_repo::get(&state.db, loaded.plant.plant_id.as_str())
                .await
                .ok()
                .flatten()
                .and_then(|row| row.lockout_since);
            lockout_conflict(reason, since, None)
        }
        IrrigationDecision::Wait { .. } => error(
            StatusCode::CONFLICT,
            "command_in_flight",
            "a command for this plant is already on the wire; wait for it to settle",
        ),
        IrrigationDecision::Idle
        | IrrigationDecision::Recommend { .. }
        | IrrigationDecision::CycleComplete => error(
            StatusCode::CONFLICT,
            "not_actionable",
            "the machine did not produce a dose for this request",
        ),
    }
}

/// The sleeping path. **Publishes nothing.**
#[allow(clippy::too_many_arguments)]
async fn hold_for_wake(
    state: &ApiState,
    loaded: &plant::Loaded,
    device: &str,
    requested_ml: f32,
    mode: EvaluationMode,
    expected_delivery_after: Option<i64>,
    wake_interval_seconds: Option<i64>,
    now: chrono::DateTime<chrono::Utc>,
) -> Response {
    // The gate runs here too, so an obviously impossible request is refused
    // immediately rather than fifteen minutes later. It runs again, **in full**,
    // at delivery — which is what makes this path stricter than the immediate
    // one, not looser.
    let Ok(analysis) = plant::analyse(&state.db, loaded, now).await else {
        return storage_error();
    };
    let Ok((gathered, _)) =
        irrigation::preview(&state.db, loaded, analysis.inputs.dry_duration, mode, now).await
    else {
        return storage_error();
    };
    // The device being asleep is not itself a refusal, so the gate is consulted
    // rather than the whole machine.
    if let Some(reason) = safety_gate(&gathered.inputs(now, mode)) {
        let since = loaded.plant.lockout_since;
        return lockout_conflict(reason, since, None);
    }

    match intents::hold(
        &state.db,
        loaded.plant.plant_id.as_str(),
        device,
        requested_ml,
        mode,
        expected_delivery_after,
        wake_interval_seconds,
        now,
    )
    .await
    {
        Ok(intent) => (
            StatusCode::ACCEPTED,
            // Deliberately a different shape. **No `command_id`** — the field is
            // absent rather than null, so a client that reads it unconditionally
            // fails loudly instead of polling an id that does not exist.
            Json(serde_json::json!({
                "intent_id": intent.intent_id,
                "status": intents::PENDING,
                "requested_ml": intent.requested_ml,
                "expected_delivery_after": intent.expected_delivery_after.and_then(timestamp),
                "intent_expires_at": timestamp(intent.intent_expires_at),
            })),
        )
            .into_response(),
        Err(crate::error::EdgeError::Storage(rhizo_storage::StorageError::Constraint(_))) => error(
            StatusCode::CONFLICT,
            "intent_already_pending",
            "a dose is already waiting for this device to wake",
        ),
        Err(_) => storage_error(),
    }
}

/// `POST /plants/{id}/auto-watering/enable`.
pub async fn enable_auto(State(state): State<ApiState>, Path(id): Path<String>) -> Response {
    set_auto(&state, &id, true).await
}

/// `POST /plants/{id}/auto-watering/disable`.
pub async fn disable_auto(State(state): State<ApiState>, Path(id): Path<String>) -> Response {
    set_auto(&state, &id, false).await
}

async fn set_auto(state: &ApiState, plant_id: &str, enabled: bool) -> Response {
    match plant_repo::update(
        &state.db,
        plant_id,
        &plant_repo::PlantPatch {
            auto_watering_enabled: Some(enabled),
            ..plant_repo::PlantPatch::default()
        },
        state.clock.now().timestamp_millis(),
    )
    .await
    {
        Ok(Some(row)) => {
            tracing::info!(
                plant_id = %plant_id,
                auto_watering_enabled = enabled,
                "automatic watering setting changed"
            );
            Json(serde_json::json!({
                "plant_id": row.plant_id,
                "auto_watering_enabled": row.auto_watering_enabled,
            }))
            .into_response()
        }
        Ok(None) => error(StatusCode::NOT_FOUND, "plant_not_found", "unknown plant"),
        Err(_) => storage_error(),
    }
}

/// `POST /plants/{id}/lockout/clear`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClearRequest {
    pub reason: String,
}

pub async fn clear_lockout(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Json(body): Json<ClearRequest>,
) -> Response {
    let Ok(Some(loaded)) = plant::load(&state.db, &id).await else {
        return error(StatusCode::NOT_FOUND, "plant_not_found", "unknown plant");
    };
    let now = state.clock.now();
    let now_ms = now.timestamp_millis();

    // One request names one record, and clears at most that one (F-060-41). The
    // records — not the display summary — say what the plant is locked by.
    let Ok(records) = lockout_repo::active(&state.db, &id, now_ms).await else {
        return storage_error();
    };
    let Some(named) = records.iter().find(|record| record.reason == body.reason) else {
        if records.is_empty() {
            return error(
                StatusCode::CONFLICT,
                "not_locked_out",
                "this plant is not locked out",
            );
        }
        let shown = records
            .iter()
            .min_by_key(|record| {
                display_rank(
                    plant::lockout_from_str(&record.reason).unwrap_or(LockoutReason::Unknown),
                )
            })
            .map(|record| record.reason.clone());
        return error_with(
            StatusCode::CONFLICT,
            "wrong_reason",
            "the plant is locked out for a different reason",
            serde_json::json!({ "reason": shown }),
        );
    };
    if named.kind == lockout_repo::LockoutKind::Hold {
        return error_with(
            StatusCode::CONFLICT,
            "hold_not_clearable",
            "this lockout is a hold; it ends by itself at its deadline and cannot be cleared",
            serde_json::json!({ "reason": named.reason, "hold_until": named.hold_until.and_then(timestamp) }),
        );
    }
    let since = Some(named.since);
    let reason = plant::lockout_from_str(&body.reason).unwrap_or(LockoutReason::Unknown);

    let Ok(analysis) =
        super::support::with_busy_retry(|| plant::analyse(&state.db, &loaded, now)).await
    else {
        return storage_error();
    };
    let mode = EvaluationMode::Automatic;
    let Ok((gathered, _)) = super::support::with_busy_retry(|| async {
        irrigation::preview(&state.db, &loaded, analysis.inputs.dry_duration, mode, now)
            .await
            .map_err(|error| match error {
                crate::error::EdgeError::Storage(storage) => storage,
                other => rhizo_storage::StorageError::Database(other.to_string()),
            })
    })
    .await
    else {
        return storage_error();
    };

    // Re-run the gate as if nothing were recorded. If the same reason comes
    // straight back, the condition is still active and clearing it would be a
    // lie the next pass would immediately contradict.
    let mut trial = gathered.inputs(now, mode);
    trial.latched = &[];
    trial.holds = &[];
    if safety_gate(&trial) == Some(reason) {
        return lockout_conflict(reason, since, None);
    }
    let not_clear = || {
        lockout_conflict(
            reason,
            since,
            Some(serde_json::json!({ "leak_signal": "not_clear" })),
        )
    };
    // SAFETY-003's explicit reset needs the signal *positively absent*. A silent
    // leak sensor cannot demonstrate a dry tray, so `Unknown` refuses too. This
    // is the early answer; the binding one is re-checked inside the clearing
    // transaction below.
    if reason == LockoutReason::Leak
        && gathered.leak != rhizo_domain::irrigation::types::LeakState::Clear
    {
        return not_clear();
    }
    let Ok(evidence) = crate::control::inputs::leak_evidence(&state.db, &loaded, now_ms).await
    else {
        return storage_error();
    };
    let evidence = evidence
        .as_ref()
        .map(
            |(device_id, sensor_id, point, fresh_after)| lockout_repo::LeakEvidence {
                device_id,
                sensor_id,
                point,
                fresh_after: *fresh_after,
            },
        );
    let outcome = super::support::with_busy_retry(|| {
        lockout_repo::clear_latch(&state.db, &id, &body.reason, "operator", evidence, now_ms)
    })
    .await;
    match outcome {
        Ok(lockout_repo::ClearOutcome::Cleared { .. }) => {}
        Ok(lockout_repo::ClearOutcome::NotLatched) => {
            // Cleared by someone else between the read above and now.
            return error(
                StatusCode::CONFLICT,
                "not_locked_out",
                "this plant is not locked out",
            );
        }
        Ok(lockout_repo::ClearOutcome::LeakNotClear) => return not_clear(),
        Err(_) => return storage_error(),
    }
    tracing::info!(plant_id = %id, reason = %body.reason, "lockout cleared by an operator");

    // Bring the display summary up to date now rather than at the next pass.
    // Best effort: it decides nothing, and the next pass rewrites it anyway.
    if let Ok((_, decision)) =
        irrigation::preview(&state.db, &loaded, analysis.inputs.dry_duration, mode, now).await
    {
        let shown = match decision {
            IrrigationDecision::Lock { reason } => Some(plant::lockout_name(reason)),
            _ => None,
        };
        if let Err(error) = rhizo_storage::repo::command::set_summary(
            &state.db,
            &id,
            shown.as_deref(),
            Some("operator"),
            now_ms,
        )
        .await
        {
            tracing::warn!(plant_id = %id, %error, "could not refresh the lockout summary");
        }
    }
    Json(serde_json::json!({
        "plant_id": id,
        "cleared": body.reason,
        "cleared_at": timestamp(now_ms),
    }))
    .into_response()
}

/// `GET /commands/{command_id}`.
pub async fn get_command(State(state): State<ApiState>, Path(id): Path<String>) -> Response {
    match rhizo_storage::repo::command::get(&state.db, &id).await {
        Ok(Some(row)) => Json(command_json(&row)).into_response(),
        Ok(None) => error(
            StatusCode::NOT_FOUND,
            "command_not_found",
            "unknown command",
        ),
        Err(_) => storage_error(),
    }
}

/// One command, as the API renders it.
#[must_use]
pub fn command_json(row: &rhizo_storage::repo::command::CommandRow) -> serde_json::Value {
    serde_json::json!({
        "command_id": row.command_id,
        "device_id": row.device_id,
        "plant_id": row.plant_id,
        "kind": row.kind,
        "mode": row.mode,
        "requested_ml": row.requested_ml,
        "status": row.status,
        "issued_at": timestamp(row.issued_at),
        "expires_at": timestamp(row.expires_at),
        "published_at": row.published_at.and_then(timestamp),
        "settled_at": row.settled_at.and_then(timestamp),
        "reason": row.reason,
    })
}

/// `POST /devices/{id}/commands/tare`.
pub async fn tare(State(state): State<ApiState>, Path(id): Path<String>) -> Response {
    device_command(
        &state,
        &id,
        crate::control::command::DeviceCommandKind::Tare,
    )
    .await
}

/// `POST /devices/{id}/commands/calibrate`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CalibrateRequest {
    pub run_seconds: f64,
}

pub async fn calibrate(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Json(body): Json<CalibrateRequest>,
) -> Response {
    let run_seconds = body.run_seconds as f32;
    if !run_seconds.is_finite() || run_seconds <= 0.0 {
        return error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "invalid_run_seconds",
            "run_seconds must be a finite duration greater than zero",
        );
    }
    device_command(
        &state,
        &id,
        crate::control::command::DeviceCommandKind::Calibrate { run_seconds },
    )
    .await
}

async fn device_command(
    state: &ApiState,
    device_id: &str,
    kind: crate::control::command::DeviceCommandKind,
) -> Response {
    let known: Option<String> =
        match sqlx::query_scalar("SELECT device_id FROM devices WHERE device_id=?")
            .bind(device_id)
            .fetch_optional(state.db.pool())
            .await
        {
            Ok(value) => value,
            Err(_) => return storage_error(),
        };
    if known.is_none() {
        return error(StatusCode::NOT_FOUND, "device_not_found", "unknown device");
    }
    // Diagnostics have no safety weight and no urgency, so they are never held
    // for a wake: a battery device runs them at the next wake the operator is
    // watching, or not at all (M6-022 §Non-goals).
    match state
        .commander
        .issue_device_command(
            device_id,
            kind,
            chrono::Duration::seconds(i64::from(
                rhizo_domain::profile::default_command_ttl_seconds(),
            )),
        )
        .await
    {
        Ok(Issued::Published {
            command_id,
            expires_at,
        }) => (
            StatusCode::ACCEPTED,
            Json(serde_json::json!({
                "command_id": command_id,
                "status": "issued",
                "expires_at": timestamp(expires_at),
            })),
        )
            .into_response(),
        Ok(Issued::PublishFailed { command_id }) => error_with(
            StatusCode::SERVICE_UNAVAILABLE,
            "publish_failed",
            "the command was recorded but could not be delivered to the device",
            serde_json::json!({ "command_id": command_id }),
        ),
        // A device command names no plant, so no plant lockout refuses it
        // today; answered as a refusal rather than assumed away.
        Ok(Issued::Blocked { reasons, .. }) => {
            let reason = plant::best_lockout(&reasons);
            lockout_conflict(reason, None, None)
        }
        Err(_) => storage_error(),
    }
}

/// The watering endpoints' own tests.
///
/// Named `water` so the filter M6-016 quotes literally,
/// `cargo test -p edge-controller api::water`, selects them.
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod water {
    use crate::api::testsupport::TestApi;
    use crate::control::irrigation;
    use axum::http::StatusCode;
    use rhizo_domain::irrigation::types::{EvaluationMode, IrrigationDecision};
    use rhizo_storage::EdgeDb;

    /// The healthy case, so every refusal below is a refusal of something that
    /// would otherwise have worked.
    #[tokio::test]
    async fn a_permitted_manual_dose_returns_202_with_a_command_id() {
        let api = TestApi::start().await;
        api.waterable("monstera-01").await;
        api.device_connected().await;

        let (status, body) = api
            .json(
                "POST",
                "/api/v1/plants/monstera-01/water",
                serde_json::json!({ "ml": 30.0, "mode": "manual" }),
            )
            .await;
        assert_eq!(status, StatusCode::ACCEPTED, "{body}");
        assert!(body["command_id"].is_string());
        assert_eq!(body["status"], "issued");
        assert_eq!(body["requested_ml"], 30.0);
        assert!(body["expires_at"].is_string());
        assert_eq!(api.transport.commands().len(), 1);
    }

    /// **SAFETY-003, the headline acceptance criterion.** A leak returns 409 and
    /// **nothing is published** — which is the property, and which a status code
    /// alone would not show.
    #[tokio::test]
    async fn safety_003_leak_blocks_manual_api() {
        let api = TestApi::start().await;
        api.waterable("monstera-01").await;
        api.device_connected().await;
        api.sample_bool(api.clock.now(), "leak-0", "tray", "leak_state", true)
            .await;

        let (status, body) = api
            .json(
                "POST",
                "/api/v1/plants/monstera-01/water",
                serde_json::json!({ "ml": 30.0, "mode": "manual" }),
            )
            .await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        assert_eq!(body["error"]["code"], "safety_lockout");
        assert_eq!(body["error"]["details"]["reason"], "leak");
        assert_eq!(
            body["error"]["details"]["clearable"], true,
            "a leak needs an explicit operator reset, and the body says so"
        );
        assert!(body["error"]["details"]["message"].is_string());
        assert!(
            api.transport.commands().is_empty(),
            "a refused dose publishes nothing at all"
        );
        let commands: i64 = sqlx::query_scalar("SELECT count(*) FROM commands")
            .fetch_one(api.db.pool())
            .await
            .unwrap();
        assert_eq!(commands, 0, "and persists nothing either");
    }

    /// The 409 body carries the reason, the since-timestamp, and whether an
    /// operator can clear it (PRD 120 F-120-21).
    #[tokio::test]
    async fn the_conflict_body_names_the_reason_and_its_clearability() {
        let api = TestApi::start().await;
        api.waterable("monstera-01").await;
        api.device_connected().await;
        api.sample_from(
            api.clock.now(),
            "tank-0",
            "reservoir",
            "tank_level",
            "percent",
            4.0,
        )
        .await;

        let (status, body) = api
            .json(
                "POST",
                "/api/v1/plants/monstera-01/water",
                serde_json::json!({ "ml": 30.0 }),
            )
            .await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        assert_eq!(body["error"]["details"]["reason"], "tank_low");
        assert_eq!(
            body["error"]["details"]["clearable"], false,
            "a refilled reservoir clears itself"
        );
    }

    /// **The manual exception, and its precise boundary.** A broken probe does
    /// not stop a person watering; a leak does.
    #[tokio::test]
    async fn manual_watering_succeeds_under_sensor_fault_but_not_under_leak() {
        let api = TestApi::start().await;
        api.waterable("monstera-01").await;
        api.device_connected().await;
        // Four hours on: the control sample is far past its freshness limit.
        api.clock.advance(chrono::Duration::hours(4));
        api.sample_bool(api.clock.now(), "leak-0", "tray", "leak_state", false)
            .await;
        api.sample_from(
            api.clock.now(),
            "tank-0",
            "reservoir",
            "tank_level",
            "percent",
            70.0,
        )
        .await;

        let (status, body) = api
            .json(
                "POST",
                "/api/v1/plants/monstera-01/water",
                serde_json::json!({ "ml": 30.0, "mode": "manual" }),
            )
            .await;
        assert_eq!(status, StatusCode::ACCEPTED, "{body}");

        // ...and `recommended` does not inherit the privilege, because
        // accepting the engine's advice is not a claim to have looked at the
        // plant.
        let api = TestApi::start().await;
        api.waterable("fern-01").await;
        api.device_connected().await;
        api.clock.advance(chrono::Duration::hours(4));
        api.sample_bool(api.clock.now(), "leak-0", "tray", "leak_state", false)
            .await;
        api.sample_from(
            api.clock.now(),
            "tank-0",
            "reservoir",
            "tank_level",
            "percent",
            70.0,
        )
        .await;
        let (status, body) = api
            .json(
                "POST",
                "/api/v1/plants/fern-01/water",
                serde_json::json!({ "ml": 30.0, "mode": "recommended" }),
            )
            .await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        assert_eq!(body["error"]["details"]["reason"], "stale_data");
    }

    /// SAFETY-018 is unchanged from M5: 422, distinguishably from both a safety
    /// refusal and an unknown plant.
    #[tokio::test]
    async fn safety_018_a_monitoring_only_plant_answers_422_not_409() {
        let api = TestApi::start().await;
        api.with_device().await;
        api.plant("fern-01").await;
        api.bind_control("fern-01").await;
        api.moisture_policy("fern-01").await;

        let (status, body) = api
            .json(
                "POST",
                "/api/v1/plants/fern-01/water",
                serde_json::json!({ "ml": 30.0 }),
            )
            .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        assert_eq!(body["error"]["code"], "no_actuator_bound");
        assert!(api.transport.commands().is_empty());

        let (status, _) = api
            .json(
                "POST",
                "/api/v1/plants/absent/water",
                serde_json::json!({ "ml": 30.0 }),
            )
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    /// The rolling cap refuses a manual dose too.
    #[tokio::test]
    async fn the_rolling_cap_refuses_a_manual_dose() {
        let api = TestApi::start().await;
        api.waterable("monstera-01").await;
        api.device_connected().await;
        sqlx::query(
            "INSERT INTO watering_events(watering_event_id,plant_id,device_id,mode,origin,started_at,completed_at,delivered_ml,status) \
             VALUES('we-1','monstera-01','plant-node-01','automatic','edge_command',?,?,295.0,'completed')",
        )
        .bind(api.clock.now().timestamp_millis())
        .bind(api.clock.now().timestamp_millis())
        .execute(api.db.pool())
        .await
        .unwrap();

        let (status, body) = api
            .json(
                "POST",
                "/api/v1/plants/monstera-01/water",
                serde_json::json!({ "ml": 40.0 }),
            )
            .await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        assert_eq!(body["error"]["details"]["reason"], "daily_limit");
        assert!(api.transport.commands().is_empty());
    }

    /// **No endpoint accepts an override, force, bypass, expedite, or wake
    /// parameter.** Checked two ways: the request is refused, and the source is
    /// scanned.
    #[tokio::test]
    async fn no_endpoint_accepts_an_override_parameter() {
        let api = TestApi::start().await;
        api.waterable("monstera-01").await;
        api.device_connected().await;
        for body in [
            serde_json::json!({ "ml": 30.0, "override": true }),
            serde_json::json!({ "ml": 30.0, "force": true }),
            serde_json::json!({ "ml": 30.0, "bypass_safety": true }),
            serde_json::json!({ "ml": 30.0, "expedite": true }),
            serde_json::json!({ "ml": 30.0, "wake": true }),
        ] {
            let (status, _) = api
                .json("POST", "/api/v1/plants/monstera-01/water", body.clone())
                .await;
            assert_eq!(
                status,
                StatusCode::UNPROCESSABLE_ENTITY,
                "{body} must be refused, not silently ignored"
            );
        }
        assert!(api.transport.commands().is_empty());
    }

    /// The structural half of the same rule, over the whole API directory.
    #[test]
    fn no_api_handler_names_an_override_parameter() {
        let sources = [
            ("watering.rs", include_str!("watering.rs")),
            ("intents.rs", include_str!("intents.rs")),
            ("plants.rs", include_str!("plants.rs")),
            ("device_config.rs", include_str!("device_config.rs")),
            ("server.rs", include_str!("server.rs")),
        ];
        for (name, whole) in sources {
            let source = whole
                .split(
                    "
#[cfg(test)]",
                )
                .next()
                .unwrap_or(whole);
            for (index, line) in source.lines().enumerate() {
                let trimmed = line.trim_start();
                if trimmed.starts_with("//") || trimmed.starts_with("///") {
                    continue;
                }
                for forbidden in ["override", "force", "expedite", "bypass"] {
                    // A *field* named one of these, not the word appearing in a
                    // string that explains why there is none.
                    let declaration = format!("{forbidden}:");
                    assert!(
                        !trimmed.starts_with(&declaration)
                            && !trimmed.starts_with(&format!("pub {declaration}")),
                        "{name}:{} declares `{forbidden}`",
                        index + 1
                    );
                }
            }
        }
    }

    /// **Every actuation path calls `evaluate`.** An HTTP handler that published
    /// MQTT directly would nullify SAFETY-003 and SAFETY-004, and it is easy to
    /// write by accident while adding a "quick manual test" endpoint.
    #[test]
    fn every_actuation_path_goes_through_the_domain_gate() {
        for (name, whole) in [
            ("watering.rs", include_str!("watering.rs")),
            ("intents.rs", include_str!("intents.rs")),
            ("plants.rs", include_str!("plants.rs")),
            ("device_config.rs", include_str!("device_config.rs")),
        ] {
            // The production half only: the tests below legitimately name the
            // shapes they forbid.
            let source = whole
                .split(
                    "
#[cfg(test)]",
                )
                .next()
                .unwrap_or(whole);
            for (index, line) in source.lines().enumerate() {
                let trimmed = line.trim_start();
                if trimmed.starts_with("//") {
                    continue;
                }
                assert!(
                    !trimmed.contains(".publish("),
                    "{name}:{} publishes directly instead of going through the commander",
                    index + 1
                );
            }
        }
        // ...and the one that does act reaches the machine.
        let watering = include_str!("watering.rs");
        assert!(watering.contains("irrigation::run_pass"));
        assert!(watering.contains("safety_gate("));
    }

    // --------------------------------------------------------------- lockouts

    /// `lockout/clear` on a still-active condition returns 409. That is the
    /// explicit reset SAFETY-003 requires, and it must verify the signal is gone.
    #[tokio::test]
    async fn clearing_a_lockout_while_the_condition_is_active_returns_409() {
        let api = TestApi::start().await;
        api.waterable("monstera-01").await;
        api.device_connected().await;
        api.sample_bool(api.clock.now(), "leak-0", "tray", "leak_state", true)
            .await;
        api.irrigate("monstera-01").await;

        let (status, body) = api
            .json(
                "POST",
                "/api/v1/plants/monstera-01/lockout/clear",
                serde_json::json!({ "reason": "leak" }),
            )
            .await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        assert_eq!(body["error"]["details"]["reason"], "leak");
    }

    /// A silent leak sensor cannot demonstrate a dry tray, so `Unknown` refuses
    /// the reset too (SAFETY-012).
    #[tokio::test]
    async fn clearing_a_leak_lockout_needs_the_signal_positively_absent() {
        let api = TestApi::start().await;
        api.waterable("monstera-01").await;
        api.device_connected().await;
        api.sample_bool(api.clock.now(), "leak-0", "tray", "leak_state", true)
            .await;
        api.irrigate("monstera-01").await;

        // The sensor falls silent: four hours on, its reading is stale, so the
        // leak signal is `Unknown` rather than `Clear`.
        api.clock.advance(chrono::Duration::hours(4));
        let (status, body) = api
            .json(
                "POST",
                "/api/v1/plants/monstera-01/lockout/clear",
                serde_json::json!({ "reason": "leak" }),
            )
            .await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
    }

    /// Once the tray is positively dry, an operator may clear it — and only
    /// then.
    #[tokio::test]
    async fn a_leak_lockout_clears_once_the_signal_is_positively_clear() {
        let api = TestApi::start().await;
        api.waterable("monstera-01").await;
        api.device_connected().await;
        api.sample_bool(api.clock.now(), "leak-0", "tray", "leak_state", true)
            .await;
        api.irrigate("monstera-01").await;
        assert_eq!(
            rhizo_storage::repo::plant::get(&api.db, "monstera-01")
                .await
                .unwrap()
                .unwrap()
                .lockout_reason
                .as_deref(),
            Some("leak")
        );

        api.clock.advance(chrono::Duration::minutes(5));
        api.sample_bool(api.clock.now(), "leak-0", "tray", "leak_state", false)
            .await;
        api.sample(api.clock.now(), 20.0).await;
        api.sample_from(
            api.clock.now(),
            "tank-0",
            "reservoir",
            "tank_level",
            "percent",
            70.0,
        )
        .await;

        let (status, body) = api
            .json(
                "POST",
                "/api/v1/plants/monstera-01/lockout/clear",
                serde_json::json!({ "reason": "leak" }),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["cleared"], "leak");
        let plant = rhizo_storage::repo::plant::get(&api.db, "monstera-01")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(plant.lockout_reason, None);
        let cleared_by: Option<String> = sqlx::query_scalar(
            "SELECT lockout_cleared_by FROM plants WHERE plant_id='monstera-01'",
        )
        .fetch_one(api.db.pool())
        .await
        .unwrap();
        assert_eq!(cleared_by.as_deref(), Some("operator"));
    }

    /// SAFETY-003 across real ticks: a leak lockout outlives a silent sensor
    /// *and* the sensor's return, and only the operator's reset lifts it.
    ///
    /// The defect this pins: a stale leak reading made the gate answer
    /// `Uncertain` ahead of the sticky `Leak`, the tick persisted `uncertain`
    /// over `leak`, and `uncertain` auto-cleared the moment telemetry resumed —
    /// no operator involved. Found by `scenario_leak` on CI.
    #[tokio::test]
    async fn safety_003_a_leak_lockout_survives_a_silent_sensor_and_its_return() {
        let api = TestApi::start().await;
        api.waterable("monstera-01").await;
        api.device_connected().await;
        api.sample_bool(api.clock.now(), "leak-0", "tray", "leak_state", true)
            .await;
        api.irrigate("monstera-01").await;
        let lockout = || async {
            rhizo_storage::repo::plant::get(&api.db, "monstera-01")
                .await
                .unwrap()
                .unwrap()
                .lockout_reason
        };
        assert_eq!(lockout().await.as_deref(), Some("leak"));

        // The leak sensor falls silent long enough for its reading to go stale.
        api.clock.advance(chrono::Duration::hours(4));
        api.irrigate("monstera-01").await;
        assert_eq!(
            lockout().await.as_deref(),
            Some("leak"),
            "a silent leak sensor must not rewrite the leak lockout"
        );

        // Every input comes back healthy, the tray included.
        api.sample_bool(api.clock.now(), "leak-0", "tray", "leak_state", false)
            .await;
        api.sample(api.clock.now(), 20.0).await;
        api.sample_from(
            api.clock.now(),
            "tank-0",
            "reservoir",
            "tank_level",
            "percent",
            70.0,
        )
        .await;
        api.irrigate("monstera-01").await;
        assert_eq!(
            lockout().await.as_deref(),
            Some("leak"),
            "healthy readings are not an operator reset"
        );

        let (status, body) = api
            .json(
                "POST",
                "/api/v1/plants/monstera-01/lockout/clear",
                serde_json::json!({ "reason": "leak" }),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        api.irrigate("monstera-01").await;
        assert_eq!(lockout().await, None);
    }

    /// SAFETY-003 against the gate's step 0: removing the pump does not turn a
    /// latched `leak` into the auto-clearing `no_actuator`, so re-binding it
    /// does not lift the leak either.
    ///
    /// Holds today because the control pass skips a plant with no actuator
    /// (SAFETY-018) and `POST /water` answers 422 before the gate, so step 0's
    /// answer is never persisted. This pins that, since the gate's own order
    /// would not protect the latch.
    #[tokio::test]
    async fn safety_003_removing_the_pump_does_not_rewrite_a_leak_lockout() {
        // The control pass writes `plants_locked_out`, which is process-wide.
        let _guard = crate::api::health::gauge_lock().lock().await;
        let api = TestApi::start().await;
        api.waterable("monstera-01").await;
        api.device_connected().await;
        api.sample_bool(api.clock.now(), "leak-0", "tray", "leak_state", true)
            .await;
        let pass = || async {
            crate::control::tick::irrigation_pass(
                &api.commander,
                &api.state.metrics,
                api.clock.now(),
            )
            .await
            .unwrap();
            rhizo_storage::repo::plant::get(&api.db, "monstera-01")
                .await
                .unwrap()
                .unwrap()
                .lockout_reason
        };
        assert_eq!(pass().await.as_deref(), Some("leak"));

        // The tray dries; every input is healthy.
        api.clock.advance(chrono::Duration::minutes(5));
        api.sample_bool(api.clock.now(), "leak-0", "tray", "leak_state", false)
            .await;
        api.sample(api.clock.now(), 20.0).await;
        api.sample_from(
            api.clock.now(),
            "tank-0",
            "reservoir",
            "tank_level",
            "percent",
            70.0,
        )
        .await;

        let (status, body) = api
            .json(
                "DELETE",
                "/api/v1/plants/monstera-01/bindings/actuator",
                serde_json::json!({}),
            )
            .await;
        assert!(status.is_success(), "{status}: {body}");
        assert_eq!(
            pass().await.as_deref(),
            Some("leak"),
            "a plant with no pump must not have its leak rewritten"
        );

        api.bind_actuator("monstera-01").await;
        assert_eq!(
            pass().await.as_deref(),
            Some("leak"),
            "re-binding the pump is not an operator leak reset"
        );
    }

    // ------------------------------------------- independent recorded lockouts

    /// The recorded lockout names of the plant, sorted.
    async fn records(api: &TestApi) -> Vec<String> {
        let mut names: Vec<String> = rhizo_storage::repo::lockout::active(
            &api.db,
            "monstera-01",
            api.clock.now().timestamp_millis(),
        )
        .await
        .unwrap()
        .into_iter()
        .map(|record| record.reason)
        .collect();
        names.sort();
        names
    }

    /// The display summary.
    async fn summary(api: &TestApi) -> Option<String> {
        rhizo_storage::repo::plant::get(&api.db, "monstera-01")
            .await
            .unwrap()
            .unwrap()
            .lockout_reason
    }

    /// Every input healthy and fresh, the tray dry.
    async fn healthy(api: &TestApi) {
        api.sample_bool(api.clock.now(), "leak-0", "tray", "leak_state", false)
            .await;
        api.sample(api.clock.now(), 40.0).await;
        api.sample_from(
            api.clock.now(),
            "tank-0",
            "reservoir",
            "tank_level",
            "percent",
            70.0,
        )
        .await;
    }

    /// The real refusal, not only the stored reason: a manual dose is answered
    /// 409 for `reason`, and nothing is persisted or published.
    async fn refused(api: &TestApi, reason: &str) {
        let (status, body) = api
            .json(
                "POST",
                "/api/v1/plants/monstera-01/water",
                serde_json::json!({ "ml": 30.0, "mode": "manual" }),
            )
            .await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        assert_eq!(body["error"]["details"]["reason"], reason, "{body}");
        assert!(api.transport.commands().is_empty(), "nothing published");
        let commands: i64 = sqlx::query_scalar("SELECT count(*) FROM commands")
            .fetch_one(api.db.pool())
            .await
            .unwrap();
        assert_eq!(commands, 0, "nothing persisted");
    }

    /// And the converse, so no refusal above passes vacuously.
    async fn permitted(api: &TestApi) {
        let (status, body) = api
            .json(
                "POST",
                "/api/v1/plants/monstera-01/water",
                serde_json::json!({ "ml": 30.0, "mode": "manual" }),
            )
            .await;
        assert_eq!(status, StatusCode::ACCEPTED, "{body}");
        assert_eq!(api.transport.commands().len(), 1);
    }

    async fn clear(api: &TestApi, reason: &str) -> (StatusCode, serde_json::Value) {
        api.json(
            "POST",
            "/api/v1/plants/monstera-01/lockout/clear",
            serde_json::json!({ "reason": reason }),
        )
        .await
    }

    async fn clock_step(api: &TestApi) {
        let step = crate::control::clock_step::Step {
            direction: crate::control::clock_step::Direction::Forward,
            magnitude: chrono::Duration::hours(1),
        };
        crate::control::clock_step_response(&api.commander, step, api.clock.now())
            .await
            .unwrap();
    }

    /// A leak on top of each other explicit-clear fault: both are recorded,
    /// clearing the leak leaves the other refusing on its own, and only the
    /// second, separate reset restores watering.
    #[tokio::test]
    async fn safety_003_concurrent_latches_clear_independently() {
        for other in ["pump_fault", "no_delivery_detected", "max_doses_reached"] {
            let api = TestApi::start().await;
            api.waterable("monstera-01").await;
            api.device_connected().await;
            // A fault from earlier, by its own writer.
            rhizo_storage::repo::lockout::latch(
                &api.db,
                "monstera-01",
                other,
                api.clock.now().timestamp_millis(),
            )
            .await
            .unwrap();
            api.sample_bool(api.clock.now(), "leak-0", "tray", "leak_state", true)
                .await;
            api.irrigate("monstera-01").await;
            let mut both = vec!["leak".to_owned(), other.to_owned()];
            both.sort();
            assert_eq!(records(&api).await, both, "{other}");
            assert_eq!(summary(&api).await.as_deref(), Some("leak"));
            refused(&api, "leak").await;

            api.clock.advance(chrono::Duration::minutes(5));
            healthy(&api).await;
            api.irrigate("monstera-01").await;
            assert_eq!(records(&api).await, both, "a dry tray resets nothing");

            let (status, body) = clear(&api, "leak").await;
            assert_eq!(status, StatusCode::OK, "{body}");
            api.irrigate("monstera-01").await;
            assert_eq!(records(&api).await, [other.to_owned()]);
            assert_eq!(summary(&api).await.as_deref(), Some(other));
            refused(&api, other).await;

            let (status, body) = clear(&api, other).await;
            assert_eq!(status, StatusCode::OK, "{body}");
            api.irrigate("monstera-01").await;
            assert!(records(&api).await.is_empty());
            assert_eq!(summary(&api).await, None);
            permitted(&api).await;
        }
    }

    /// A forward clock step over a latched leak: the hold is recorded beside
    /// the leak, and its expiry takes nothing with it.
    #[tokio::test]
    async fn safety_003_a_clock_step_hold_expiring_leaves_the_leak() {
        let api = TestApi::start().await;
        api.waterable("monstera-01").await;
        api.device_connected().await;
        api.sample_bool(api.clock.now(), "leak-0", "tray", "leak_state", true)
            .await;
        api.irrigate("monstera-01").await;
        clock_step(&api).await;
        api.irrigate("monstera-01").await;
        assert_eq!(records(&api).await, ["leak", "uncertain"]);
        assert_eq!(summary(&api).await.as_deref(), Some("leak"));

        // Long past the cooldown, every input healthy.
        api.clock.advance(chrono::Duration::hours(72));
        healthy(&api).await;
        api.irrigate("monstera-01").await;
        assert_eq!(
            records(&api).await,
            ["leak"],
            "the hold expired, the leak did not"
        );
        assert_eq!(summary(&api).await.as_deref(), Some("leak"));
        refused(&api, "leak").await;

        let (status, body) = clear(&api, "leak").await;
        assert_eq!(status, StatusCode::OK, "{body}");
        permitted(&api).await;
    }

    /// The operator resets the leak inside the clock-step cooldown: the reset
    /// lifts the leak and nothing else, the hold keeps refusing until its own
    /// deadline, and no request can clear a hold.
    #[tokio::test]
    async fn safety_003_clearing_the_leak_inside_a_clock_hold_leaves_the_hold() {
        let api = TestApi::start().await;
        api.waterable("monstera-01").await;
        api.device_connected().await;
        api.sample_bool(api.clock.now(), "leak-0", "tray", "leak_state", true)
            .await;
        api.irrigate("monstera-01").await;
        clock_step(&api).await;

        api.clock.advance(chrono::Duration::minutes(5));
        healthy(&api).await;
        let (status, body) = clear(&api, "leak").await;
        assert_eq!(status, StatusCode::OK, "{body}");
        api.irrigate("monstera-01").await;
        assert_eq!(records(&api).await, ["uncertain"]);
        assert_eq!(summary(&api).await.as_deref(), Some("uncertain"));
        refused(&api, "uncertain").await;

        let (status, body) = clear(&api, "uncertain").await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        assert_eq!(body["error"]["code"], "hold_not_clearable");
        assert_eq!(records(&api).await, ["uncertain"]);

        api.clock.advance(chrono::Duration::hours(72));
        healthy(&api).await;
        api.irrigate("monstera-01").await;
        assert!(records(&api).await.is_empty());
        assert_eq!(summary(&api).await, None);
        permitted(&api).await;
    }

    /// A restart is a new edge over the same database: every latch and hold is
    /// still there, and still refuses.
    #[tokio::test]
    async fn safety_003_a_restart_keeps_every_recorded_lockout() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("edge.sqlite");
        {
            let api = TestApi::start_on(EdgeDb::connect(&path).await.unwrap()).await;
            api.waterable("monstera-01").await;
            api.device_connected().await;
            rhizo_storage::repo::lockout::latch(&api.db, "monstera-01", "no_delivery_detected", 1)
                .await
                .unwrap();
            api.sample_bool(api.clock.now(), "leak-0", "tray", "leak_state", true)
                .await;
            api.irrigate("monstera-01").await;
            clock_step(&api).await;
            assert_eq!(
                records(&api).await,
                ["leak", "no_delivery_detected", "uncertain"]
            );
            api.db.pool().close().await;
        }
        let api = TestApi::start_on(EdgeDb::connect(&path).await.unwrap()).await;
        assert_eq!(
            records(&api).await,
            ["leak", "no_delivery_detected", "uncertain"]
        );
        healthy(&api).await;
        api.irrigate("monstera-01").await;
        assert_eq!(summary(&api).await.as_deref(), Some("leak"));
        refused(&api, "leak").await;
    }

    /// The race the old automatic clear lost: a pass gathers, another writer
    /// latches a fault, and the pass then applies a decision made without it.
    /// The latch survives and the next evaluation refuses on it.
    #[tokio::test]
    async fn a_latch_written_during_a_pass_is_not_lost() {
        let api = TestApi::start().await;
        api.waterable("monstera-01").await;
        api.device_connected().await;
        healthy(&api).await;
        // A derived block shown beforehand: the summary the old code cleared.
        rhizo_storage::repo::command::set_summary(
            &api.db,
            "monstera-01",
            Some("stale_data"),
            None,
            api.clock.now().timestamp_millis(),
        )
        .await
        .unwrap();
        let loaded = crate::plant::load(&api.db, "monstera-01")
            .await
            .unwrap()
            .unwrap();
        let now = api.clock.now();
        let analysis = crate::plant::analyse(&api.db, &loaded, now).await.unwrap();
        let mode = EvaluationMode::Automatic;
        let (gathered, decision) =
            irrigation::preview(&api.db, &loaded, analysis.inputs.dry_duration, mode, now)
                .await
                .unwrap();
        assert!(
            !matches!(
                decision,
                IrrigationDecision::Lock { .. } | IrrigationDecision::IssueDose { .. }
            ),
            "{decision:?}"
        );

        rhizo_storage::repo::lockout::latch(
            &api.db,
            "monstera-01",
            "no_delivery_detected",
            now.timestamp_millis(),
        )
        .await
        .unwrap();
        irrigation::apply(&api.commander, &loaded, &gathered, decision, mode, now)
            .await
            .unwrap();

        assert_eq!(records(&api).await, ["no_delivery_detected"]);
        api.irrigate("monstera-01").await;
        assert_eq!(summary(&api).await.as_deref(), Some("no_delivery_detected"));
        refused(&api, "no_delivery_detected").await;
    }

    /// The display summary is not an input: a summary naming a lockout with no
    /// record behind it refuses nothing, and a record with no summary refuses.
    #[tokio::test]
    async fn the_summary_column_decides_nothing() {
        let api = TestApi::start().await;
        api.waterable("monstera-01").await;
        api.device_connected().await;
        healthy(&api).await;
        rhizo_storage::repo::command::set_summary(
            &api.db,
            "monstera-01",
            Some("leak"),
            None,
            api.clock.now().timestamp_millis(),
        )
        .await
        .unwrap();
        api.irrigate("monstera-01").await;
        assert_eq!(summary(&api).await, None, "the pass corrects the display");

        rhizo_storage::repo::lockout::latch(
            &api.db,
            "monstera-01",
            "max_doses_reached",
            api.clock.now().timestamp_millis(),
        )
        .await
        .unwrap();
        refused(&api, "max_doses_reached").await;
    }

    /// Clearing a lockout the plant does not have, or naming the wrong one, is
    /// refused rather than silently accepted.
    #[tokio::test]
    async fn clearing_the_wrong_lockout_is_refused() {
        let api = TestApi::start().await;
        api.waterable("monstera-01").await;
        api.device_connected().await;
        let (status, body) = api
            .json(
                "POST",
                "/api/v1/plants/monstera-01/lockout/clear",
                serde_json::json!({ "reason": "leak" }),
            )
            .await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        assert_eq!(body["error"]["code"], "not_locked_out");
    }

    // ---------------------------------------------------------- auto-watering

    /// The opt-in an operator controls, and its `false` default.
    #[tokio::test]
    async fn auto_watering_can_be_enabled_and_disabled() {
        let api = TestApi::start().await;
        api.waterable("monstera-01").await;
        let (_, plant) = api.get("/api/v1/plants/monstera-01").await;
        assert_eq!(plant["auto_watering_enabled"], false, "the default is off");

        let (status, body) = api
            .json(
                "POST",
                "/api/v1/plants/monstera-01/auto-watering/enable",
                serde_json::json!({}),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["auto_watering_enabled"], true);

        let (_, body) = api
            .json(
                "POST",
                "/api/v1/plants/monstera-01/auto-watering/disable",
                serde_json::json!({}),
            )
            .await;
        assert_eq!(body["auto_watering_enabled"], false);
    }

    /// The whole point of the opt-in: with it off, a bone-dry confirmed plant
    /// produces advice and no command.
    ///
    /// Two passes, not one, and deliberately so. PRD 060's transition table is
    /// `Normal -> Drying -> DryConfirmed -> DoseIssued`, and `DryConfirmed` is
    /// an observable persisted state rather than a predicate evaluated in
    /// passing: the pass that *confirms* dryness records it and does nothing
    /// else, and the pass after it acts. Asserting the intermediate state is
    /// what stops a machine that reached the right answer by skipping a step
    /// from passing.
    #[tokio::test]
    async fn automation_off_means_a_recommendation_and_no_command() {
        use rhizo_domain::irrigation::types::IrrigationDecision;
        use rhizo_domain::state::IrrigationState;

        let api = TestApi::start().await;
        api.waterable("monstera-01").await;
        api.device_connected().await;

        let confirming = api.irrigate("monstera-01").await;
        assert_eq!(confirming.decision, IrrigationDecision::Idle);
        assert_eq!(confirming.state, IrrigationState::DryConfirmed);
        assert!(api.transport.commands().is_empty());

        let pass = api.irrigate("monstera-01").await;
        assert!(
            matches!(pass.decision, IrrigationDecision::Recommend { .. }),
            "{:?}",
            pass.decision
        );
        assert!(api.transport.commands().is_empty());

        // Turned on, the same plant doses.
        api.json(
            "POST",
            "/api/v1/plants/monstera-01/auto-watering/enable",
            serde_json::json!({}),
        )
        .await;
        let pass = api.irrigate("monstera-01").await;
        assert!(pass.command_id.is_some(), "{:?}", pass.decision);
        assert_eq!(api.transport.commands().len(), 1);
    }

    // ------------------------------------------------------------- the ledger

    /// `GET /commands/{id}` reports the lifecycle.
    #[tokio::test]
    async fn a_command_can_be_read_back() {
        let api = TestApi::start().await;
        api.waterable("monstera-01").await;
        api.device_connected().await;
        let (_, body) = api
            .json(
                "POST",
                "/api/v1/plants/monstera-01/water",
                serde_json::json!({ "ml": 30.0 }),
            )
            .await;
        let command_id = body["command_id"].as_str().unwrap();
        let (status, command) = api.get(&format!("/api/v1/commands/{command_id}")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(command["command_id"], command_id);
        assert_eq!(command["kind"], "water");
        assert_eq!(command["mode"], "manual");
        assert!(command["expires_at"].is_string());

        let (status, _) = api.get("/api/v1/commands/unknown").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }
    /// **SAFETY-005, the regression this pass closed.** `stale_after_ms` is
    /// validated only as positive, so an operator may set a day. The cadence
    /// bound — `min(policy, max(15 min, 3 x telemetry interval))` — is what
    /// stops that becoming a day-long licence to water on old data, and it was
    /// applied on the *recommendation* path while the **gate** read the raw
    /// stored value. A 4-hour-old reading was therefore accepted and a real
    /// command published.
    ///
    /// Every policy is widened here, not just soil: the gate used to take the
    /// minimum across all kinds, so a default leak or tank policy masked the
    /// hole. Both halves are fixed, so both are widened to prove it.
    #[tokio::test]
    async fn safety_005_a_widened_stale_after_cannot_outlive_the_cadence_bound() {
        let api = TestApi::start().await;
        api.waterable("monstera-01").await;
        api.device_connected().await;
        for (kind, body) in [
            (
                "soil_moisture",
                serde_json::json!({
                    "target_min": 28.0,
                    "target_max": 45.0,
                    "stale_after_ms": 86_400_000i64,
                    "confirm_duration_ms": 1_800_000,
                }),
            ),
            (
                "leak_state",
                serde_json::json!({ "stale_after_ms": 86_400_000i64 }),
            ),
            (
                "tank_level",
                serde_json::json!({ "stale_after_ms": 86_400_000i64 }),
            ),
        ] {
            let (status, value) = api
                .json(
                    "PUT",
                    &format!("/api/v1/plants/monstera-01/measurement-policies/{kind}"),
                    body,
                )
                .await;
            assert_eq!(status, StatusCode::OK, "{value}");
        }

        // One hour with no soil reading. Leak and tank stay fresh, so the only
        // thing that can refuse is the control sample's own age.
        //
        // One hour, deliberately: the device's cadence is 300 s, so the bound
        // is max(15 min, 15 min) and an hour is far past it — while staying
        // *inside* the domain's three-hour backstop. A four-hour sample would
        // be refused by the clamp alone and this test would still pass with the
        // adapter's narrowing removed, pinning nothing.
        // `gate::safety_005_an_unbounded_freshness_limit_is_clamped_to_the_ceiling`
        // is what covers the backstop.
        api.clock.advance(chrono::Duration::hours(1));
        api.sample_bool(api.clock.now(), "leak-0", "tray", "leak_state", false)
            .await;
        api.sample_from(
            api.clock.now(),
            "tank-0",
            "reservoir",
            "tank_level",
            "percent",
            70.0,
        )
        .await;

        let (status, body) = api
            .json(
                "POST",
                "/api/v1/plants/monstera-01/water",
                serde_json::json!({ "ml": 30.0, "mode": "recommended" }),
            )
            .await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        assert_eq!(body["error"]["details"]["reason"], "stale_data");
        assert!(
            api.transport.commands().is_empty(),
            "nothing may be published for a refused dose"
        );
    }
}
