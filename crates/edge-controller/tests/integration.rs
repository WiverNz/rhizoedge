//! Real-Mosquitto M3 ingestion tests.
#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)]
#[path = "../../device-simulator/tests/support/mod.rs"]
mod support;
use chrono::{TimeZone, Utc};
use edge_controller::{metrics::Metrics, mqtt::ingress, pipeline};
use rhizo_testkit::TestClock;
use rumqttc::{MqttOptions, QoS};
use std::sync::Arc;
use std::time::Duration;

/// The device every assertion in this suite is about.
///
/// Named rather than implied: the broker is shared and the edge subscribes to
/// `+/status`, so a retained status from any other suite registers a second
/// device in this edge's table. An unscoped `SELECT ... FROM devices` then reads
/// whichever row SQLite happens to return, which made two of these tests depend
/// on leftover broker state instead of on the edge.
const DEVICE: &str = "plant-node-01";

struct EdgeHarness {
    db: rhizo_storage::EdgeDb,
    commander: edge_controller::control::command::Commander,
    clock: Arc<TestClock>,
    _stop: tokio::sync::watch::Sender<bool>,
    ingress: tokio::task::JoinHandle<Result<(), String>>,
    pipeline: tokio::task::JoinHandle<Result<(), String>>,
}
impl EdgeHarness {
    async fn start(b: &support::TestBroker) -> Self {
        let db = rhizo_storage::EdgeDb::in_memory().await.unwrap();
        db.migrate().await.unwrap();
        let mut o = MqttOptions::new(format!("edge-it-{}", uuid::Uuid::new_v4()), &b.host, b.port);
        o.set_clean_session(true);
        o.set_credentials(&b.edge_username, &b.edge_password);
        let (client, events) = ingress::connect(o, 32);
        let (tx, rx) = tokio::sync::mpsc::channel(8);
        let (stop, shutdown) = tokio::sync::watch::channel(false);
        let metrics = Metrics::new().unwrap();
        let ingress = tokio::spawn(ingress::run(
            client.clone(),
            events,
            tx,
            shutdown.clone(),
            metrics.clone(),
        ));
        let test_clock = Arc::new(TestClock::new(
            Utc.timestamp_millis_opt(1_900_000_000_000)
                .single()
                .unwrap(),
        ));
        let clock: Arc<dyn rhizo_domain::Clock> = test_clock.clone();
        let commander = edge_controller::control::command::Commander::new(
            db.clone(),
            clock.clone(),
            Arc::new(edge_controller::control::transport::MqttTransport::new(
                client.clone(),
            )),
            metrics.clone(),
        );
        let pipeline = tokio::spawn(pipeline::run(
            rx,
            db.clone(),
            clock,
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
        tokio::time::sleep(Duration::from_millis(300)).await;
        Self {
            db,
            commander,
            clock: test_clock,
            _stop: stop,
            ingress,
            pipeline,
        }
    }
}
impl Drop for EdgeHarness {
    fn drop(&mut self) {
        self.ingress.abort();
        self.pipeline.abort();
    }
}

async fn publish(b: &support::TestBroker, payload: &[u8]) {
    let mut peer = support::Subscriber::connect(
        b,
        &format!("plant-node-01-it-{}", uuid::Uuid::new_v4()),
        "plant-node-01",
        &b.device_password("plant-node-01"),
        "rhizo/v1/devices/plant-node-01/telemetry",
    )
    .await;
    peer.client()
        .publish(
            "rhizo/v1/devices/plant-node-01/telemetry",
            QoS::AtLeastOnce,
            false,
            payload,
        )
        .await
        .unwrap();
    assert!(
        peer.next_matching(Duration::from_secs(3), |m| m.topic.ends_with("/telemetry"))
            .await
            .is_some(),
        "broker did not deliver published telemetry"
    );
}

/// Publishes a replay and returns the acknowledgement, or `None` if the edge
/// deliberately published none within the window.
async fn publish_replay(
    b: &support::TestBroker,
    payload: &[u8],
    wait: Duration,
) -> Option<support::Received> {
    let mut peer = support::Subscriber::connect(
        b,
        &format!("plant-node-01-replay-it-{}", uuid::Uuid::new_v4()),
        "plant-node-01",
        &b.device_password("plant-node-01"),
        "rhizo/v1/devices/plant-node-01/events/ack",
    )
    .await;
    peer.client()
        .publish(
            "rhizo/v1/devices/plant-node-01/events",
            QoS::AtLeastOnce,
            false,
            payload,
        )
        .await
        .unwrap();
    peer.next_matching(wait, |m| m.topic.ends_with("/events/ack"))
        .await
}

async fn publish_replay_and_wait_ack(b: &support::TestBroker, payload: &[u8]) -> support::Received {
    publish_replay(b, payload, Duration::from_secs(5))
        .await
        .expect("commit must be followed by a live ACK")
}

/// Renumbers a replay fixture's events onto `seqs`, keeping every other field.
///
/// `device_seq` is zero-based and the shipped fixture is deliberately a
/// suffix (118-121, after a gap), so a test that wants a prefix has to say so.
fn replay_with_sequences(fixture: &[u8], seqs: &[u64]) -> Vec<u8> {
    let mut value: serde_json::Value = serde_json::from_slice(fixture).unwrap();
    value["message_id"] = serde_json::json!(uuid::Uuid::new_v4());
    let events = value["data"]["events"].as_array_mut().unwrap();
    assert!(seqs.len() <= events.len());
    events.truncate(seqs.len());
    for (event, seq) in events.iter_mut().zip(seqs) {
        event["device_seq"] = serde_json::json!(seq);
    }
    serde_json::to_vec(&value).unwrap()
}
async fn count(db: &rhizo_storage::EdgeDb) -> i64 {
    for _ in 0..50 {
        let n = sqlx::query_scalar("SELECT count(*) FROM measurements")
            .fetch_one(db.pool())
            .await
            .unwrap();
        if n > 0 {
            return n;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    0
}

#[tokio::test]
async fn retained_status() {
    let Some(b) = support::broker("retained_status").await else {
        return;
    };
    let mut device = support::Subscriber::connect(
        &b,
        &format!("status-publisher-{}", uuid::Uuid::new_v4()),
        "plant-node-01",
        &b.device_password("plant-node-01"),
        "rhizo/v1/devices/plant-node-01/time",
    )
    .await;
    device
        .client()
        .publish(
            "rhizo/v1/devices/plant-node-01/status",
            QoS::AtLeastOnce,
            true,
            include_bytes!("../../../test/fixtures/protocol/valid/status-with-capabilities.json")
                .as_slice(),
        )
        .await
        .unwrap();
    let edge = EdgeHarness::start(&b).await;
    for _ in 0..50 {
        let n = sqlx::query_scalar::<_, i64>("SELECT count(*) FROM devices WHERE status='online'")
            .fetch_one(edge.db.pool())
            .await
            .unwrap();
        if n == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM devices WHERE status='online'")
            .fetch_one(edge.db.pool())
            .await
            .unwrap(),
        1
    );
    assert!(
        device
            .next_matching(Duration::from_secs(3), |m| m.topic.ends_with("/time"))
            .await
            .is_some(),
        "every valid status receipt must produce edge.time"
    );
    let mut fresh = support::Subscriber::connect(
        &b,
        &format!("fresh-time-{}", uuid::Uuid::new_v4()),
        "plant-node-01",
        &b.device_password("plant-node-01"),
        "rhizo/v1/devices/plant-node-01/time",
    )
    .await;
    assert!(
        fresh
            .next_matching(Duration::from_millis(500), |m| m.topic.ends_with("/time"))
            .await
            .is_none(),
        "edge.time must never be retained"
    );
}

#[tokio::test]
async fn intentional_sleep_and_duplicate_each_receive_edge_time() {
    use rhizo_mqtt_contract::payload::{DeviceStatus, DeviceStatusValue, PowerMode, PowerStatus};
    let Some(b) = support::broker("intentional_sleep_time").await else {
        return;
    };
    let edge = EdgeHarness::start(&b).await;
    let mut device = support::Subscriber::connect(
        &b,
        &format!("sleep-publisher-{}", uuid::Uuid::new_v4()),
        "plant-node-01",
        &b.device_password("plant-node-01"),
        "rhizo/v1/devices/plant-node-01/time",
    )
    .await;
    let mut status: rhizo_mqtt_contract::Envelope<DeviceStatus> =
        rhizo_mqtt_contract::Envelope::from_json(include_bytes!(
            "../../../test/fixtures/protocol/valid/status-with-capabilities.json"
        ))
        .unwrap();
    status.message_id = rhizo_mqtt_contract::MessageId::from_uuid(uuid::Uuid::new_v4());
    status.sequence = Some(status.sequence.unwrap() + 1);
    status.data.status = DeviceStatusValue::Offline;
    status.data.reason = Some("sleeping".into());
    status.data.power = Some(Box::new(PowerStatus {
        mode: PowerMode::Battery,
        wake_interval_seconds: Some(900),
        expected_wake_ms: Some(u64::MAX),
        wake_reason: Some(rhizo_mqtt_contract::payload::WakeReason::Timer),
        battery_mv: None,
        awake_ms: None,
    }));
    let payload = status.to_json().unwrap();
    for _ in 0..2 {
        device
            .client()
            .publish(
                "rhizo/v1/devices/plant-node-01/status",
                QoS::AtLeastOnce,
                true,
                payload.clone(),
            )
            .await
            .unwrap();
        assert!(
            device
                .next_matching(Duration::from_secs(3), |m| m.topic.ends_with("/time"))
                .await
                .is_some(),
            "every valid status receipt, including a duplicate, needs edge.time"
        );
    }
    // Scoped to the device under test. The broker is shared and the edge
    // subscribes to `+/status`, so a retained status left behind by any other
    // suite registers a second device here and an unscoped `fetch_one` reads
    // whichever row SQLite returns first. That is what made this assertion
    // depend on broker state rather than on the edge's behaviour.
    let row = sqlx::query(
        "SELECT connectivity_mode,last_seen_at,expected_wake_at FROM devices WHERE device_id=?",
    )
    .bind(DEVICE)
    .fetch_one(edge.db.pool())
    .await
    .unwrap();
    use sqlx::Row as _;
    assert_eq!(row.get::<String, _>("connectivity_mode"), "sleeping");
    assert_eq!(row.get::<Option<i64>, _>("last_seen_at"), None);
    assert_eq!(
        row.get::<Option<i64>, _>("expected_wake_at"),
        Some(1_900_000_900_000)
    );
}

/// SAFETY-021 against a real broker: a retained sleep announcement redelivered
/// after a broker restart must not buy the device another wake interval.
///
/// This is the transport half of the restart story. `persistence true` means
/// Mosquitto keeps the retained status across the restart and hands it to the
/// edge again the moment it re-subscribes, so the edge sees a second copy of a
/// message it has already acted on. The dedup ledger is what stops that copy
/// from re-opening the window, and it is worth proving over the wire rather
/// than by calling `persist_status` twice: the redelivery here is produced by
/// the broker, on reconnect, exactly as it will be in the field.
#[tokio::test]
async fn safety_021_a_retained_sleep_redelivered_after_a_broker_restart_extends_nothing() {
    use rhizo_mqtt_contract::payload::{DeviceStatus, DeviceStatusValue, PowerMode, PowerStatus};
    use sqlx::Row as _;
    if std::env::var("RHIZO_REQUIRE_BROKER").is_err() {
        eprintln!("SKIPPING retained-sleep broker restart unless RHIZO_REQUIRE_BROKER=1");
        return;
    }
    let Some(b) = support::broker("retained_sleep_broker_restart").await else {
        return;
    };
    // A sibling test's retained online status would be replayed to this edge on
    // connect *and* again after the restart, and whichever of the two carries
    // the higher `(boot_generation, sequence)` would win the ordering -- a flake
    // whose cause appears nowhere in the assertion that reports it. The whole
    // point here is what the broker replays, so the retained state has to start
    // known.
    support::clear_device_retained(&b, "plant-node-01").await;
    let edge = EdgeHarness::start(&b).await;
    let mut device = support::Subscriber::connect(
        &b,
        &format!("sleep-restart-{}", uuid::Uuid::new_v4()),
        "plant-node-01",
        &b.device_password("plant-node-01"),
        "rhizo/v1/devices/plant-node-01/time",
    )
    .await;
    let mut status: rhizo_mqtt_contract::Envelope<DeviceStatus> =
        rhizo_mqtt_contract::Envelope::from_json(include_bytes!(
            "../../../test/fixtures/protocol/valid/status-sleeping.json"
        ))
        .unwrap();
    status.message_id = rhizo_mqtt_contract::MessageId::from_uuid(uuid::Uuid::new_v4());
    assert_eq!(status.data.status, DeviceStatusValue::Offline);
    assert_eq!(
        status.data.declared_power_mode(),
        Some(PowerMode::Battery),
        "the fixture must be a battery sleep announcement"
    );
    assert!(
        matches!(
            status.data.power.as_deref(),
            Some(PowerStatus {
                wake_interval_seconds: Some(900),
                ..
            })
        ),
        "the fixture must declare the 900 s interval this test asserts on"
    );
    device
        .client()
        .publish(
            "rhizo/v1/devices/plant-node-01/status",
            QoS::AtLeastOnce,
            true,
            status.to_json().unwrap(),
        )
        .await
        .unwrap();
    assert!(
        device
            .next_matching(Duration::from_secs(3), |m| m.topic.ends_with("/time"))
            .await
            .is_some(),
        "an accepted sleep announcement still gets a live edge.time"
    );

    let snapshot = |db: rhizo_storage::EdgeDb| async move {
        // Scoped: see the note in `intentional_sleep_and_duplicate_each_receive_edge_time`.
        let row = sqlx::query(
            "SELECT connectivity_mode,expected_wake_at,overdue_at,sleep_received_at FROM devices WHERE device_id=?",
        )
        .bind(DEVICE)
        .fetch_one(db.pool())
        .await
        .unwrap();
        (
            row.get::<String, _>("connectivity_mode"),
            row.get::<Option<i64>, _>("expected_wake_at"),
            row.get::<Option<i64>, _>("overdue_at"),
            row.get::<Option<i64>, _>("sleep_received_at"),
        )
    };
    let before = snapshot(edge.db.clone()).await;
    assert_eq!(
        before.0, "sleeping",
        "the announcement must have opened a window before the restart is meaningful"
    );
    assert!(before.1.is_some() && before.2.is_some());

    assert!(restart_broker().success());
    // Wait through rumqttc's bounded first retry, then let the broker replay the
    // retained status to the re-subscribed edge.
    tokio::time::sleep(Duration::from_secs(8)).await;

    // Without this the test could pass by the redelivery never happening, which
    // would prove nothing at all. A fresh subscriber gets exactly what the
    // reconnecting edge got: the announcement is still retained on the restarted
    // broker and is handed out on subscribe.
    let mut fresh = support::Subscriber::connect(
        &b,
        &format!("sleep-restart-witness-{}", uuid::Uuid::new_v4()),
        "plant-node-01",
        &b.device_password("plant-node-01"),
        "rhizo/v1/devices/plant-node-01/status",
    )
    .await;
    let replayed = fresh
        .next_matching(Duration::from_secs(5), |m| m.topic.ends_with("/status"))
        .await
        .expect("the restarted broker must still hold the retained announcement");
    let replayed = rhizo_mqtt_contract::Envelope::<DeviceStatus>::from_json(&replayed.payload)
        .expect("the replayed retained message must still decode");
    assert_eq!(replayed.message_id, status.message_id, "same message");
    assert_eq!(replayed.data.announced_sleep_interval_seconds(), Some(900));
    assert_eq!(
        edge_controller::metrics::Metrics::new()
            .unwrap()
            .connection
            .get(),
        3,
        "the edge must have reconnected and re-subscribed, so it received it too"
    );

    assert_eq!(
        snapshot(edge.db.clone()).await,
        before,
        "a redelivered retained announcement must not move the window"
    );
    for (kind, expected) in [("device_slept", 1), ("offline", 0)] {
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM device_events WHERE kind=? AND device_id='plant-node-01'"
            )
            .bind(kind)
            .fetch_one(edge.db.pool())
            .await
            .unwrap(),
            expected,
            "{kind} must be recorded exactly {expected} time(s) across the restart"
        );
    }
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM processed_messages WHERE kind='device.status' AND device_id=?"
        )
        .bind(DEVICE)
        .fetch_one(edge.db.pool())
        .await
        .unwrap(),
        1,
        "the redelivery is the same message and must be deduplicated, not re-applied"
    );
    assert_eq!(
        sqlx::query_scalar::<_, Option<i64>>("SELECT last_seen_at FROM devices WHERE device_id=?")
            .bind(DEVICE)
            .fetch_one(edge.db.pool())
            .await
            .unwrap(),
        None,
        "a sleep announcement is not a sighting"
    );
}

