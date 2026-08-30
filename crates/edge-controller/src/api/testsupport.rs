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

/// A test edge: a migrated database, a movable clock, the real router, and a
/// transport that records every publish.
///
/// The recorder is the point. "The endpoint returned 409" is a weaker claim than
/// "and nothing appeared on a command topic", and the second is what SAFETY-003
/// actually says — so every refusal test below can assert it.
pub struct TestApi {
    pub db: EdgeDb,
    pub clock: Arc<rhizo_testkit::TestClock>,
    pub state: ApiState,
    pub router: Router,
    pub transport: crate::control::transport::RecordingTransport,
    pub commander: crate::control::command::Commander,
}

impl TestApi {
    pub async fn start() -> Self {
        let db = EdgeDb::in_memory().await.unwrap();
        db.migrate().await.unwrap();
        let clock = Arc::new(rhizo_testkit::TestClock::new(base()));
        let metrics = crate::metrics::Metrics::new().unwrap();
        let transport = crate::control::transport::RecordingTransport::new();
        let commander = crate::control::command::Commander::new(
            db.clone(),
            clock.clone(),
            Arc::new(transport.clone()),
            metrics.clone(),
        );
        let state = ApiState {
            db: db.clone(),
            metrics,
            clock: clock.clone(),
            commander: commander.clone(),
        };
        let router = super::server::router(state.clone(), vec![]);
        Self {
            db,
            clock,
            state,
            router,
            transport,
            commander,
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

    /// Binds a sensor of any kind and role.
    pub async fn bind(&self, plant_id: &str, sensor_id: &str, point: &str, kind: &str, role: &str) {
        let (status, value) = self
            .json(
                "PUT",
                &format!("/api/v1/plants/{plant_id}/bindings/sensors"),
                serde_json::json!({
                    "device_id": "plant-node-01",
                    "sensor_id": sensor_id,
                    "point": point,
                    "kind": kind,
                    "role": role,
                }),
            )
            .await;
        assert_eq!(status, StatusCode::CREATED, "{value}");
    }

    /// Binds the fixture's pump.
    pub async fn bind_actuator(&self, plant_id: &str) {
        let (status, value) = self
            .json(
                "PUT",
                &format!("/api/v1/plants/{plant_id}/bindings/actuator"),
                serde_json::json!({ "device_id": "plant-node-01", "actuator_id": "pump-0" }),
            )
            .await;
        assert!(status.is_success(), "{value}");
    }

    /// A per-kind policy with only a freshness horizon.
    pub async fn policy(&self, plant_id: &str, kind: &str, stale_after_ms: i64) {
        let (status, value) = self
            .json(
                "PUT",
                &format!("/api/v1/plants/{plant_id}/measurement-policies/{kind}"),
                serde_json::json!({ "stale_after_ms": stale_after_ms }),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{value}");
    }

    /// Records one boolean reading, which is how `leak_state` is carried.
    pub async fn sample_bool(
        &self,
        at: DateTime<Utc>,
        sensor: &str,
        point: &str,
        kind: &str,
        value: bool,
    ) {
        sqlx::query(
            "INSERT INTO measurements(device_id,sensor_id,point,kind,value_bool,unit,quality,received_at,batch_id,origin)              VALUES('plant-node-01',?,?,?,?,'boolean','ok',?,?,'live')",
        )
        .bind(sensor)
        .bind(point)
        .bind(kind)
        .bind(i64::from(value))
        .bind(at.timestamp_millis())
        .bind(uuid::Uuid::new_v4().to_string())
        .execute(self.db.pool())
        .await
        .unwrap();
    }

    /// Records one scalar reading from a named sensor and point.
    pub async fn sample_from(
        &self,
        at: DateTime<Utc>,
        sensor: &str,
        point: &str,
        kind: &str,
        unit: &str,
        value: f64,
    ) {
        sqlx::query(
            "INSERT INTO measurements(device_id,sensor_id,point,kind,value_num,unit,quality,received_at,batch_id,origin)              VALUES('plant-node-01',?,?,?,?,?,'ok',?,?,'live')",
        )
        .bind(sensor)
        .bind(point)
        .bind(kind)
        .bind(value)
        .bind(unit)
        .bind(at.timestamp_millis())
        .bind(uuid::Uuid::new_v4().to_string())
        .execute(self.db.pool())
        .await
        .unwrap();
    }

    /// A plant that can actually water: a control probe, a pump, a leak sensor
    /// reading clear, a tank reading full, and a drying series.
    ///
    /// Everything the gate needs is positively **present**, so a test that bends
    /// one input is testing that input and nothing else.
    pub async fn waterable(&self, plant_id: &str) {
        self.with_device().await;
        self.plant(plant_id).await;
        self.bind_control(plant_id).await;
        self.moisture_policy(plant_id).await;
        self.bind_actuator(plant_id).await;
        self.bind(plant_id, "leak-0", "tray", "leak_state", "required")
            .await;
        self.policy(plant_id, "leak_state", 900_000).await;
        self.bind(plant_id, "tank-0", "reservoir", "tank_level", "required")
            .await;
        self.policy(plant_id, "tank_level", 900_000).await;
        for i in 0i64..72 {
            let at = base() - chrono::Duration::minutes((71 - i) * 5);
            self.sample(at, 40.0 - i as f64 * 0.25).await;
        }
        self.sample_bool(base(), "leak-0", "tray", "leak_state", false)
            .await;
        self.sample_from(base(), "tank-0", "reservoir", "tank_level", "percent", 70.0)
            .await;
    }

    /// Marks the fixture device as awake and reachable.
    pub async fn device_connected(&self) {
        sqlx::query(
            "UPDATE devices SET connectivity_mode='connected',status='online',last_seen_at=? WHERE device_id='plant-node-01'",
        )
        .bind(base().timestamp_millis())
        .execute(self.db.pool())
        .await
        .unwrap();
    }

    /// Marks the fixture device as asleep inside an open, edge-computed window.
    pub async fn device_sleeping(&self, wake_in_ms: i64) {
        let now = self.clock.now().timestamp_millis();
        sqlx::query(
            "UPDATE devices SET connectivity_mode='sleeping',power_mode='battery',wake_interval_seconds=900,             sleep_received_at=?,expected_wake_at=?,overdue_at=? WHERE device_id='plant-node-01'",
        )
        .bind(now)
        .bind(now + wake_in_ms)
        .bind(now + wake_in_ms * 2)
        .execute(self.db.pool())
        .await
        .unwrap();
    }

    /// Runs one irrigation pass at the clock's current instant.
    pub async fn irrigate(&self, plant_id: &str) -> crate::control::irrigation::Pass {
        let loaded = crate::plant::load(&self.db, plant_id)
            .await
            .unwrap()
            .unwrap();
        let now = self.clock.now();
        let analysis = crate::plant::analyse(&self.db, &loaded, now).await.unwrap();
        crate::control::irrigation::run_pass(
            &self.commander,
            &loaded,
            analysis.inputs.dry_duration,
            rhizo_domain::irrigation::types::EvaluationMode::Automatic,
            now,
        )
        .await
        .unwrap()
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
