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
    let power_mode: String = row.get("power_mode");
    let wake_interval: Option<i64> = row.get("wake_interval_seconds");
    let liveness_interval =
        crate::device::health::liveness_interval_seconds(&power_mode, interval, wake_interval);
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
    // Derived at read time against the edge's own clock, so an overdue sleeper
    // reports `isolated` even if the liveness timer has not run (SAFETY-021).
    let mode: String = row.get("connectivity_mode");
    let connectivity = crate::device::connectivity::from_projection(
        &mode,
        row.get("expected_wake_at"),
        row.get("overdue_at"),
        now,
    );
    Ok(Some(serde_json::json!({
        "device_id": row.get::<String,_>("device_id"), "name": row.get::<Option<String>,_>("name"),
        "status": row.get::<String,_>("status"), "firmware_version": row.get::<Option<String>,_>("firmware_version"),
        "protocol_version": row.get::<Option<i64>,_>("protocol_version"), "clock_synced": row.get::<i64,_>("clock_synced") != 0,
        "last_seen_at": timestamp(last_seen), "sample_age_seconds": last_seen.map(|seen| crate::device::health::sample_age_seconds(now, seen)),
        "stale": last_seen.is_some_and(|seen| crate::device::health::sample_age_seconds(now, seen) >= crate::device::health::max_sample_age_seconds(liveness_interval)),
        "config":{"desired_version":desired,"applied_version":applied,"drift":applied != Some(desired) && drift_since.is_some_and(|since| now.saturating_sub(since) >= interval.saturating_mul(2000))},
        "sensors": sensors, "capabilities": capabilities,
        "limits": status_snapshot.get("limits").cloned().unwrap_or(serde_json::Value::Null),
        "connectivity": connectivity.api_name(),
        "expected_wake_at": timestamp(connectivity.expected_wake_at()),
        "power_mode": power_mode,
        "wake_interval_seconds": wake_interval,
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

pub async fn latest_measurements(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Response {
    let exists =
        sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM devices WHERE device_id=?)")
            .bind(&id)
            .fetch_one(state.db.pool())
            .await;
    match exists {
        Ok(false) => return error(StatusCode::NOT_FOUND, "device_not_found", "unknown device"),
        Err(_) => {
            return error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "storage_error",
                "measurement query failed",
            );
        }
        Ok(true) => {}
    }
    let rows =
        match rhizo_storage::repo::query::latest_measurements_for_device(&state.db, &id).await {
            Ok(rows) => rows,
            Err(_) => {
                return error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "storage_error",
                    "measurement query failed",
                );
            }
        };
    let now = state.clock.now().timestamp_millis();
    let measurements = rows
        .into_iter()
        .map(|row| {
            let value = match (row.value_num, row.value_bool) {
                (Some(value), None) => serde_json::json!(value),
                (None, Some(value)) => serde_json::json!(value != 0),
                _ => serde_json::Value::Null,
            };
            serde_json::json!({
                "device_id": row.device_id,
                "sensor_id": row.sensor_id,
                "point": row.point,
                "kind": row.kind,
                "value": value,
                "unit": row.unit,
                "quality": row.quality,
                "received_at": timestamp(Some(row.received_at)),
                "age_seconds": now.saturating_sub(row.received_at).max(0) as f64 / 1000.0,
            })
        })
        .collect::<Vec<_>>();
    Json(serde_json::json!({"device_id":id,"measurements":measurements})).into_response()
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
    async fn latest_measurements_preserve_typed_stream_identity_and_order() {
        let api = crate::api::testsupport::TestApi::start().await;
        api.with_device().await;
        for (sensor, point, kind, number, boolean, unit) in [
            ("z-leak", "reservoir", "leak_state", None, Some(1_i64), ""),
            (
                "a-soil",
                "pot",
                "soil_moisture",
                Some(20.0),
                None,
                "vwc_percent",
            ),
            (
                "a-soil",
                "pot",
                "soil_moisture",
                Some(21.0),
                None,
                "vwc_percent",
            ),
        ] {
            sqlx::query("INSERT INTO measurements(device_id,sensor_id,point,kind,value_num,value_bool,unit,quality,received_at,batch_id,origin) VALUES('plant-node-01',?,?,?,?,?,?,'good',?,?, 'live')")
                .bind(sensor).bind(point).bind(kind).bind(number).bind(boolean).bind(unit)
                .bind(crate::api::testsupport::base().timestamp_millis()).bind(uuid::Uuid::new_v4().to_string())
                .execute(api.db.pool()).await.unwrap();
        }
        let (status, body) = api
            .get("/api/v1/devices/plant-node-01/measurements/latest")
            .await;
        assert_eq!(status, axum::http::StatusCode::OK);
        let rows = body["measurements"].as_array().unwrap();
        assert_eq!(rows.len(), 2, "only the latest row of each exact stream");
        assert_eq!(rows[0]["sensor_id"], "a-soil");
        assert_eq!(rows[0]["point"], "pot");
        assert_eq!(rows[0]["kind"], "soil_moisture");
        assert_eq!(rows[0]["value"], 21.0);
        assert_eq!(rows[1]["sensor_id"], "z-leak");
        assert_eq!(rows[1]["value"], true);
    }

    #[tokio::test]
    async fn latest_measurements_distinguish_empty_from_unknown_device() {
        let api = crate::api::testsupport::TestApi::start().await;
        api.with_device().await;
        let (status, body) = api
            .get("/api/v1/devices/plant-node-01/measurements/latest")
            .await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert!(body["measurements"].as_array().unwrap().is_empty());
        let (status, body) = api.get("/api/v1/devices/unknown/measurements/latest").await;
        assert_eq!(status, axum::http::StatusCode::NOT_FOUND);
        assert_eq!(body["error"]["code"], "device_not_found");
    }
    /// Builds a device whose sleep announcement was received `age_ms` ago on the
    /// edge clock, so the derived window is open or closed on purpose.
    async fn sleeping_device(age_ms: i64, wake_interval_seconds: u32) -> rhizo_storage::EdgeDb {
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
            wake_interval_seconds: Some(wake_interval_seconds),
            // Advisory only, and deliberately absurd: it must extend nothing.
            expected_wake_ms: Some(u64::MAX),
            wake_reason: None,
            battery_mv: None,
            awake_ms: None,
        }));
        #[allow(
            clippy::disallowed_methods,
            reason = "test fixture anchored to the host clock the endpoint reads"
        )]
        let received_at = chrono::Utc::now().timestamp_millis() - age_ms;
        rhizo_storage::repo::ingest::persist_status(&db, &status, received_at)
            .await
            .unwrap();
        db
    }

    #[tokio::test]
    async fn battery_state_exposes_expected_wake_without_device_time() {
        let db = sleeping_device(1_000, 900).await;
        let value = super::one(&db, "plant-node-01").await.unwrap().unwrap();
        assert_eq!(value["connectivity"], "sleeping");
        assert_eq!(value["power_mode"], "battery");
        assert_eq!(value["wake_interval_seconds"], 900);
        assert!(
            value["expected_wake_at"].is_string(),
            "an open window must publish its edge-computed wake instant"
        );
        assert_eq!(value["missed_wake_count"], 0);
    }

    /// SAFETY-021 read-side, and the negative control for the field above: the
    /// row still says `sleeping` and the liveness timer has never run, yet the
    /// endpoint must report `isolated` and must not advertise a wake instant.
    #[tokio::test]
    async fn safety_021_an_overdue_sleeper_reports_isolated_with_no_expected_wake() {
        // 900 s window, so `overdue_at` is 1800 s after receipt.
        let db = sleeping_device(1_801_000, 900).await;
        assert_eq!(
            sqlx::query_scalar::<_, String>("SELECT connectivity_mode FROM devices")
                .fetch_one(db.pool())
                .await
                .unwrap(),
            "sleeping",
            "the stored projection is deliberately left untouched by this test"
        );
        let value = super::one(&db, "plant-node-01").await.unwrap().unwrap();
        assert_eq!(value["connectivity"], "isolated");
        assert_eq!(
            value["expected_wake_at"],
            serde_json::Value::Null,
            "a missed wake is not an expected one"
        );
    }

    /// An always-on device never carries a wake instant, whatever else is in the
    /// row -- the second negative control for the same field.
    #[tokio::test]
    async fn an_always_on_device_publishes_no_expected_wake() {
        let db = rhizo_storage::EdgeDb::in_memory().await.unwrap();
        db.migrate().await.unwrap();
        let status: rhizo_mqtt_contract::Envelope<rhizo_mqtt_contract::payload::DeviceStatus> =
            rhizo_mqtt_contract::Envelope::from_json(include_bytes!(
                "../../../../test/fixtures/protocol/valid/status-with-capabilities.json"
            ))
            .unwrap();
        rhizo_storage::repo::ingest::persist_status(&db, &status, 1_000)
            .await
            .unwrap();
        let value = super::one(&db, "plant-node-01").await.unwrap().unwrap();
        assert_eq!(value["connectivity"], "connected");
        assert_eq!(value["power_mode"], "always_on");
        assert_eq!(value["expected_wake_at"], serde_json::Value::Null);
        assert_eq!(value["wake_interval_seconds"], serde_json::Value::Null);
    }
}