#[tokio::test]
async fn auto_registration_creates_no_plant() {
    let db = rhizo_storage::EdgeDb::in_memory().await.unwrap();
    db.migrate().await.unwrap();
    let status = rhizo_mqtt_contract::Envelope::from_json(include_bytes!(
        "../../../test/fixtures/protocol/valid/status-with-capabilities.json"
    ))
    .unwrap();
    rhizo_storage::repo::ingest::persist_status(&db, &status, 1)
        .await
        .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM plants")
            .fetch_one(db.pool())
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn lwt_offline() {
    let db = rhizo_storage::EdgeDb::in_memory().await.unwrap();
    db.migrate().await.unwrap();
    let online: rhizo_mqtt_contract::Envelope<rhizo_mqtt_contract::payload::DeviceStatus> =
        rhizo_mqtt_contract::Envelope::from_json(include_bytes!(
            "../../../test/fixtures/protocol/valid/status-with-capabilities.json"
        ))
        .unwrap();
    rhizo_storage::repo::ingest::persist_status(&db, &online, 10)
        .await
        .unwrap();
    let mut lwt = online.clone();
    lwt.message_id = rhizo_mqtt_contract::MessageId::from_uuid(uuid::Uuid::new_v4());
    lwt.sequence = Some(0);
    lwt.data.status = rhizo_mqtt_contract::payload::DeviceStatusValue::Offline;
    lwt.data.reason = Some("connection_lost".to_owned());
    rhizo_storage::repo::ingest::persist_status(&db, &lwt, 20)
        .await
        .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT status FROM devices")
            .fetch_one(db.pool())
            .await
            .unwrap(),
        "offline"
    );
    assert_eq!(
        sqlx::query_scalar::<_, Option<i64>>("SELECT last_seen_at FROM devices")
            .fetch_one(db.pool())
            .await
            .unwrap(),
        Some(10)
    );
    assert_eq!(
        rhizo_storage::repo::ingest::persist_status(&db, &lwt, 30)
            .await
            .unwrap(),
        rhizo_storage::repo::ingest::Dedup::Duplicate
    );
}

