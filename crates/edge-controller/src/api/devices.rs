//! Device registry endpoints.
#![allow(missing_docs)]
use super::ApiState;
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use chrono::{SecondsFormat, TimeZone, Utc};
use serde::Deserialize;
use sqlx::Row as _;

fn error(status: StatusCode, code: &str, message: &str) -> Response {
    (
        status,
        Json(serde_json::json!({"error":{"code":code,"message":message,"details":{}}})),
    )
        .into_response()
}
fn timestamp(value: Option<i64>) -> Option<String> {
    value
        .and_then(|ms| Utc.timestamp_millis_opt(ms).single())
        .map(|v| v.to_rfc3339_opts(SecondsFormat::Millis, true))
}

async fn one(
    db: &rhizo_storage::EdgeDb,
    id: &str,
) -> Result<Option<serde_json::Value>, sqlx::Error> {
    let Some(row) = sqlx::query("SELECT * FROM devices WHERE device_id=?")
        .bind(id)
        .fetch_optional(db.pool())
        .await?
    else {
        return Ok(None);
    };
    #[allow(
        clippy::disallowed_methods,
        reason = "HTTP serialization is a host-clock adapter, not domain decision logic"
    )]
    let now = Utc::now().timestamp_millis();
    let last_seen: Option<i64> = row.get("last_seen_at");
    let interval: i64 = row.get("telemetry_interval_seconds");
    let effective_interval = if row.get::<String, _>("power_mode") == "battery" {
        row.get::<Option<i64>, _>("wake_interval_seconds")
            .unwrap_or(interval)
    } else {
        interval
    };
    let desired: i64 = row.get("desired_config_version");
    let applied: Option<i64> = row.get("applied_config_version");
    let drift_since: Option<i64> = row.get("drift_since");
    let sensors: serde_json::Value = serde_json::from_str(row.get::<&str, _>("sensors_json"))
        .unwrap_or_else(|_| serde_json::json!([]));
    let status_snapshot: serde_json::Value = row
        .get::<Option<String>, _>("status_json")
        .and_then(|value| serde_json::from_str(&value).ok())
        .unwrap_or(serde_json::Value::Null);
    let capabilities = sqlx::query("SELECT capability_id,class,kinds_json,point FROM device_capabilities WHERE device_id=? ORDER BY class,capability_id").bind(id).fetch_all(db.pool()).await?
        .into_iter().map(|r| serde_json::json!({"id":r.get::<String,_>("capability_id"),"class":r.get::<String,_>("class"),"kinds":serde_json::from_str::<serde_json::Value>(r.get::<&str,_>("kinds_json")).unwrap_or_else(|_|serde_json::json!([])),"point":r.get::<Option<String>,_>("point")})).collect::<Vec<_>>();
    let mode: String = row.get("connectivity_mode");
    let expected_wake_at: Option<i64> = row.get("expected_wake_at");
    let connectivity = crate::device::connectivity::from_projection(&mode, expected_wake_at);
    Ok(Some(serde_json::json!({
        "device_id": row.get::<String,_>("device_id"), "name": row.get::<Option<String>,_>("name"),
        "status": row.get::<String,_>("status"), "firmware_version": row.get::<Option<String>,_>("firmware_version"),
        "protocol_version": row.get::<Option<i64>,_>("protocol_version"), "clock_synced": row.get::<i64,_>("clock_synced") != 0,
        "last_seen_at": timestamp(last_seen), "sample_age_seconds": last_seen.map(|seen| crate::device::health::sample_age_seconds(now, seen)),
        "stale": last_seen.is_some_and(|seen| crate::device::health::sample_age_seconds(now, seen) >= crate::device::health::stale_after_seconds(effective_interval)),
        "config":{"desired_version":desired,"applied_version":applied,"drift":applied != Some(desired) && drift_since.is_some_and(|since| now.saturating_sub(since) >= interval.saturating_mul(2000))},
        "sensors": sensors, "capabilities": capabilities,
        "limits": status_snapshot.get("limits").cloned().unwrap_or(serde_json::Value::Null),
        "connectivity": connectivity.api_name(),
        "expected_wake_at": timestamp(expected_wake_at),
        "power_mode": row.get::<String,_>("power_mode"),
        "wake_interval_seconds": row.get::<Option<i64>,_>("wake_interval_seconds"),
        "missed_wake_count": row.get::<i64,_>("missed_wake_count"),
        "plant_id": serde_json::Value::Null
    })))
}

