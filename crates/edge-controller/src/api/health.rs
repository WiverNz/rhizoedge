//! Liveness and readiness endpoints.
#![allow(missing_docs)]
use super::ApiState;
use axum::{Json, extract::State, http::StatusCode};
use serde::Serialize;

#[derive(Serialize)]
struct Check {
    status: &'static str,
}
#[derive(Serialize)]
struct Health {
    status: &'static str,
    migrations: Check,
    mqtt: Check,
    control_loop: Check,
}

pub async fn live() -> Json<serde_json::Value> {
    Json(serde_json::json!({"status":"ok"}))
}

pub async fn ready(State(state): State<ApiState>) -> (StatusCode, Json<impl Serialize>) {
    let subscribed = state.metrics.connection.get() == 3;
    let body = Health {
        status: if subscribed { "ready" } else { "not_ready" },
        migrations: Check { status: "ok" },
        mqtt: Check {
            status: if subscribed {
                "subscribed"
            } else {
                "disconnected"
            },
        },
        control_loop: Check {
            status: "not_applicable",
        },
    };
    (
        if subscribed {
            StatusCode::OK
        } else {
            StatusCode::SERVICE_UNAVAILABLE
        },
        Json(body),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    async fn state(connection: i64) -> ApiState {
        let db = rhizo_storage::EdgeDb::in_memory().await.unwrap();
        db.migrate().await.unwrap();
        let metrics = crate::metrics::Metrics::new().unwrap();
        metrics.connection.set(connection);
        let clock = std::sync::Arc::new(rhizo_testkit::TestClock::new(
            chrono::DateTime::from_timestamp_millis(1_000).unwrap(),
        ));
        ApiState {
            db: db.clone(),
            metrics: metrics.clone(),
            clock: clock.clone(),
            commander: crate::control::command::Commander::new(
                db,
                clock,
                std::sync::Arc::new(crate::control::transport::RecordingTransport::new()),
                metrics,
            ),
        }
    }
    /// Serialises the two readiness tests.
    ///
    /// `Metrics::new()` is a process-wide singleton, so `metrics.connection` is
    /// **one gauge shared by every test in this binary** — not a fixture each
    /// test owns. These two set it to different values and then read it back, so
    /// run concurrently they overwrite each other and either can fail. Observed
    /// as roughly one failure in three full workspace runs.
    ///
    /// A lock rather than a per-test registry, because the singleton is
    /// deliberate: metrics are a process-global surface and pretending otherwise
    /// in a test would be testing something the binary does not do. Anything
    /// else that asserts on a shared gauge must take this lock too.
    fn gauge_lock() -> &'static tokio::sync::Mutex<()> {
        static LOCK: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
        LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
    }

    #[tokio::test]
    async fn subscribed_is_ready_and_cloud_is_not_a_check() {
        let _guard = gauge_lock().lock().await;
        let (status, Json(body)) = ready(State(state(3).await)).await;
        assert_eq!(status, StatusCode::OK);
        let json = serde_json::to_value(body).unwrap();
        assert_eq!(json["mqtt"]["status"], "subscribed");
        assert!(json.get("cloud").is_none());
        assert_eq!(json["control_loop"]["status"], "not_applicable");
    }
    #[tokio::test]
    async fn safety_008_edge_readiness_stays_200_without_cloud() {
        let _guard = gauge_lock().lock().await;
        let (status, Json(body)) = ready(State(state(3).await)).await;
        assert_eq!(status, StatusCode::OK);
        assert!(serde_json::to_value(body).unwrap().get("cloud").is_none());
    }
    #[tokio::test]
    async fn disconnected_is_specific_and_unready() {
        let _guard = gauge_lock().lock().await;
        let (status, Json(body)) = ready(State(state(0).await)).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            serde_json::to_value(body).unwrap()["mqtt"]["status"],
            "disconnected"
        );
    }
}
