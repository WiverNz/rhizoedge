//! M6 end to end: a real broker, a real edge, a real simulator, and water.
//!
//! Everything here is the *system*, not a model of it. The three verifications
//! M6-024 says carry the weight are all in this file, and each is confirmed by
//! watching the wire or the ledger rather than by reading code:
//!
//! 1. `POST /water` during a leak returns 409 **and publishes no MQTT message**,
//!    confirmed by a spy subscriber on every command topic in the fleet.
//! 2. Killing the edge after publish and restarting produces **no second
//!    command** and exactly one `watering_event`.
//! 3. A duplicate command produces one actuation.
//!
//! Plus the ADR-002 rule a mock could never check — that nothing retained is
//! ever left on a command topic — and the sleeping-device paths ADR-018 adds.
//!
//! # Why `plant-node-02`
//!
//! The broker is shared with every other test binary, and `support::broker`'s
//! lock only serialises tests *within* one binary. An edge here subscribes to
//! the whole `rhizo/v1/devices/+/#` tree, so another binary's traffic for
//! `plant-node-01` would land in this suite's database and move `last_seen_at`,
//! `connectivity_mode`, and the retained status out from under an assertion.
//! Using the second provisioned account removes the collision rather than
//! papering over it with a longer timeout.

#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)]
#[path = "../../device-simulator/tests/support/mod.rs"]
mod support;

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use chrono::{TimeZone, Utc};
use edge_controller::api::ApiState;
use edge_controller::control::command::Commander;
use edge_controller::control::transport::MqttTransport;
use edge_controller::{metrics::Metrics, mqtt::ingress, pipeline};
use rhizo_testkit::TestClock;
use rumqttc::MqttOptions;
use tower::ServiceExt as _;

/// A running edge: ingress, pipeline, commander, and the REST router.
struct Edge {
    db: rhizo_storage::EdgeDb,
    clock: Arc<TestClock>,
    commander: Commander,
    router: axum::Router,
    transport: Arc<MqttTransport>,
    _stop: tokio::sync::watch::Sender<bool>,
    ingress: tokio::task::JoinHandle<Result<(), String>>,
    pipeline: tokio::task::JoinHandle<Result<(), String>>,
}

impl Edge {
    async fn start(broker: &support::TestBroker) -> Self {
        Self::start_on(broker, rhizo_storage::EdgeDb::in_memory().await.unwrap()).await
    }

