//! The composite operator overview (M8-002, M8-004).
//!
//! One request that answers "is this site healthy?" — the landing view the
//! Compose topology's own smoke check uses, and the endpoint M8-004 reads the
//! running time scale from.
//!
//! # Connectivity is derived here, never read from the column
//!
//! `devices.connectivity_mode` is what the liveness timer *wrote*; whether a
//! device is still inside its wake window is a question about the clock now.
//! Counting the column directly would report a device as `sleeping` for ever if
//! the timer stopped, wedged, or had not run since startup — which is exactly
//! the place SAFETY-021 says dead devices must not be allowed to hide. Every
//! read-side projection in this crate goes through
//! [`connectivity::from_projection`](crate::device::connectivity::from_projection),
//! and this one is no exception.

use super::ApiState;
use axum::{Json, extract::State, http::StatusCode};
use chrono::{SecondsFormat, TimeZone, Utc};
use serde_json::{Value, json};
use sqlx::Row as _;

use crate::device::connectivity::{self, State as Connectivity};

fn timestamp(value: Option<i64>) -> Option<String> {
    value
        .and_then(|ms| Utc.timestamp_millis_opt(ms).single())
        .map(|v| v.to_rfc3339_opts(SecondsFormat::Millis, true))
}

/// Returns the dashboard projection.
///
/// Counts come from authoritative SQLite state; the time scale and the
/// fault-injection flag are process facts, not database ones.
///
/// # Errors
///
/// Returns 500 if any of the underlying reads fail. There is no partial
/// overview: a page that silently reported zero locked-out plants because one
/// query failed would be worse than no page.
pub async fn get(State(state): State<ApiState>) -> Result<Json<Value>, StatusCode> {
    let now = state.clock.now().timestamp_millis();
    let pool = state.db.pool();

    let device_rows = sqlx::query(
        "SELECT status,connectivity_mode,expected_wake_at,overdue_at FROM devices ORDER BY device_id",
    )
    .fetch_all(pool)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let mut devices_online = 0i64;
    let mut devices_offline = 0i64;
    let mut devices_sleeping = 0i64;
    let mut devices_reconciling = 0i64;
    for row in &device_rows {
        // A device whose row says `connected` but whose status is not `online`
        // has not been observed connected; the status column is the broker's
        // last word and the mode is the timer's, and disagreement resolves to
        // the unreachable answer (SAFETY-012).
        let status: String = row.get("status");
        let mode: String = row.get("connectivity_mode");
        let derived = connectivity::from_projection(
            &mode,
            row.get("expected_wake_at"),
            row.get("overdue_at"),
            now,
        );
        match derived {
            Connectivity::Online if status == "online" => devices_online += 1,
            Connectivity::Online | Connectivity::OfflineUnexpectedly => devices_offline += 1,
            Connectivity::SleepingExpected { .. } => devices_sleeping += 1,
            Connectivity::Reconciling => devices_reconciling += 1,
        }
    }

    let plant_rows = sqlx::query(
        "SELECT p.plant_id,p.name,p.auto_watering_enabled,p.lockout_reason,p.lockout_since,\
                s.state,r.decision \
         FROM plants p \
         LEFT JOIN irrigation_state s ON s.plant_id=p.plant_id \
         LEFT JOIN plant_recommendations r ON r.id=(\
             SELECT id FROM plant_recommendations WHERE plant_id=p.plant_id \
             ORDER BY evaluated_at DESC, id DESC LIMIT 1) \
         WHERE p.deleted_at IS NULL ORDER BY p.plant_id",
    )
    .fetch_all(pool)
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let mut plants = Vec::with_capacity(plant_rows.len());
    let mut plants_locked_out = 0i64;
    for row in &plant_rows {
        let lockout: Option<String> = row.get("lockout_reason");
        if lockout.is_some() {
            plants_locked_out += 1;
        }
        plants.push(json!({
            "plant_id": row.get::<String, _>("plant_id"),
            "name": row.get::<Option<String>, _>("name"),
            "auto_watering_enabled": row.get::<i64, _>("auto_watering_enabled") != 0,
            "irrigation_state": row.get::<Option<String>, _>("state"),
            "decision": row.get::<Option<String>, _>("decision"),
            "lockout_reason": lockout,
            "lockout_since": timestamp(row.get("lockout_since")),
        }));
    }

    let pending_events: i64 =
        sqlx::query_scalar("SELECT count(*) FROM pending_cloud_events WHERE status='pending'")
            .fetch_one(pool)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let last_success_at: Option<i64> =
        sqlx::query_scalar("SELECT max(synced_at) FROM pending_cloud_events WHERE status='synced'")
            .fetch_one(pool)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok(Json(json!({
        "edge_id": state.edge_id,
        "time_scale": state.time_scale,
        // Whether this build carries M8's process-boundary crash hooks. The
        // scenario runner refuses to start against a `false` here rather than
        // silently skipping the two scenarios that need them.
        "fault_injection": crate::faults::available(),
        "plants": plants,
        "devices_online": devices_online,
        "devices_offline": devices_offline,
        "devices_sleeping": devices_sleeping,
        "devices_reconciling": devices_reconciling,
        "plants_locked_out": plants_locked_out,
        "cloud": {
            "pending_events": pending_events,
            "last_success_at": timestamp(last_success_at)
        },
        "control_loop": { "healthy": true }
    })))
}