#[tokio::test]
async fn stale_without_inbound_message() {
    let db = rhizo_storage::EdgeDb::in_memory().await.unwrap();
    db.migrate().await.unwrap();
    let status = rhizo_mqtt_contract::Envelope::from_json(include_bytes!(
        "../../../test/fixtures/protocol/valid/status-with-capabilities.json"
    ))
    .unwrap();
    rhizo_storage::repo::ingest::persist_status(&db, &status, 1_000)
        .await
        .unwrap();
    let clock = Arc::new(TestClock::new(
        Utc.timestamp_millis_opt(902_000).single().unwrap(),
    ));
    let options = MqttOptions::new("unused-health-test", "127.0.0.1", 9);
    let (client, _) = rumqttc::AsyncClient::new(options, 8);
    let (stop, shutdown) = tokio::sync::watch::channel(false);
    let task = tokio::spawn(edge_controller::device::health::run(
        db.clone(),
        clock,
        client,
        Metrics::new().unwrap(),
        shutdown,
    ));
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM device_events WHERE kind='stale'")
            .fetch_one(db.pool())
            .await
            .unwrap(),
        1
    );
    stop.send(true).unwrap();
    task.await.unwrap().unwrap();
}

fn restart_broker() -> std::process::ExitStatus {
    let root = support::workspace_root();
    #[cfg(windows)]
    return std::process::Command::new("cmd")
        .current_dir(root)
        .args([
            "/C",
            "docker",
            "compose",
            "-f",
            "deploy/docker-compose.yml",
            "restart",
            "mosquitto",
        ])
        .status()
        .expect("docker compose must be available when broker tests are required");
    #[cfg(not(windows))]
    return std::process::Command::new("docker")
        .current_dir(root)
        .args([
            "compose",
            "-f",
            "deploy/docker-compose.yml",
            "restart",
            "mosquitto",
        ])
        .status()
        .expect("docker compose must be available when broker tests are required");
}

