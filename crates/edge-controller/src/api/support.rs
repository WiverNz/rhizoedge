//! Shared REST helpers.
//!
//! One error shape and one timestamp format across every endpoint, so a client
//! written against the device endpoints parses the plant ones unchanged
//! (`http-api-boundaries.md` §1).
use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use chrono::{SecondsFormat, TimeZone, Utc};

/// The documented error envelope.
#[must_use]
pub fn error(status: StatusCode, code: &str, message: &str) -> Response {
    (
        status,
        Json(serde_json::json!({"error":{"code":code,"message":message,"details":{}}})),
    )
        .into_response()
}

/// The documented error envelope with structured detail.
#[must_use]
pub fn error_with(
    status: StatusCode,
    code: &str,
    message: &str,
    details: serde_json::Value,
) -> Response {
    (
        status,
        Json(serde_json::json!({"error":{"code":code,"message":message,"details":details}})),
    )
        .into_response()
}

/// Runs a storage operation, waiting out a transient SQLite busy.
///
/// SQLite serialises writers even under WAL, and the edge has three of them: the
/// control loop, the ingestion pipeline, and this API. A request that arrives
/// while one of the others holds the write lock is *queued*, not failed — the
/// ingestion pipeline has taken that view since M3, and an operator's dose
/// request deserves the same.
///
/// Only [`StorageError::Busy`] is retried, which `classify` already names
/// transient. A constraint violation or an I/O error returns on the first
/// attempt, unchanged: this makes a caller wait, it does not make a failure
/// disappear.
///
/// # Errors
///
/// Returns the last failure when every attempt was busy, or the first failure
/// of any other kind.
pub async fn with_busy_retry<T, F, Fut>(mut operation: F) -> Result<T, rhizo_storage::StorageError>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, rhizo_storage::StorageError>>,
{
    let mut delay = std::time::Duration::from_millis(25);
    for attempt in 0..4 {
        match operation().await {
            Ok(value) => return Ok(value),
            Err(rhizo_storage::StorageError::Busy(reason)) if attempt < 3 => {
                tracing::warn!(attempt = attempt + 1, %reason, "retrying a busy SQLite read");
                tokio::time::sleep(delay).await;
                delay *= 2;
            }
            Err(error) => return Err(error),
        }
    }
    unreachable!("the bounded retry loop always returns")
}

/// A storage failure, reported without leaking the query.
#[must_use]
pub fn storage_error() -> Response {
    error(
        StatusCode::INTERNAL_SERVER_ERROR,
        "storage_error",
        "the request could not be served from storage",
    )
}

/// RFC 3339 in UTC, milliseconds, as every other endpoint renders time.
#[must_use]
pub fn timestamp(value: i64) -> Option<serde_json::Value> {
    Utc.timestamp_millis_opt(value)
        .single()
        .map(|v| serde_json::Value::String(v.to_rfc3339_opts(SecondsFormat::Millis, true)))
}

/// RFC 3339 in, edge milliseconds out.
#[must_use]
pub fn parse_timestamp(value: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|v| v.timestamp_millis())
}
