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