#[tokio::test]
async fn mqtt_ingress_persists_and_deduplicates_qos1() {
    let Some(b) = support::broker("mqtt_ingress_persists_and_deduplicates_qos1").await else {
        return;
    };
    let edge = EdgeHarness::start(&b).await;
    let bytes = include_bytes!("../../../test/fixtures/protocol/valid/telemetry-batch.json");
    publish(&b, bytes).await;
    let first = count(&edge.db).await;
    assert!(
        first > 0,
        "no rows; processed={:?}, quarantine={}, ingress_finished={}, pipeline_finished={}",
        sqlx::query_as::<_, (String, String)>("SELECT message_id,kind FROM processed_messages")
            .fetch_all(edge.db.pool())
            .await
            .unwrap(),
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM quarantined_messages")
            .fetch_one(edge.db.pool())
            .await
            .unwrap(),
        edge.ingress.is_finished(),
        edge.pipeline.is_finished()
    );
    publish(&b, bytes).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(count(&edge.db).await, first);
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM processed_messages WHERE kind='telemetry.batch'"
        )
        .fetch_one(edge.db.pool())
        .await
        .unwrap(),
        1
    );
}

#[tokio::test]
async fn real_m2_simulator_telemetry_reaches_sqlite() {
    let Some(b) = support::broker("real_m2_simulator_telemetry_reaches_sqlite").await else {
        return;
    };
    let edge = EdgeHarness::start(&b).await;
    let simulator = support::SimulatedDevice::start(
        &b,
        "plant-node-01",
        &[
            "--telemetry-interval",
            "10",
            "--time-scale",
            "100",
            "--no-control-api",
        ],
    )
    .await;
    assert!(
        count(&edge.db).await > 0,
        "simulator telemetry was not persisted"
    );
    assert!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM measurements WHERE received_at=1900000000000"
        )
        .fetch_one(edge.db.pool())
        .await
        .unwrap()
            > 0,
        "simulator samples did not use the injected Edge receipt clock"
    );
    simulator.stop_cleanly().await;
}

#[tokio::test]
async fn partial_invalid_fields_persist_good_siblings_through_mqtt() {
    let Some(b) =
        support::broker("partial_invalid_fields_persist_good_siblings_through_mqtt").await
    else {
        return;
    };
    let edge = EdgeHarness::start(&b).await;
    let mut message: serde_json::Value = serde_json::from_slice(include_bytes!(
        "../../../test/fixtures/protocol/valid/telemetry-batch.json"
    ))
    .unwrap();
    message["message_id"] = serde_json::json!(uuid::Uuid::new_v4());
    message["data"]["batch_id"] = serde_json::json!(uuid::Uuid::new_v4());
    message["data"]["samples"][0]["value"] = serde_json::json!(140.0);
    publish(&b, &serde_json::to_vec(&message).unwrap()).await;
    assert!(count(&edge.db).await > 1);
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM measurements WHERE value_num IS NULL AND value_bool IS NULL"
        )
        .fetch_one(edge.db.pool())
        .await
        .unwrap(),
        1
    );
}

