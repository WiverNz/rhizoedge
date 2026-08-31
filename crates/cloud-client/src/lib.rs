//! Typed client for the optional append-only cloud history service.
#![forbid(unsafe_code)]
#![allow(missing_docs)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use chrono::{DateTime, SecondsFormat, Utc};
use reqwest::{Client, StatusCode, Url};
use rhizo_telemetry::{Classify, FailureKind};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;
use thiserror::Error;
use uuid::Uuid;

/// One immutable fact waiting in the Edge outbox.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct OutboxEvent {
    pub event_id: Uuid,
    pub kind: String,
    pub occurred_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plant_id: Option<String>,
    pub payload: Value,
}

/// Per-event cloud ingestion outcome.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum EventResult {
    Accepted { event_id: Uuid },
    Duplicate { event_id: Uuid },
    Rejected { event_id: Uuid, error: String },
}

#[derive(Serialize)]
struct Batch<'a> {
    events: &'a [OutboxEvent],
}
#[derive(Deserialize)]
struct BatchResult {
    results: Vec<EventResult>,
}

/// Classifiable cloud transport failure.
#[derive(Debug, Error)]
pub enum CloudError {
    #[error("cloud transport failed: {0}")]
    Transport(#[from] reqwest::Error),
    #[error("cloud server returned {status}")]
    Server { status: u16 },
    #[error("cloud rejected the batch envelope with {status}")]
    BadRequest { status: u16 },
    #[error("cloud rate limited the edge")]
    RateLimited { retry_after: Option<Duration> },
    #[error("invalid cloud value: {0}")]
    Invalid(String),
}

impl Classify for CloudError {
    fn classify(&self) -> FailureKind {
        match self {
            Self::Transport(_) | Self::Server { .. } | Self::RateLimited { .. } => {
                FailureKind::Transient
            }
            Self::BadRequest { .. } | Self::Invalid(_) => FailureKind::Permanent,
        }
    }
}

/// Reusable connection-pooled cloud client, owned only by the drain task.
#[derive(Clone)]
pub struct CloudClient {
    base: Url,
    http: Client,
    edge_id: String,
}

impl CloudClient {
    /// Builds a client with the configured whole-request timeout.
    pub fn new(
        base: &str,
        edge_id: impl Into<String>,
        timeout: Duration,
    ) -> Result<Self, CloudError> {
        let base = Url::parse(base).map_err(|e| CloudError::Invalid(e.to_string()))?;
        let http = Client::builder().timeout(timeout).build()?;
        Ok(Self {
            base,
            http,
            edge_id: edge_id.into(),
        })
    }

    /// Sends one already-selected outbox batch.
    pub async fn send_batch(&self, events: &[OutboxEvent]) -> Result<Vec<EventResult>, CloudError> {
        let url = self
            .base
            .join(&format!("api/v1/edges/{}/events", self.edge_id))
            .map_err(|e| CloudError::Invalid(e.to_string()))?;
        let response = self.http.post(url).json(&Batch { events }).send().await?;
        let status = response.status();
        if status == StatusCode::TOO_MANY_REQUESTS {
            let retry_after = response
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .and_then(|value| {
                    // HTTP-date Retry-After values are relative to wall time;
                    // this client is outside every Edge decision path.
                    #[allow(clippy::disallowed_methods)]
                    let now = std::time::SystemTime::now();
                    parse_retry_after(value, now)
                });
            return Err(CloudError::RateLimited { retry_after });
        }
        if status.is_server_error() {
            return Err(CloudError::Server {
                status: status.as_u16(),
            });
        }
        if status.is_client_error() {
            return Err(CloudError::BadRequest {
                status: status.as_u16(),
            });
        }
        Ok(response.json::<BatchResult>().await?.results)
    }
}

fn parse_retry_after(value: &str, now: std::time::SystemTime) -> Option<Duration> {
    if let Ok(seconds) = value.trim().parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }
    httpdate::parse_http_date(value)
        .ok()
        .map(|deadline| deadline.duration_since(now).unwrap_or(Duration::ZERO))
}

/// Converts integer milliseconds to RFC 3339 UTC without precision loss.
pub fn millis_to_rfc3339(value: i64) -> Result<String, CloudError> {
    DateTime::<Utc>::from_timestamp_millis(value)
        .map(|v| v.to_rfc3339_opts(SecondsFormat::Millis, true))
        .ok_or_else(|| CloudError::Invalid(format!("timestamp {value} is out of range")))
}

/// Converts RFC 3339 back to exact integer milliseconds.
pub fn rfc3339_to_millis(value: &str) -> Result<i64, CloudError> {
    DateTime::parse_from_rfc3339(value)
        .map(|v| v.timestamp_millis())
        .map_err(|e| CloudError::Invalid(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    proptest! {
        #[test]
        fn time_roundtrip_is_exact_to_the_millisecond(ms in -62_135_596_800_000i64..253_402_300_799_999i64) {
            let encoded = millis_to_rfc3339(ms).unwrap();
            prop_assert!(encoded.ends_with('Z'));
            prop_assert_eq!(rfc3339_to_millis(&encoded).unwrap(), ms);
        }
    }
    #[test]
    fn time_boundaries_and_subseconds_are_preserved() {
        for value in [
            -2_208_988_799_123,
            -1,
            0,
            1_787_999_999_987,
            253_402_300_799_999,
        ] {
            assert_eq!(
                rfc3339_to_millis(&millis_to_rfc3339(value).unwrap()).unwrap(),
                value
            );
        }
    }
    #[test]
    fn every_error_is_classified() {
        assert_eq!(
            CloudError::Server { status: 503 }.classify(),
            FailureKind::Transient
        );
        assert_eq!(
            CloudError::RateLimited {
                retry_after: Some(Duration::from_secs(4))
            }
            .classify(),
            FailureKind::Transient
        );
        assert_eq!(
            CloudError::BadRequest { status: 400 }.classify(),
            FailureKind::Permanent
        );
        assert_eq!(
            CloudError::Invalid("bad".into()).classify(),
            FailureKind::Permanent
        );
    }
    #[test]
    fn retry_after_delta_seconds_is_parsed() {
        let now = std::time::UNIX_EPOCH + Duration::from_secs(1_000_000);
        assert_eq!(
            parse_retry_after("120", now),
            Some(Duration::from_secs(120))
        );
        let deadline = now + Duration::from_secs(45);
        assert_eq!(
            parse_retry_after(&httpdate::fmt_http_date(deadline), now),
            Some(Duration::from_secs(45))
        );
        assert_eq!(parse_retry_after("not-a-delay", now), None);
    }
}
