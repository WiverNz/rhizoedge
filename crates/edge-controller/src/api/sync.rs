//! Operator visibility into the optional cloud outbox.
#![allow(missing_docs)]
use super::ApiState;
use axum::{
    Json,
    extract::{Query, State},
    http::StatusCode,
};
use serde::Deserialize;
use serde_json::{Value, json};
#[derive(Deserialize)]
pub struct Limit {
    limit: Option<u32>,
}
pub async fn status(State(s): State<ApiState>) -> Result<Json<Value>, StatusCode> {
    let (enabled, outbox_max_rows) = rhizo_storage::repo::outbox::settings(&s.db)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let (pending, quarantined) = rhizo_storage::repo::outbox::counts(&s.db)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(
        json!({"enabled":enabled,"outbox_max_rows":outbox_max_rows,"pending":pending,"quarantined":quarantined,"last_success_timestamp_seconds":s.metrics.cloud_last_success.get(),"batch_size":s.metrics.cloud_batch_size.get()}),
    ))
}
pub async fn quarantined(
    State(s): State<ApiState>,
    Query(q): Query<Limit>,
) -> Result<Json<Value>, StatusCode> {
    let rows = rhizo_storage::repo::outbox::quarantined(&s.db, q.limit.unwrap_or(100))
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(json!({"items":rows})))
}
