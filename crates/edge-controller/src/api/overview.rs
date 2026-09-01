//! Composite operator overview and runtime time-scale report.

use super::ApiState;
use axum::{Json, extract::State, http::StatusCode};
use serde_json::{Value, json};

/// Returns the small dashboard projection. Counts come from authoritative
/// SQLite state; the time scale is process configuration, not a database fact.
pub async fn get(State(state): State<ApiState>) -> Result<Json<Value>, StatusCode> {
    let devices_online: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM devices WHERE status='online' AND connectivity_mode='connected'",
    )
    .fetch_one(state.db.pool())
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let devices_offline: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM devices WHERE status!='online' OR connectivity_mode='isolated'",
    )
    .fetch_one(state.db.pool())
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let pending_events: i64 =
        sqlx::query_scalar("SELECT count(*) FROM pending_cloud_events WHERE status='pending'")
            .fetch_one(state.db.pool())
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let plants_locked_out: i64 =
        sqlx::query_scalar("SELECT count(*) FROM irrigation_state WHERE state='locked'")
            .fetch_one(state.db.pool())
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(json!({
        "edge_id": state.edge_id,
        "time_scale": state.time_scale,
        "plants": [],
        "devices_online": devices_online,
        "devices_offline": devices_offline,
        "plants_locked_out": plants_locked_out,
        "cloud": {
            "pending_events": pending_events,
            "last_success_at": Value::Null
        },
        "control_loop": { "healthy": true }
    })))
}