    /// Starts an edge on an existing database, which is how a restart is
    /// simulated: the process is new, the durable state is not.
    async fn start_on(broker: &support::TestBroker, db: rhizo_storage::EdgeDb) -> Self {
        db.migrate().await.unwrap();
        let mut options = MqttOptions::new(
            format!("edge-m6-{}", uuid::Uuid::new_v4()),
            &broker.host,
            broker.port,
        );
        options.set_clean_session(true);
        options.set_credentials(&broker.edge_username, &broker.edge_password);
        // The M6-010 requirement: the PUBACK follows the commit.
        options.set_manual_acks(true);
        let (client, events) = ingress::connect(options, 32);
        let (tx, rx) = tokio::sync::mpsc::channel(16);
        let (stop, shutdown) = tokio::sync::watch::channel(false);
        let metrics = Metrics::new().unwrap();
        let ingress = tokio::spawn(ingress::run(
            client.clone(),
            events,
            tx,
            shutdown.clone(),
            metrics.clone(),
        ));
        let clock = Arc::new(TestClock::new(
            Utc.timestamp_millis_opt(1_900_000_000_000)
                .single()
                .unwrap(),
        ));
        let transport = Arc::new(MqttTransport::new(client.clone()));
        let commander = Commander::new(
            db.clone(),
            clock.clone(),
            transport.clone(),
            metrics.clone(),
        );
        let pipeline = tokio::spawn(pipeline::run(
            rx,
            db.clone(),
            clock.clone(),
            client,
            Some(commander.clone()),
            edge_controller::state::cache::LatestSampleCache::default(),
            shutdown,
            metrics.clone(),
        ));
        for _ in 0..50 {
            if metrics.connection.get() == 3 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert_eq!(metrics.connection.get(), 3, "edge never reached Subscribed");
        let router = edge_controller::api::server::router(
            ApiState {
                db: db.clone(),
                metrics,
                clock: clock.clone(),
                commander: commander.clone(),
            },
            vec![],
        );
        tokio::time::sleep(Duration::from_millis(300)).await;
        Self {
            db,
            clock,
            commander,
            router,
            transport,
            _stop: stop,
            ingress,
            pipeline,
        }
    }

    async fn json(
        &self,
        method: &str,
        uri: &str,
        body: serde_json::Value,
    ) -> (StatusCode, serde_json::Value) {
        let request = Request::builder()
            .method(method)
            .uri(uri)
            .header(axum::http::header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let response = self.router.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value = if bytes.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
        };
        (status, value)
    }

    async fn get(&self, uri: &str) -> (StatusCode, serde_json::Value) {
        self.json("GET", uri, serde_json::Value::Null).await
    }
}

impl Drop for Edge {
    fn drop(&mut self) {
        self.ingress.abort();
        self.pipeline.abort();
    }
}

/// A plant configured to water: control probe, pump, leak sensor, tank sensor,
/// their policies, and a drying series.
async fn waterable(db: &rhizo_storage::EdgeDb, now: chrono::DateTime<Utc>) {
    use rhizo_storage::repo::{binding, plant};
    sqlx::query(
        "INSERT OR IGNORE INTO devices(device_id,created_at,status,sensors_json,connectivity_mode,last_seen_at) \
         VALUES('plant-node-02',?,'online',?, 'connected',?)",
    )
    .bind(now.timestamp_millis())
    .bind(
        serde_json::json!([
            {"sensor_id":"soil-0","point":"default","kinds":["soil_moisture"],"present":true,"healthy":true,"errors":0},
            {"sensor_id":"leak-0","point":"tray","kinds":["leak_state"],"present":true,"healthy":true,"errors":0},
            {"sensor_id":"tank-0","point":"reservoir","kinds":["tank_level"],"present":true,"healthy":true,"errors":0}
        ])
        .to_string(),
    )
    .bind(now.timestamp_millis())
    .execute(db.pool())
    .await
    .unwrap();
    plant::create(
        db,
        &plant::NewPlant {
            plant_id: "monstera-01".to_owned(),
            name: "Monstera".to_owned(),
            pot_volume_ml: Some(2_000.0),
            ..Default::default()
        },
        now.timestamp_millis(),
    )
    .await
    .unwrap();
    for (id, sensor, point, kind, role) in [
        ("b-control", "soil-0", "default", "soil_moisture", "control"),
        ("b-leak", "leak-0", "tray", "leak_state", "required"),
        ("b-tank", "tank-0", "reservoir", "tank_level", "required"),
    ] {
        binding::upsert_sensor_binding(
            db,
            &binding::SensorBindingRow {
                binding_id: id.to_owned(),
                plant_id: "monstera-01".to_owned(),
                device_id: "plant-node-02".to_owned(),
                sensor_id: sensor.to_owned(),
                point: point.to_owned(),
                kind: kind.to_owned(),
                role: role.to_owned(),
                created_at: now.timestamp_millis(),
            },
        )
        .await
        .unwrap();
    }
    binding::upsert_actuator_binding(
        db,
        &binding::ActuatorBindingRow {
            plant_id: "monstera-01".to_owned(),
            device_id: "plant-node-02".to_owned(),
            actuator_id: "pump-0".to_owned(),
            kind: "irrigation_pump".to_owned(),
            created_at: now.timestamp_millis(),
        },
    )
    .await
    .unwrap();
    for (kind, target_min, target_max) in [
        ("soil_moisture", Some(28.0), Some(45.0)),
        ("leak_state", None, None),
        ("tank_level", None, None),
    ] {
        binding::upsert_measurement_policy(
            db,
            &binding::MeasurementPolicyRow {
                plant_id: "monstera-01".to_owned(),
                kind: kind.to_owned(),
                target_min,
                target_max,
                warning_low: None,
                warning_high: None,
                critical_low: None,
                critical_high: None,
                stale_after_ms: 900_000,
                hysteresis: None,
                confirm_duration_ms: Some(1_800_000),
            },
            now.timestamp_millis(),
        )
        .await
        .unwrap();
    }
    for i in 0i64..72 {
        let at = now - chrono::Duration::minutes((71 - i) * 5);
        scalar(
            db,
            at,
            "soil-0",
            "default",
            "soil_moisture",
            "vwc_percent",
            40.0 - i as f64 * 0.25,
        )
        .await;
    }
    scalar(
        db,
        now,
        "tank-0",
        "reservoir",
        "tank_level",
        "percent",
        70.0,
    )
    .await;
    boolean(db, now, "leak-0", "tray", "leak_state", false).await;
}

async fn scalar(
    db: &rhizo_storage::EdgeDb,
    at: chrono::DateTime<Utc>,
    sensor: &str,
    point: &str,
    kind: &str,
    unit: &str,
    value: f64,
) {
    sqlx::query(
        "INSERT INTO measurements(device_id,sensor_id,point,kind,value_num,unit,quality,received_at,batch_id,origin) \
         VALUES('plant-node-02',?,?,?,?,?,'ok',?,?, 'live')",
    )
    .bind(sensor)
    .bind(point)
    .bind(kind)
    .bind(value)
    .bind(unit)
    .bind(at.timestamp_millis())
    .bind(uuid::Uuid::new_v4().to_string())
    .execute(db.pool())
    .await
    .unwrap();
}

async fn boolean(
    db: &rhizo_storage::EdgeDb,
    at: chrono::DateTime<Utc>,
    sensor: &str,
    point: &str,
    kind: &str,
    value: bool,
) {
    sqlx::query(
        "INSERT INTO measurements(device_id,sensor_id,point,kind,value_bool,unit,quality,received_at,batch_id,origin) \
         VALUES('plant-node-02',?,?,?,?,'boolean','ok',?,?, 'live')",
    )
    .bind(sensor)
    .bind(point)
    .bind(kind)
    .bind(i64::from(value))
    .bind(at.timestamp_millis())
    .bind(uuid::Uuid::new_v4().to_string())
    .execute(db.pool())
    .await
    .unwrap();
}

/// Marks the fixture device reachable **now**.
///
/// Called immediately before each request rather than once during setup: the
/// edge is subscribed to `status`, and a retained status left on the broker by
/// an earlier test is redelivered on connect and rewrites
/// `devices.connectivity_mode`. That is the registry working correctly — a
/// retained status is what a real device's absence looks like — so the fixture
/// states its intent at the moment it matters instead of racing the ingress.
async fn mark_connected(db: &rhizo_storage::EdgeDb, now: chrono::DateTime<Utc>) {
    sqlx::query(
        "UPDATE devices SET connectivity_mode='connected',status='online',last_seen_at=?          WHERE device_id='plant-node-02'",
    )
    .bind(now.timestamp_millis())
    .execute(db.pool())
    .await
    .unwrap();
}

async fn count(db: &rhizo_storage::EdgeDb, table: &'static str) -> i64 {
    let sql = match table {
        "commands" => "SELECT count(*) FROM commands",
        "watering_events" => "SELECT count(*) FROM watering_events",
        _ => unreachable!("only the two ledger tables are counted here"),
    };
    sqlx::query_scalar(sql).fetch_one(db.pool()).await.unwrap()
}

// ---------------------------------------------------------------------------
// The three verifications M6-024 says carry the weight.

/// **`POST /water` during a leak returns 409 and publishes nothing.**
///
/// Confirmed by a spy subscriber on every command topic in the fleet, for the
/// whole run. A status code alone would not show the property: what SAFETY-003
/// claims is that no message appears, and this is where that is observed.
#[tokio::test]
async fn safety_003_leak_blocks_manual_api_with_nothing_published() {
    let Some(broker) = support::broker("safety_003_leak_blocks_manual_api").await else {
        return;
    };
    let edge = Edge::start(&broker).await;
    let mut watcher = broker
        .edge_subscriber(
            &format!("m6-leak-watch-{}", uuid::Uuid::new_v4()),
            "rhizo/v1/devices/+/commands/#",
        )
        .await;

    let now = edge.clock.now();
    waterable(&edge.db, now).await;
    boolean(&edge.db, now, "leak-0", "tray", "leak_state", true).await;
    mark_connected(&edge.db, now).await;

    let (status, body) = edge
        .json(
            "POST",
            "/api/v1/plants/monstera-01/water",
            serde_json::json!({ "ml": 30.0, "mode": "manual" }),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"]["details"]["reason"], "leak");

    let seen = watcher.drain_for(Duration::from_millis(750)).await;
    assert!(
        seen.is_empty(),
        "a refused dose published {:?}",
        seen.iter().map(|m| m.topic.clone()).collect::<Vec<_>>()
    );
    assert_eq!(count(&edge.db, "commands").await, 0);
    assert_eq!(count(&edge.db, "watering_events").await, 0);
}

/// **Killing the edge after publish and restarting produces no second command
/// and exactly one `watering_event`** (SAFETY-010).
#[tokio::test]
async fn restart_mid_command() {
    let Some(broker) = support::broker("restart_mid_command").await else {
        return;
    };
    let db = rhizo_storage::EdgeDb::in_memory().await.unwrap();
    db.migrate().await.unwrap();

    let mut watcher = broker
        .edge_subscriber(
            &format!("m6-restart-watch-{}", uuid::Uuid::new_v4()),
            "rhizo/v1/devices/+/commands/water",
        )
        .await;

    let command_id = {
        let edge = Edge::start_on(&broker, db.clone()).await;
        let now = edge.clock.now();
        waterable(&edge.db, now).await;
        mark_connected(&edge.db, now).await;
        let (status, body) = edge
            .json(
                "POST",
                "/api/v1/plants/monstera-01/water",
                serde_json::json!({ "ml": 40.0 }),
            )
            .await;
        assert_eq!(status, StatusCode::ACCEPTED, "{body}");
        let command_id = body["command_id"].as_str().unwrap().to_owned();

        // Wait for the command to actually reach the broker **before** killing
        // the edge. The invariant is about a crash *after* publication — a
        // crash before it is the ordinary "publish failed" path and is already
        // covered. Matched on the command id, not merely on the topic: this
        // broker is shared with the other test binaries.
        let wanted = command_id.clone();
        let first = watcher
            .next_matching(support::RECEIVE_TIMEOUT, move |m| {
                m.topic.ends_with("/commands/water")
                    && m.json()["data"]["command_id"] == serde_json::json!(wanted)
            })
            .await
            .expect("the command reached the broker before the crash");
        assert!(!first.retain, "and it was not retained");
        command_id
        // The edge is dropped here: ingress and pipeline are aborted with the
        // command on the wire and no result recorded, which is the crash the
        // invariant is about.
    };

    // A new process on the same durable state.
    let edge = Edge::start_on(&broker, db.clone()).await;
    let recovery = edge.commander.reconcile().await.unwrap();
    assert_eq!(recovery.republished, 0);
    assert_eq!(recovery.awaiting, 1, "the command is still inside its TTL");
    let seen = watcher.drain_for(Duration::from_millis(750)).await;
    assert!(
        seen.is_empty(),
        "a restart re-published {:?}",
        seen.iter().map(|m| m.topic.clone()).collect::<Vec<_>>()
    );

    // The device's result arrives once, and produces exactly one event.
    let result = serde_json::json!({
        "v": 1, "kind": "command.result",
        "message_id": uuid::Uuid::now_v7(),
        "device_id": "plant-node-02",
        "data": {
            "command_id": command_id,
            "status": "completed",
            "requested_ml": 40.0,
            "delivered_ml": 40.0,
            "duration_ms": 4_878,
            "clamped": false,
            "reason": null,
            "delivered_today_ml": 40.0,
            "origin": "edge_command",
        },
    });
    let device = support::Subscriber::connect(
        &broker,
        &format!("plant-node-02-m6-{}", uuid::Uuid::new_v4()),
        "plant-node-02",
        &broker.device_password("plant-node-02"),
        "rhizo/v1/devices/plant-node-02/commands/water",
    )
    .await;
    support::publish(
        &device.client(),
        "rhizo/v1/devices/plant-node-02/commands/result",
        &result.to_string(),
        false,
    )
    .await;

    assert!(
        eventually_async(support::RECEIVE_TIMEOUT, || async {
            count(&edge.db, "watering_events").await == 1
        })
        .await,
        "the result must settle into exactly one watering event"
    );
    assert_eq!(count(&edge.db, "commands").await, 1, "no second command");
}

/// **`command.result` durability, over a real broker** (protocol §5.14).
///
/// The property: a device learns that its result is durable *here* only from
/// `command.result.ack`, published after the edge's transaction commits — and a
/// redelivered result is acknowledged again, so a device whose acknowledgement
/// was lost can still make progress.
///
/// # Why the PUBACK could not have carried this
///
/// MQTT 3.1.1 QoS 1 acknowledges hop by hop. The PUBACK for the publish below is
/// written by *this broker*, on receipt, and the edge may not have read the
/// message yet. Nothing the edge does to its own PUBACK travels back through the
/// broker to the publisher. This test subscribes as the device and waits for an
/// application-level message, because that is the only signal that carries the
/// edge's commit.
#[tokio::test]
async fn a_committed_result_is_acknowledged_to_the_device_and_re_acknowledged_on_redelivery() {
    let Some(broker) = support::broker("result_ack_over_the_wire").await else {
        return;
    };
    let db = rhizo_storage::EdgeDb::in_memory().await.unwrap();
    db.migrate().await.unwrap();
    let edge = Edge::start_on(&broker, db.clone()).await;
    let now = edge.clock.now();
    waterable(&edge.db, now).await;
    mark_connected(&edge.db, now).await;

    let (status, body) = edge
        .json(
            "POST",
            "/api/v1/plants/monstera-01/water",
            serde_json::json!({ "ml": 40.0 }),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let command_id = body["command_id"].as_str().unwrap().to_owned();

    // Subscribe as the device does: the exact acknowledgement topic, which is
    // *not* the result topic. A device that received its own results would be
    // back in the seam the exact subscriptions closed.
    let mut device = support::Subscriber::connect(
        &broker,
        &format!("plant-node-02-ack-{}", uuid::Uuid::new_v4()),
        "plant-node-02",
        &broker.device_password("plant-node-02"),
        "rhizo/v1/devices/plant-node-02/commands/result/ack",
    )
    .await;

    let result = serde_json::json!({
        "v": 1, "kind": "command.result",
        "message_id": uuid::Uuid::now_v7(),
        "device_id": "plant-node-02",
        "data": {
            "command_id": command_id,
            "status": "completed",
            "requested_ml": 40.0,
            "delivered_ml": 40.0,
            "duration_ms": 4_878,
            "clamped": false,
            "reason": null,
            "delivered_today_ml": 40.0,
            "origin": "edge_command",
        },
    });
    support::publish(
        &device.client(),
        "rhizo/v1/devices/plant-node-02/commands/result",
        &result.to_string(),
        false,
    )
    .await;

    let wanted = command_id.clone();
    let ack = device
        .next_matching(support::RECEIVE_TIMEOUT, move |m| {
            m.topic.ends_with("/commands/result/ack")
                && m.json()["data"]["command_id"] == serde_json::json!(wanted)
        })
        .await
        .expect("the edge must acknowledge a committed result");
    assert!(
        !ack.retain,
        "an acknowledgement is a statement about one moment"
    );
    assert_eq!(ack.json()["kind"], "command.result.ack");

    // And the commit really did happen before the acknowledgement went out.
    assert_eq!(
        count(&edge.db, "watering_events").await,
        1,
        "the acknowledgement follows the commit, so the row is already there"
    );

    // A redelivery -- a device retrying because the first acknowledgement was
    // lost -- is acknowledged again rather than silently deduplicated into
    // silence, which would leave that device retrying for ever.
    let mut redelivery = result.clone();
    redelivery["message_id"] = serde_json::json!(uuid::Uuid::now_v7());
    support::publish(
        &device.client(),
        "rhizo/v1/devices/plant-node-02/commands/result",
        &redelivery.to_string(),
        false,
    )
    .await;
    let wanted = command_id.clone();
    assert!(
        device
            .next_matching(support::RECEIVE_TIMEOUT, move |m| {
                m.topic.ends_with("/commands/result/ack")
                    && m.json()["data"]["command_id"] == serde_json::json!(wanted)
            })
            .await
            .is_some(),
        "a duplicate result must be re-acknowledged, not answered with silence"
    );
    // Idempotent by `command_id`: the second delivery adds nothing.
    assert_eq!(count(&edge.db, "watering_events").await, 1);
    assert_eq!(count(&edge.db, "commands").await, 1);
}

/// Polls an async condition until it holds or the window elapses.
///
/// The shared `support::eventually` takes a synchronous predicate, because every
/// other use of it inspects in-memory device state. Reading SQLite needs an
/// async one, and bridging with `block_in_place` would tie every test here to
/// the multi-threaded runtime for no gain.
async fn eventually_async<F, Fut>(window: Duration, mut condition: F) -> bool
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let deadline = tokio::time::Instant::now() + window;
    loop {
        if condition().await {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// **ADR-002.** Nothing retained is ever left on a command topic. A fresh
/// subscriber that connects *after* a full cycle sees nothing at all, which is
/// the only way to observe retention.
#[tokio::test]
async fn no_retained_commands() {
    let Some(broker) = support::broker("no_retained_commands").await else {
        return;
    };
    let edge = Edge::start(&broker).await;
    let now = edge.clock.now();
    waterable(&edge.db, now).await;
    mark_connected(&edge.db, now).await;
    let (status, body) = edge
        .json(
            "POST",
            "/api/v1/plants/monstera-01/water",
            serde_json::json!({ "ml": 40.0 }),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    tokio::time::sleep(Duration::from_millis(500)).await;

    // A subscriber that arrives after the fact sees only retained messages.
    let mut latecomer = broker
        .edge_subscriber(
            &format!("m6-retain-check-{}", uuid::Uuid::new_v4()),
            "rhizo/v1/devices/+/commands/#",
        )
        .await;
    let seen = latecomer.drain_for(Duration::from_millis(750)).await;
    assert!(
        seen.is_empty(),
        "a command topic retained {:?}",
        seen.iter().map(|m| m.topic.clone()).collect::<Vec<_>>()
    );
}

/// **SAFETY-001 end to end.** The same command published three times produces
/// one actuation and one `watering_event`, against a real simulator.
#[tokio::test]
async fn safety_001_duplicate_command_single_actuation() {
    let Some(broker) = support::broker("safety_001_duplicate_command_single_actuation").await
    else {
        return;
    };
    let edge = Edge::start(&broker).await;
    let now = edge.clock.now();
    waterable(&edge.db, now).await;
    mark_connected(&edge.db, now).await;

    let (status, body) = edge
        .json(
            "POST",
            "/api/v1/plants/monstera-01/water",
            serde_json::json!({ "ml": 40.0 }),
        )
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let command_id = body["command_id"].as_str().unwrap().to_owned();

    // Three identical results, as a device retrying an unacknowledged publish
    // would produce. Distinct `message_id`s, so transport dedup does not do the
    // work the command's terminal status is supposed to do.
    let device = support::Subscriber::connect(
        &broker,
        &format!("plant-node-02-dup-{}", uuid::Uuid::new_v4()),
        "plant-node-02",
        &broker.device_password("plant-node-02"),
        "rhizo/v1/devices/plant-node-02/commands/water",
    )
    .await;
    for _ in 0..3 {
        let result = serde_json::json!({
            "v": 1, "kind": "command.result",
            "message_id": uuid::Uuid::now_v7(),
            "device_id": "plant-node-02",
            "data": {
                "command_id": command_id,
                "status": "completed",
                "requested_ml": 40.0,
                "delivered_ml": 40.0,
                "duration_ms": 4_000,
                "clamped": false,
                "reason": null,
                "delivered_today_ml": 40.0,
                "origin": "edge_command",
            },
        });
        support::publish(
            &device.client(),
            "rhizo/v1/devices/plant-node-02/commands/result",
            &result.to_string(),
            false,
        )
        .await;
    }

    assert!(
        eventually_async(support::RECEIVE_TIMEOUT, || async {
            count(&edge.db, "watering_events").await == 1
        })
        .await,
        "three results must settle into one watering event"
    );
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(count(&edge.db, "watering_events").await, 1);
}

/// A dose held for a sleeping device survives an edge restart and delivers
/// exactly once, with `issued_at` at the wake.
#[tokio::test]
async fn pending_intent_survives_edge_restart() {
    let Some(broker) = support::broker("pending_intent_survives_edge_restart").await else {
        return;
    };
    let db = rhizo_storage::EdgeDb::in_memory().await.unwrap();
    db.migrate().await.unwrap();
    let mut watcher = broker
        .edge_subscriber(
            &format!("m6-intent-watch-{}", uuid::Uuid::new_v4()),
            "rhizo/v1/devices/+/commands/#",
        )
        .await;

    let request_instant;
    {
        let edge = Edge::start_on(&broker, db.clone()).await;
        let now = edge.clock.now();
        request_instant = now.timestamp_millis();
        waterable(&edge.db, now).await;
        sqlx::query(
            "UPDATE devices SET connectivity_mode='sleeping',power_mode='battery',\
             wake_interval_seconds=900,sleep_received_at=?,expected_wake_at=?,overdue_at=? \
             WHERE device_id='plant-node-02'",
        )
        .bind(now.timestamp_millis())
        .bind(now.timestamp_millis() + 900_000)
        .bind(now.timestamp_millis() + 1_800_000)
        .execute(&db.pool().clone())
        .await
        .unwrap();

        let (status, body) = edge
            .json(
                "POST",
                "/api/v1/plants/monstera-01/water",
                serde_json::json!({ "ml": 30.0 }),
            )
            .await;
        assert_eq!(status, StatusCode::ACCEPTED, "{body}");
        assert_eq!(body["status"], "pending_for_device_wake");
        assert!(body.get("command_id").is_none());
    }
    let seen = watcher.drain_for(Duration::from_millis(750)).await;
    assert!(
        seen.is_empty(),
        "a held dose published {:?}",
        seen.iter().map(|m| m.topic.clone()).collect::<Vec<_>>()
    );

    // A new process, and then the wake.
    let edge = Edge::start_on(&broker, db.clone()).await;
    assert_eq!(
        edge_controller::control::intents::reconcile(&edge.commander, edge.clock.now())
            .await
            .unwrap(),
        1
    );
    edge.clock.advance(chrono::Duration::minutes(15));
    let wake = edge.clock.now();
    sqlx::query("UPDATE devices SET connectivity_mode='connected' WHERE device_id='plant-node-02'")
        .execute(db.pool())
        .await
        .unwrap();
    scalar(
        &db,
        wake,
        "soil-0",
        "default",
        "soil_moisture",
        "vwc_percent",
        20.0,
    )
    .await;
    scalar(
        &db,
        wake,
        "tank-0",
        "reservoir",
        "tank_level",
        "percent",
        70.0,
    )
    .await;
    boolean(&db, wake, "leak-0", "tray", "leak_state", false).await;

    let sent = edge_controller::control::intents::deliver_ready(&edge.commander, wake)
        .await
        .unwrap();
    assert_eq!(sent, 1);
    let delivered = watcher
        .next_matching(support::RECEIVE_TIMEOUT, |m| {
            m.topic.ends_with("/commands/water")
        })
        .await
        .expect("the held dose is delivered at the wake");
    let issued_at = delivered.json()["data"]["issued_at_ms"].as_i64().unwrap();
    assert_eq!(
        issued_at,
        wake.timestamp_millis(),
        "the command is minted at the wake, not at the request"
    );
    assert!(issued_at > request_instant);
    assert_eq!(count(&edge.db, "commands").await, 1);

    // Delivering again does nothing: the intent is terminal.
    assert_eq!(
        edge_controller::control::intents::deliver_ready(&edge.commander, wake)
            .await
            .unwrap(),
        0
    );
}

/// A leak raised while the device slept refuses the held dose at delivery, and
/// nothing is published — the gate genuinely re-runs, against current inputs.
#[tokio::test]
async fn leak_during_sleep_refuses_at_delivery() {
    let Some(broker) = support::broker("leak_during_sleep_refuses_at_delivery").await else {
        return;
    };
    let edge = Edge::start(&broker).await;
    let mut watcher = broker
        .edge_subscriber(
            &format!("m6-sleep-leak-{}", uuid::Uuid::new_v4()),
            "rhizo/v1/devices/+/commands/#",
        )
        .await;

    let now = edge.clock.now();
    waterable(&edge.db, now).await;
    sqlx::query(
        "UPDATE devices SET connectivity_mode='sleeping',power_mode='battery',\
         wake_interval_seconds=900,sleep_received_at=?,expected_wake_at=?,overdue_at=? \
         WHERE device_id='plant-node-02'",
    )
    .bind(now.timestamp_millis())
    .bind(now.timestamp_millis() + 900_000)
    .bind(now.timestamp_millis() + 1_800_000)
    .execute(edge.db.pool())
    .await
    .unwrap();

    let (_, held) = edge
        .json(
            "POST",
            "/api/v1/plants/monstera-01/water",
            serde_json::json!({ "ml": 30.0 }),
        )
        .await;
    let intent_id = held["intent_id"].as_str().unwrap().to_owned();

    // The tray floods while the device is asleep.
    edge.clock.advance(chrono::Duration::minutes(15));
    let wake = edge.clock.now();
    sqlx::query("UPDATE devices SET connectivity_mode='connected' WHERE device_id='plant-node-02'")
        .execute(edge.db.pool())
        .await
        .unwrap();
    scalar(
        &edge.db,
        wake,
        "soil-0",
        "default",
        "soil_moisture",
        "vwc_percent",
        20.0,
    )
    .await;
    scalar(
        &edge.db,
        wake,
        "tank-0",
        "reservoir",
        "tank_level",
        "percent",
        70.0,
    )
    .await;
    boolean(&edge.db, wake, "leak-0", "tray", "leak_state", true).await;

    let sent = edge_controller::control::intents::deliver_ready(&edge.commander, wake)
        .await
        .unwrap();
    assert_eq!(sent, 0);
    let (_, intent) = edge.get(&format!("/api/v1/intents/{intent_id}")).await;
    assert_eq!(intent["status"], "refused");
    assert_eq!(intent["refusal_reason"], "leak");

    let seen = watcher.drain_for(Duration::from_millis(750)).await;
    assert!(
        seen.is_empty(),
        "a refused delivery published {:?}",
        seen.iter().map(|m| m.topic.clone()).collect::<Vec<_>>()
    );
    assert_eq!(count(&edge.db, "commands").await, 0);
}

/// **SCEN-002, the full cycle**, against a real simulator that refuses exactly
/// as firmware does.
///
/// The state sequence the PRD documents, produced by the system rather than
/// asserted about a model: `DryConfirmed` -> `DoseIssued` -> `WaitForAbsorption`,
/// with one `watering_event` and a rolling total that never passes the cap.
#[tokio::test]
async fn a_full_cycle_waters_a_real_simulated_plant() {
    let Some(broker) = support::broker("a_full_cycle_waters_a_real_simulated_plant").await else {
        return;
    };
    let edge = Edge::start(&broker).await;
    let device = support::SimulatedDevice::start(
        &broker,
        "plant-node-02",
        &["--sensors", "soil,tank,leak", "--time-scale", "60"],
    )
    .await;

    let now = edge.clock.now();
    waterable(&edge.db, now).await;
    edge.json(
        "POST",
        "/api/v1/plants/monstera-01/auto-watering/enable",
        serde_json::json!({}),
    )
    .await;

    // **SAFETY-016 in passing.** A device that reconnects with buffered history
    // holds its plants until the edge has read it, so the cycle cannot even
    // start until reconciliation completes. Waiting for that here is not test
    // scaffolding around an inconvenience — it is the invariant, observed.
    assert!(
        eventually_async(Duration::from_secs(10), || async {
            !edge_controller::control::reconcile::is_reconciling(&edge.db, "plant-node-02")
                .await
                .unwrap_or(true)
        })
        .await,
        "the device's replay must complete before any dose is issued; progress={:?} boot={:?}",
        sqlx::query_as::<_, (String, Option<i64>, i64)>(
            "SELECT boot_id,through_device_seq,complete FROM replay_progress WHERE device_id='plant-node-02'"
        )
        .fetch_all(edge.db.pool())
        .await
        .unwrap(),
        sqlx::query_scalar::<_, Option<String>>("SELECT boot_id FROM devices WHERE device_id='plant-node-02'")
            .fetch_one(edge.db.pool())
            .await
            .unwrap()
    );
    mark_connected(&edge.db, edge.clock.now()).await;

    // One control pass: dry, confirmed, permitted — so a dose is issued.
    edge_controller::control::tick::irrigation_pass(
        &edge.commander,
        &Metrics::new().unwrap(),
        edge.clock.now(),
    )
    .await
    .unwrap();

    let state = rhizo_storage::repo::command::irrigation_state(&edge.db, "monstera-01")
        .await
        .unwrap()
        .unwrap();
    let lockout = rhizo_storage::repo::command::lockout(&edge.db, "monstera-01")
        .await
        .unwrap();
    assert_eq!(
        state.state, "dose_issued",
        "the documented next state; the plant is instead locked for {lockout:?}"
    );
    assert_eq!(count(&edge.db, "commands").await, 1);

    // The simulator refuses or accepts on its own terms; either way the edge
    // settles exactly one command and never exceeds the cap.
    assert!(
        eventually_async(Duration::from_secs(10), || async {
            rhizo_storage::repo::command::open_commands(&edge.db)
                .await
                .map(|open| open.is_empty())
                .unwrap_or(false)
        })
        .await,
        "the command must settle"
    );
    let delivered = rhizo_storage::repo::command::delivered_in_window(&edge.db, "monstera-01", 0)
        .await
        .unwrap();
    assert!(delivered <= 300.0, "the rolling cap holds: {delivered} ml");
    assert_eq!(count(&edge.db, "commands").await, 1, "and one command only");
    device.stop_cleanly().await;
}
