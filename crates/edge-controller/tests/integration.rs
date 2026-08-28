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

struct EdgeHarness {
    db: rhizo_storage::EdgeDb,
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
        let clock: Arc<dyn rhizo_domain::Clock> = Arc::new(TestClock::new(
            Utc.timestamp_millis_opt(1_900_000_000_000)
                .single()
                .unwrap(),
        ));
        let pipeline = tokio::spawn(pipeline::run(
            rx,
            db.clone(),
            clock,
            client,
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