#[tokio::test]
async fn malformed_payload_is_quarantined_and_following_mqtt_message_processes() {
    let Some(b) =
        support::broker("malformed_payload_is_quarantined_and_following_mqtt_message_processes")
            .await
    else {
        return;
    };
    let edge = EdgeHarness::start(&b).await;
    publish(&b, b"not-json").await;
    publish(
        &b,
        include_bytes!("../../../test/fixtures/protocol/valid/telemetry-batch.json"),
    )
    .await;
    assert!(count(&edge.db).await > 0);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM quarantined_messages")
            .fetch_one(edge.db.pool())
            .await
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn broker_restart_reconnects_and_resubscribes() {
    if std::env::var("RHIZO_REQUIRE_BROKER").is_err() {
        eprintln!("SKIPPING broker restart mutation unless RHIZO_REQUIRE_BROKER=1");
        return;
    }
    let Some(b) = support::broker("broker_restart_reconnects_and_resubscribes").await else {
        return;
    };
    let edge = EdgeHarness::start(&b).await;
    let first = include_bytes!("../../../test/fixtures/protocol/valid/telemetry-batch.json");
    publish(&b, first).await;
    assert!(count(&edge.db).await > 0);
    let status = restart_broker();
    assert!(status.success());
    // Mosquitto may restart faster than rumqttc's jittered reconnect window.
    // Wait through the bounded first retry before publishing the proof message.
    tokio::time::sleep(Duration::from_secs(6)).await;
    let second = include_bytes!("../../../test/fixtures/protocol/valid/telemetry-partial.json");
    publish(&b, second).await;
    for _ in 0..50 {
        let n = sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM processed_messages WHERE kind='telemetry.batch'",
        )
        .fetch_one(edge.db.pool())
        .await
        .unwrap();
        if n == 2 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("telemetry did not resume after broker restart");
}

#[tokio::test]
async fn offline_replay_is_idempotent_and_acknowledged_after_commit() {
    let Some(b) =
        support::broker("offline_replay_is_idempotent_and_acknowledged_after_commit").await
    else {
        return;
    };
    let edge = EdgeHarness::start(&b).await;
    let raw = replay_with_sequences(
        include_bytes!("../../../test/fixtures/protocol/valid/events-replay-gap.json"),
        &[0, 1, 2, 3],
    );
    let ack = publish_replay_and_wait_ack(&b, &raw).await;
    let json = ack.json();
    assert_eq!(json["data"]["through_device_seq"], 3);
    for _ in 0..50 {
        if sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM device_events WHERE origin='offline_replay'",
        )
        .fetch_one(edge.db.pool())
        .await
        .unwrap()
            == 4
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let _lost_ack = ack;
    let second = publish_replay_and_wait_ack(&b, &raw).await;
    assert!(!second.retain, "event.ack must never be retained");
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM device_events WHERE origin='offline_replay'"
        )
        .fetch_one(edge.db.pool())
        .await
        .unwrap(),
        4
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM history_gaps")
            .fetch_one(edge.db.pool())
            .await
            .unwrap(),
        1
    );
}

/// A stale/restored Edge database can retain the durable replayed events while
/// losing their reconstructable progress row. A reconnect uses stable event
/// ids under a fresh transport message id; Edge must rebuild the proven prefix
/// and acknowledge it rather than leaving the simulator in a replay loop.
#[tokio::test]
async fn missing_replay_progress_is_reconstructed_and_acknowledged() {
    let Some(b) =
        support::broker("missing_replay_progress_is_reconstructed_and_acknowledged").await
    else {
        return;
    };
    let edge = EdgeHarness::start(&b).await;
    let fixture = include_bytes!("../../../test/fixtures/protocol/valid/events-replay-gap.json");
    let first = replay_with_sequences(fixture, &[0, 1, 2, 3]);
    assert_eq!(
        publish_replay_and_wait_ack(&b, &first).await.json()["data"]["through_device_seq"],
        3
    );

    sqlx::query("DELETE FROM replay_progress")
        .execute(edge.db.pool())
        .await
        .unwrap();

    let reconnect = replay_with_sequences(fixture, &[0, 1, 2, 3]);
    let ack = publish_replay_and_wait_ack(&b, &reconnect).await;
    assert_eq!(ack.json()["data"]["through_device_seq"], 3);
    assert!(
        !ack.retain,
        "the reconstructed acknowledgement stays live-only"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM device_events WHERE origin='offline_replay'"
        )
        .fetch_one(edge.db.pool())
        .await
        .unwrap(),
        4,
        "reconstruction must not duplicate logical events"
    );
}

/// The shipped fixture is a genuine suffix: it opens with a sealed gap marker
/// at `device_seq` 118 covering 101-117. The edge holds nothing at or below
/// 117, so protocol section 5.13's prefix rule leaves it nothing truthful to
/// acknowledge — and 0 is a real sequence, not a way to say "nothing".
#[tokio::test]
async fn a_suffix_only_replay_is_committed_without_an_acknowledgement() {
    let Some(b) =
        support::broker("a_suffix_only_replay_is_committed_without_an_acknowledgement").await
    else {
        return;
    };
    let edge = EdgeHarness::start(&b).await;
    let raw = include_bytes!("../../../test/fixtures/protocol/valid/events-replay-gap.json");
    let ack = publish_replay(&b, raw, Duration::from_secs(3)).await;
    assert!(
        ack.is_none(),
        "a suffix-only replay must not be acknowledged, got {ack:?}"
    );
    for _ in 0..50 {
        if sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM device_events WHERE origin='offline_replay'",
        )
        .fetch_one(edge.db.pool())
        .await
        .unwrap()
            == 4
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM device_events WHERE origin='offline_replay'"
        )
        .fetch_one(edge.db.pool())
        .await
        .unwrap(),
        4,
        "the events are durably committed; only the acknowledgement is withheld"
    );
    assert!(
        sqlx::query_scalar::<_, Option<i64>>("SELECT through_device_seq FROM replay_progress")
            .fetch_one(edge.db.pool())
            .await
            .unwrap()
            .is_none()
    );
}

