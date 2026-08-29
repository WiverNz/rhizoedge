//! Shared scaffolding for the API unit tests.
//!
//! One in-memory database, one router, one clock a test can move. Nothing here
//! binds a socket or speaks to a broker: every M5 endpoint is reachable through
//! `tower`'s `oneshot`, which is also a small proof that none of them needs an
//! MQTT client.
#![allow(clippy::unwrap_used, clippy::expect_used, missing_docs)]
use std::sync::Arc;

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, Response, StatusCode},
};
use chrono::{DateTime, TimeZone, Utc};
use rhizo_storage::EdgeDb;
use tower::ServiceExt as _;

use super::ApiState;

/// A fixed instant every test starts from, so ages are exact.
#[must_use]
pub fn base() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 8, 29, 12, 0, 0).unwrap()
}

/// A test edge: a migrated database, a movable clock, and the real router.
pub struct TestApi {
    pub db: EdgeDb,
    pub clock: Arc<rhizo_testkit::TestClock>,
    pub state: ApiState,
    pub router: Router,
}

impl TestApi {
    pub async fn start() -> Self {
        let db = EdgeDb::in_memory().await.unwrap();
        db.migrate().await.unwrap();
        let clock = Arc::new(rhizo_testkit::TestClock::new(base()));
        let state = ApiState {
            db: db.clone(),
            metrics: crate::metrics::Metrics::new().unwrap(),
            clock: clock.clone(),
        };
        let router = super::server::router(state.clone(), vec![]);
        Self {
            db,
            clock,
            state,
            router,
        }
    }

    /// Registers `plant-node-01` with the capabilities the fixture declares, so
    /// bindings have something real to name (M4-011).
    pub async fn with_device(&self) -> &Self {
        let envelope: rhizo_mqtt_contract::Envelope<rhizo_mqtt_contract::payload::DeviceStatus> =
            rhizo_mqtt_contract::Envelope::from_json(include_bytes!(
                "../../../../test/fixtures/protocol/valid/status-with-capabilities.json"
            ))
            .unwrap();
        rhizo_storage::repo::ingest::persist_status(&self.db, &envelope, base().timestamp_millis())
            .await
            .unwrap();
        self
    }

    pub async fn send(&self, request: Request<Body>) -> (StatusCode, serde_json::Value) {
        let response: Response<Body> = self.router.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let value = if bytes.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
        };
        (status, value)
    }

    pub async fn get(&self, uri: &str) -> (StatusCode, serde_json::Value) {
        self.send(Request::get(uri).body(Body::empty()).unwrap())
            .await
    }

    pub async fn json(
        &self,
        method: &str,
        uri: &str,
        body: serde_json::Value,
    ) -> (StatusCode, serde_json::Value) {
        self.send(
            Request::builder()
                .method(method)
                .uri(uri)
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
    }

    pub async fn delete(&self, uri: &str) -> (StatusCode, serde_json::Value) {
        self.send(Request::delete(uri).body(Body::empty()).unwrap())
            .await
    }

    /// Creates a plant with no preset and no bindings.
    pub async fn plant(&self, plant_id: &str) -> serde_json::Value {
        let (status, value) = self
            .json(
                "POST",
                "/api/v1/plants",
                serde_json::json!({
                    "plant_id": plant_id,
                    "name": "Monstera",
                    "species": "Monstera deliciosa",
                    "pot_volume_ml": 2000.0,
                }),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{value}");
        value
    }

    /// Binds the fixture's soil probe as the control measurement.
    pub async fn bind_control(&self, plant_id: &str) -> serde_json::Value {
        let (status, value) = self
            .json(
                "PUT",
                &format!("/api/v1/plants/{plant_id}/bindings/sensors"),
                serde_json::json!({
                    "device_id": "plant-node-01",
                    "sensor_id": "soil-0",
                    "point": "default",
                    "kind": "soil_moisture",
                    "role": "control",
                }),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{value}");
        value
    }

    /// The moisture policy a hand-configured plant would have.
    pub async fn moisture_policy(&self, plant_id: &str) {
        let (status, value) = self
            .json(
                "PUT",
                &format!("/api/v1/plants/{plant_id}/measurement-policies/soil_moisture"),
                serde_json::json!({
                    "target_min": 28.0,
                    "target_max": 45.0,
                    "stale_after_ms": 900_000,
                    "confirm_duration_ms": 1_800_000,
                }),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{value}");
    }

    /// Records one soil-moisture reading at `at`.
    pub async fn sample(&self, at: DateTime<Utc>, moisture: f64) {
        self.sample_kind(at, "soil_moisture", "vwc_percent", moisture)
            .await;
    }

    pub async fn sample_kind(&self, at: DateTime<Utc>, kind: &str, unit: &str, value: f64) {
        sqlx::query(
            "INSERT INTO measurements(device_id,sensor_id,point,kind,value_num,unit,quality,received_at,batch_id,origin) \
             VALUES('plant-node-01','soil-0','default',?,?,?,'ok',?,?,'live')",
        )
        .bind(kind)
        .bind(value)
        .bind(unit)
        .bind(at.timestamp_millis())
        .bind(uuid::Uuid::new_v4().to_string())
        .execute(self.db.pool())
        .await
        .unwrap();
    }

    /// Runs one evaluation pass at the clock's current instant.
    pub async fn evaluate(&self, plant_id: &str) -> rhizo_domain::recommend::Recommendation {
        crate::control::tick::evaluate_plant(
            &self.db,
            plant_id,
            self.clock.as_ref(),
            &self.state.metrics,
        )
        .await
        .unwrap()
        .unwrap()
    }
}