pub async fn list(State(state): State<ApiState>) -> Response {
    let ids = sqlx::query_scalar::<_, String>("SELECT device_id FROM devices ORDER BY device_id")
        .fetch_all(state.db.pool())
        .await;
    let Ok(ids) = ids else {
        return error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "storage_error",
            "registry query failed",
        );
    };
    let mut devices = Vec::new();
    for id in ids {
        match one(&state.db, &id).await {
            Ok(Some(v)) => devices.push(v),
            Ok(None) => {}
            Err(_) => {
                return error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "storage_error",
                    "registry query failed",
                );
            }
        }
    }
    Json(serde_json::json!({"devices":devices})).into_response()
}
pub async fn get(State(state): State<ApiState>, Path(id): Path<String>) -> Response {
    match one(&state.db, &id).await {
        Ok(Some(v)) => Json(v).into_response(),
        Ok(None) => error(StatusCode::NOT_FOUND, "device_not_found", "unknown device"),
        Err(_) => error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "storage_error",
            "registry query failed",
        ),
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatchDevice {
    name: String,
}
pub async fn patch(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Json(body): Json<PatchDevice>,
) -> Response {
    match sqlx::query("UPDATE devices SET name=? WHERE device_id=?")
        .bind(body.name)
        .bind(&id)
        .execute(state.db.pool())
        .await
    {
        Ok(done) if done.rows_affected() == 1 => get(State(state), Path(id)).await,
        Ok(_) => error(StatusCode::NOT_FOUND, "device_not_found", "unknown device"),
        Err(_) => error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "storage_error",
            "registry update failed",
        ),
    }
}
#[derive(Deserialize)]
pub struct EventQuery {
    since: Option<String>,
    limit: Option<u32>,
}
pub async fn events(
    State(state): State<ApiState>,
    Path(id): Path<String>,
    Query(q): Query<EventQuery>,
) -> Response {
    let exists = sqlx::query_scalar::<_, i64>("SELECT count(*) FROM devices WHERE device_id=?")
        .bind(&id)
        .fetch_one(state.db.pool())
        .await
        .unwrap_or(0);
    if exists == 0 {
        return error(StatusCode::NOT_FOUND, "device_not_found", "unknown device");
    }
    let since = q
        .since
        .as_deref()
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|v| v.timestamp_millis())
        .unwrap_or(i64::MIN);
    let limit = i64::from(q.limit.unwrap_or(100).min(500));
    match sqlx::query("SELECT event_id,kind,severity,detail_json,occurred_at FROM device_events WHERE device_id=? AND occurred_at>=? ORDER BY occurred_at DESC LIMIT ?").bind(id).bind(since).bind(limit).fetch_all(state.db.pool()).await {
        Ok(rows) => Json(serde_json::json!({"events":rows.into_iter().map(|r|serde_json::json!({"event_id":r.get::<String,_>("event_id"),"kind":r.get::<String,_>("kind"),"severity":r.get::<String,_>("severity"),"detail":r.get::<Option<String>,_>("detail_json").and_then(|s|serde_json::from_str::<serde_json::Value>(&s).ok()),"occurred_at":timestamp(Some(r.get::<i64,_>("occurred_at")))})).collect::<Vec<_>>() })).into_response(),
        Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR,"storage_error","event query failed")
    }
}
pub async fn quarantined(State(state): State<ApiState>) -> Response {
    match sqlx::query("SELECT id,device_id,topic,error,received_at FROM quarantined_messages ORDER BY received_at DESC LIMIT 500").fetch_all(state.db.pool()).await {
        Ok(rows) => Json(serde_json::json!({"messages":rows.into_iter().map(|r|serde_json::json!({"id":r.get::<i64,_>("id"),"device_id":r.get::<Option<String>,_>("device_id"),"topic":r.get::<String,_>("topic"),"error":r.get::<String,_>("error"),"received_at":timestamp(Some(r.get::<i64,_>("received_at")))})).collect::<Vec<_>>() })).into_response(),
        Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR,"storage_error","quarantine query failed")
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn rfc3339_is_utc() {
        assert!(super::timestamp(Some(0)).unwrap().ends_with('Z'));
    }
    #[tokio::test]
    async fn battery_state_exposes_expected_wake_without_device_time() {
        use rhizo_mqtt_contract::payload::{
            DeviceStatus, DeviceStatusValue, PowerMode, PowerStatus,
        };
        let db = rhizo_storage::EdgeDb::in_memory().await.unwrap();
        db.migrate().await.unwrap();
        let mut status: rhizo_mqtt_contract::Envelope<DeviceStatus> =
            rhizo_mqtt_contract::Envelope::from_json(include_bytes!(
                "../../../../test/fixtures/protocol/valid/status-with-capabilities.json"
            ))
            .unwrap();
        status.sequence = Some(status.sequence.unwrap() + 1);
        status.message_id = rhizo_mqtt_contract::MessageId::from_uuid(uuid::Uuid::new_v4());
        status.data.status = DeviceStatusValue::Offline;
        status.data.reason = Some("sleeping".into());
        status.data.power = Some(Box::new(PowerStatus {
            mode: PowerMode::Battery,
            wake_interval_seconds: Some(60),
            expected_wake_ms: Some(u64::MAX),
            wake_reason: None,
            battery_mv: None,
            awake_ms: None,
        }));
        rhizo_storage::repo::ingest::persist_status(&db, &status, 1_000)
            .await
            .unwrap();
        let value = super::one(&db, "plant-node-01").await.unwrap().unwrap();
        assert_eq!(value["connectivity"], "sleeping");
        assert_eq!(value["power_mode"], "battery");
        assert_eq!(value["wake_interval_seconds"], 60);
        assert_eq!(value["expected_wake_at"], "1970-01-01T00:01:01.000Z");
    }
}