/// Sequence 0 is acknowledged as 0, on the wire. This is the case the old
/// `u64` representation could not distinguish from "nothing committed", and
/// the reason the earlier test had to renumber its fixture away from it.
#[tokio::test]
async fn a_replay_of_sequence_zero_is_acknowledged_with_zero_on_the_wire() {
    let Some(b) =
        support::broker("a_replay_of_sequence_zero_is_acknowledged_with_zero_on_the_wire").await
    else {
        return;
    };
    let _edge = EdgeHarness::start(&b).await;
    let raw = replay_with_sequences(
        include_bytes!("../../../test/fixtures/protocol/valid/events-replay-gap.json"),
        &[0],
    );
    let ack = publish_replay_and_wait_ack(&b, &raw).await;
    assert_eq!(ack.json()["data"]["through_device_seq"], 0);
    assert!(!ack.retain, "event.ack must never be retained");
}

// ---------------------------------------------------------------------------
// M5 — the plant model, end to end.
//
// The property that defines the milestone is the *absence* of an action:
// `no_commands_in_m5` runs a full drying cycle against a real broker and
// asserts that nothing at all appears on any `commands/*` topic. It is the
// reason M5 exists as a separate milestone — the recommendation logic can be
// validated against a living plant for a week before anything can pump.

/// A plant configured the way the plant tick expects, with a drying series.
async fn drying_plant(db: &rhizo_storage::EdgeDb, now: chrono::DateTime<Utc>) {
    use rhizo_storage::repo::{binding, plant};
    // Measurements reference a device row, so the registry has to know about
    // the node before any reading can be stored against it.
    sqlx::query(
        "INSERT OR IGNORE INTO devices(device_id,created_at,status,sensors_json) \
         VALUES('plant-node-01',?,'online',?)",
    )
    .bind(now.timestamp_millis())
    .bind(
        serde_json::json!([
            {"sensor_id":"soil-0","point":"default","kinds":["soil_moisture","soil_temperature"],
             "present":true,"healthy":true,"errors":0}
        ])
        .to_string(),
    )
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
    binding::upsert_sensor_binding(
        db,
        &binding::SensorBindingRow {
            binding_id: "b-control".to_owned(),
            plant_id: "monstera-01".to_owned(),
            device_id: "plant-node-01".to_owned(),
            sensor_id: "soil-0".to_owned(),
            point: "default".to_owned(),
            kind: "soil_moisture".to_owned(),
            role: "control".to_owned(),
            created_at: now.timestamp_millis(),
        },
    )
    .await
    .unwrap();
    binding::upsert_actuator_binding(
        db,
        &binding::ActuatorBindingRow {
            plant_id: "monstera-01".to_owned(),
            device_id: "plant-node-01".to_owned(),
            actuator_id: "pump-0".to_owned(),
            kind: "irrigation_pump".to_owned(),
            created_at: now.timestamp_millis(),
        },
    )
    .await
    .unwrap();
    binding::upsert_measurement_policy(
        db,
        &binding::MeasurementPolicyRow {
            plant_id: "monstera-01".to_owned(),
            kind: "soil_moisture".to_owned(),
            target_min: Some(28.0),
            target_max: Some(45.0),
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

async fn record_sample(db: &rhizo_storage::EdgeDb, at: i64, kind: &str, unit: &str, value: f64) {
    sqlx::query(
        "INSERT INTO measurements(device_id,sensor_id,point,kind,value_num,unit,quality,received_at,batch_id,origin) \
         VALUES('plant-node-01','soil-0','default',?,?,?,'ok',?,?,'live')",
    )
    .bind(kind)
    .bind(value)
    .bind(unit)
    .bind(at)
    .bind(uuid::Uuid::new_v4().to_string())
    .execute(db.pool())
    .await
    .unwrap();
}

/// **The M5 exit criterion.** A real broker, a real edge, a full drying cycle
/// that reaches `water_recommended` — and not one byte on any command topic.
#[tokio::test]
async fn no_commands_in_m5() {
    let Some(b) = support::broker("no_commands_in_m5").await else {
        return;
    };
    let edge = EdgeHarness::start(&b).await;

    // A subscriber on every command topic in the fleet, for the whole run.
    let mut watcher = b
        .edge_subscriber(
            &format!("m5-command-watch-{}", uuid::Uuid::new_v4()),
            "rhizo/v1/devices/+/commands/#",
        )
        .await;

    let start = Utc
        .timestamp_millis_opt(1_900_000_000_000)
        .single()
        .unwrap();
    let clock = TestClock::new(start);
    drying_plant(&edge.db, start).await;
    // Six hours of five-minute readings drying from 40 % through 28 %.
    for i in 0i64..72 {
        record_sample(
            &edge.db,
            (start - chrono::Duration::minutes((71 - i) * 5)).timestamp_millis(),
            "soil_moisture",
            "vwc_percent",
            40.0 - i as f64 * 0.25,
        )
        .await;
    }

    let metrics = Metrics::new().unwrap();
    for _ in 0..5 {
        edge_controller::control::tick::tick(&edge.db, &clock, &metrics)
            .await
            .unwrap();
        clock.advance(chrono::Duration::seconds(30));
    }

    let state = rhizo_storage::repo::plant::plant_state(&edge.db, "monstera-01")
        .await
        .unwrap();
    assert_eq!(
        state.as_deref(),
        Some("water_recommended"),
        "the drying cycle must actually reach the state the criterion is about"
    );
    let recommendation = rhizo_storage::repo::plant::latest_recommendation(&edge.db, "monstera-01")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(recommendation.decision, "water");
    let reasons: Vec<serde_json::Value> =
        serde_json::from_str(&recommendation.reasons_json).unwrap();
    assert!(
        !reasons.is_empty(),
        "the recommendation must carry a non-empty structured reason list"
    );
    assert!(reasons.iter().any(|r| r["code"] == "moisture_below_target"));
    assert!(reasons.iter().any(|r| r["code"] == "dry_for"));

    let seen = watcher.drain_for(Duration::from_millis(750)).await;
    assert!(
        seen.is_empty(),
        "M5 issues no commands, and something published {:?}",
        seen.iter().map(|m| m.topic.clone()).collect::<Vec<_>>()
    );

    // And nothing was recorded as having watered anything.
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM commands")
            .fetch_one(edge.db.pool())
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM watering_events")
            .fetch_one(edge.db.pool())
            .await
            .unwrap(),
        0
    );
}

/// A manual watering is detected, resets the cooldown, and a rise following a
/// completed command creates no second event (F-050-14, F-050-16).
#[tokio::test]
async fn manual_watering_detection() {
    let db = rhizo_storage::EdgeDb::in_memory().await.unwrap();
    db.migrate().await.unwrap();
    let start = Utc
        .timestamp_millis_opt(1_900_000_000_000)
        .single()
        .unwrap();
    drying_plant(&db, start).await;
    record_sample(
        &db,
        (start - chrono::Duration::minutes(5)).timestamp_millis(),
        "soil_moisture",
        "vwc_percent",
        24.0,
    )
    .await;
    record_sample(
        &db,
        start.timestamp_millis(),
        "soil_moisture",
        "vwc_percent",
        44.0,
    )
    .await;

    let loaded = edge_controller::plant::load(&db, "monstera-01")
        .await
        .unwrap()
        .unwrap();
    let outcome = edge_controller::plant::detect::run(&db, &loaded, start)
        .await
        .unwrap();
    assert!(outcome.recorded);
    assert_eq!(
        rhizo_storage::repo::plant::last_watering_at(&db, "monstera-01")
            .await
            .unwrap(),
        Some(start.timestamp_millis()),
        "a detected watering resets time-since-last-watering"
    );
    assert_eq!(
        rhizo_storage::repo::plant::delivered_since(&db, "monstera-01", 0)
            .await
            .unwrap(),
        0.0,
        "and is excluded from the automatic daily budget"
    );

    // Now the same rise, but with a command that completed inside the
    // absorption window: no second event.
    let db = rhizo_storage::EdgeDb::in_memory().await.unwrap();
    db.migrate().await.unwrap();
    drying_plant(&db, start).await;
    sqlx::query(
        "INSERT INTO commands(command_id,device_id,plant_id,kind,requested_ml,mode,issued_at,expires_at,status) \
         VALUES('cmd-1','plant-node-01','monstera-01','water',40.0,'manual',?,?,'completed')",
    )
    .bind(start.timestamp_millis() - 300_000)
    .bind(start.timestamp_millis())
    .execute(db.pool())
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO watering_events(watering_event_id,plant_id,device_id,command_id,mode,origin,started_at,completed_at,requested_ml,delivered_ml,status) \
         VALUES('we-1','monstera-01','plant-node-01','cmd-1','manual','edge_command',?,?,40.0,40.0,'completed')",
    )
    .bind(start.timestamp_millis() - 300_000)
    .bind(start.timestamp_millis() - 120_000)
    .execute(db.pool())
    .await
    .unwrap();
    record_sample(
        &db,
        (start - chrono::Duration::minutes(5)).timestamp_millis(),
        "soil_moisture",
        "vwc_percent",
        24.0,
    )
    .await;
    record_sample(
        &db,
        start.timestamp_millis(),
        "soil_moisture",
        "vwc_percent",
        44.0,
    )
    .await;
    let loaded = edge_controller::plant::load(&db, "monstera-01")
        .await
        .unwrap()
        .unwrap();
    let outcome = edge_controller::plant::detect::run(&db, &loaded, start)
        .await
        .unwrap();
    assert!(!outcome.recorded, "the command explains the rise");
    assert!(outcome.attributed_to_command);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM watering_events")
            .fetch_one(db.pool())
            .await
            .unwrap(),
        1,
        "the command's own event, and no second one"
    );
}

/// SCEN-024: a sensor stuck on one value is marked, once.
#[tokio::test]
async fn stuck_sensor() {
    let db = rhizo_storage::EdgeDb::in_memory().await.unwrap();
    db.migrate().await.unwrap();
    let start = Utc
        .timestamp_millis_opt(1_900_000_000_000)
        .single()
        .unwrap();
    drying_plant(&db, start).await;
    let loaded = edge_controller::plant::load(&db, "monstera-01")
        .await
        .unwrap()
        .unwrap();
    for i in 0i64..25 {
        let at = start + chrono::Duration::minutes(i * 5);
        record_sample(
            &db,
            at.timestamp_millis(),
            "soil_moisture",
            "vwc_percent",
            31.25,
        )
        .await;
        edge_controller::plant::detect::stuck(&db, &loaded, at)
            .await
            .unwrap();
    }
    let events = rhizo_storage::repo::plant::plant_events(&db, "monstera-01", 100)
        .await
        .unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|(kind, ..)| kind == "sensor_stuck")
            .count(),
        1,
        "one event per stuck run, not one per sample"
    );
    let stuck = events
        .iter()
        .find(|(kind, ..)| kind == "sensor_stuck")
        .unwrap();
    assert_eq!(stuck.1, "warning");
}

/// A critical threshold alerts on a plant that cannot be watered, and waters
/// nothing on a plant that can.
#[tokio::test]
async fn threshold_alerts() {
    let db = rhizo_storage::EdgeDb::in_memory().await.unwrap();
    db.migrate().await.unwrap();
    let start = Utc
        .timestamp_millis_opt(1_900_000_000_000)
        .single()
        .unwrap();
    drying_plant(&db, start).await;
    // A monitoring-only plant sharing the same probe, with its own bands.
    rhizo_storage::repo::plant::create(
        &db,
        &rhizo_storage::repo::plant::NewPlant {
            plant_id: "fern-01".to_owned(),
            name: "Fern".to_owned(),
            ..Default::default()
        },
        start.timestamp_millis(),
    )
    .await
    .unwrap();
    rhizo_storage::repo::binding::upsert_sensor_binding(
        &db,
        &rhizo_storage::repo::binding::SensorBindingRow {
            binding_id: "b-fern".to_owned(),
            plant_id: "fern-01".to_owned(),
            device_id: "plant-node-01".to_owned(),
            sensor_id: "soil-0".to_owned(),
            point: "default".to_owned(),
            kind: "soil_temperature".to_owned(),
            role: "advisory".to_owned(),
            created_at: start.timestamp_millis(),
        },
    )
    .await
    .unwrap();
    rhizo_storage::repo::binding::upsert_measurement_policy(
        &db,
        &rhizo_storage::repo::binding::MeasurementPolicyRow {
            plant_id: "fern-01".to_owned(),
            kind: "soil_temperature".to_owned(),
            target_min: Some(16.0),
            target_max: Some(24.0),
            warning_low: Some(12.0),
            warning_high: Some(28.0),
            critical_low: Some(8.0),
            critical_high: Some(32.0),
            stale_after_ms: 900_000,
            hysteresis: Some(1.0),
            confirm_duration_ms: None,
        },
        start.timestamp_millis(),
    )
    .await
    .unwrap();
    record_sample(
        &db,
        start.timestamp_millis(),
        "soil_temperature",
        "celsius",
        34.0,
    )
    .await;

    let fern = edge_controller::plant::load(&db, "fern-01")
        .await
        .unwrap()
        .unwrap();
    assert!(
        fern.actuator.is_none(),
        "the point of the test is a plant that cannot be watered"
    );
    let crossings = edge_controller::control::threshold::run(&db, &fern, start)
        .await
        .unwrap();
    assert_eq!(crossings.len(), 1);
    assert_eq!(
        crossings[0].1.to,
        rhizo_domain::threshold::Severity::Critical
    );
    let events = rhizo_storage::repo::plant::plant_events(&db, "fern-01", 50)
        .await
        .unwrap();
    assert!(events.iter().any(|(kind, ..)| kind == "threshold.critical"));

    // The crossing waters nothing, here or anywhere.
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM commands")
            .fetch_one(db.pool())
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM watering_events")
            .fetch_one(db.pool())
            .await
            .unwrap(),
        0
    );
}

/// SCEN-110 and SCEN-111, end to end: a *simulated* battery device drives the
/// M4 liveness model that until now had nothing to reason about.
///
/// The edge derives `sleeping` from its own receipt time while the window is
/// open, and `isolated` once the device stops waking — with the transition made
/// by the liveness timer, with no inbound message (SAFETY-021).
#[tokio::test]
async fn a_simulated_sleeping_device_is_sleeping_then_isolated() {
    let Some(b) = support::broker("simulated_battery_liveness").await else {
        return;
    };
    // Retention outlives a test process; start from a clean slate.
    let publisher = b
        .edge_subscriber(
            &format!("battery-clear-{}", uuid::Uuid::new_v4()),
            "rhizo/v1/devices/+/status",
        )
        .await;
    publisher
        .client()
        .publish(
            "rhizo/v1/devices/plant-node-01/status",
            QoS::AtLeastOnce,
            true,
            Vec::new(),
        )
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;

    let edge = EdgeHarness::start(&b).await;
    let device = support::SimulatedDevice::start(
        &b,
        "plant-node-01",
        &[
            "--power-mode",
            "battery",
            "--wake-interval-seconds",
            "60",
            "--awake-budget-seconds",
            "5",
            "--sensor-warmup-ms",
            "500",
            "--telemetry-interval",
            "10",
            "--time-scale",
            "60",
            "--no-control-api",
            "--no-noise",
            // Two consecutive missed wakes, so the device stops waking after
            // its first announced sleep.
            "--fault",
            "miss-wake:2",
        ],
    )
    .await;

    // Wait for the announced sleep to reach the registry.
    let mut mode = String::new();
    for _ in 0..80 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        mode = sqlx::query_scalar::<_, String>(
            "SELECT connectivity_mode FROM devices WHERE device_id='plant-node-01'",
        )
        .fetch_optional(edge.db.pool())
        .await
        .unwrap()
        .unwrap_or_default();
        if mode == "sleeping" {
            break;
        }
    }
    assert_eq!(
        mode, "sleeping",
        "an announced, bounded sleep is not an offline device"
    );
    let (expected_wake_at, overdue_at) = sqlx::query_as::<_, (Option<i64>, Option<i64>)>(
        "SELECT expected_wake_at, overdue_at FROM devices WHERE device_id='plant-node-01'",
    )
    .fetch_one(edge.db.pool())
    .await
    .unwrap();
    let expected_wake_at = expected_wake_at.expect("an open window has a wake instant");
    let overdue_at = overdue_at.expect("and a deadline");
    assert!(
        overdue_at > expected_wake_at,
        "the deadline is past the expected wake, not at it"
    );

    // The read-side derivation is what SAFETY-021 turns on: past the deadline
    // the device is `isolated`, whether or not the timer has run.
    let derived = |now: i64| {
        edge_controller::device::connectivity::from_projection(
            "sleeping",
            Some(expected_wake_at),
            Some(overdue_at),
            now,
        )
    };
    assert_eq!(derived(expected_wake_at).api_name(), "sleeping");
    assert_eq!(
        derived(overdue_at).api_name(),
        "isolated",
        "a device past its window is isolated, never sleeping"
    );

    // And the timer makes the durable transition with no inbound message: the
    // device is asleep and publishing nothing at all.
    let clock: Arc<dyn rhizo_domain::Clock> = Arc::new(TestClock::new(
        Utc.timestamp_millis_opt(overdue_at + 1).single().unwrap(),
    ));
    let metrics = Metrics::new().unwrap();
    let (client, _events) = ingress::connect(MqttOptions::new("liveness-timer", "127.0.0.1", 9), 4);
    edge_controller::device::health::tick(&edge.db, clock.as_ref(), &client, &metrics)
        .await
        .unwrap();
    let (mode, missed) = sqlx::query_as::<_, (String, i64)>(
        "SELECT connectivity_mode, missed_wake_count FROM devices WHERE device_id='plant-node-01'",
    )
    .fetch_one(edge.db.pool())
    .await
    .unwrap();
    assert_eq!(mode, "isolated");
    assert_eq!(
        missed, 1,
        "one sleep window opened and one closed unmet; M4's timer counts each \
         window at most once, and a device that stops waking opens no further \
         windows to miss"
    );
    drop(device);
}
