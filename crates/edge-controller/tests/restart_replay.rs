//! Restart and replay behaviour of the battery liveness window.
//!
//! Everything here uses a **file-backed** database that is closed and reopened,
//! because the whole question is what survives a process boundary. An in-memory
//! database cannot answer it: the sleep deadline, the dedup ledger, and the
//! missed-wake counter all live in SQLite precisely so that an edge restart in
//! the middle of a fifteen-minute sleep does not lose the window, and none of
//! that is exercised by a handle that dies with the test.
//!
//! The device in these tests never sends anything after it goes to sleep. That
//! is the point: a sleeping device is silent, so every transition below has to
//! come from persisted state plus the edge's own clock (SAFETY-021).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use chrono::{TimeZone, Utc};
use edge_controller::{device::connectivity, device::health, metrics::Metrics};
use rhizo_mqtt_contract::{
    Envelope, MessageId,
    payload::{DeviceStatus, DeviceStatusValue, PowerMode, PowerStatus},
};
use rhizo_storage::{EdgeDb, repo::ingest};
use sqlx::Row as _;

const WAKE_INTERVAL_SECONDS: i64 = 900;
/// Receipt time of the sleep announcement, on the edge clock.
const SLEEP_AT: i64 = 2_000_000;
/// `received_at + wake_interval`.
const EXPECTED_WAKE_AT: i64 = SLEEP_AT + WAKE_INTERVAL_SECONDS * 1_000;
/// `expected_wake_at + max(wake_interval, 300 s)`.
const OVERDUE_AT: i64 = EXPECTED_WAKE_AT + WAKE_INTERVAL_SECONDS * 1_000;

fn online() -> Envelope<DeviceStatus> {
    Envelope::from_json(include_bytes!(
        "../../../test/fixtures/protocol/valid/status-with-capabilities.json"
    ))
    .unwrap()
}

/// The retained sleep announcement. Its `message_id` is stable, because the
/// broker redelivers *the same retained message* to a fresh subscriber.
fn sleep_announcement(base: &Envelope<DeviceStatus>) -> Envelope<DeviceStatus> {
    let mut e = base.clone();
    e.message_id = MessageId::from_uuid(uuid::Uuid::from_u128(0x5133_9100));
    e.sequence = Some(base.sequence.unwrap() + 1);
    e.data.status = DeviceStatusValue::Offline;
    e.data.reason = Some("sleeping".into());
    e.data.power = Some(Box::new(PowerStatus {
        mode: PowerMode::Battery,
        wake_interval_seconds: Some(u32::try_from(WAKE_INTERVAL_SECONDS).unwrap()),
        expected_wake_ms: Some(u64::MAX),
        wake_reason: Some(rhizo_mqtt_contract::payload::WakeReason::Timer),
        battery_mv: Some(3_280),
        awake_ms: Some(4_120),
    }));
    e
}

/// Closing the pool and opening the same file again is what an edge restart
/// looks like to SQLite.
async fn restart(db: EdgeDb, path: &std::path::Path) -> EdgeDb {
    db.pool().close().await;
    drop(db);
    let db = EdgeDb::connect(path).await.unwrap();
    db.migrate().await.unwrap();
    db
}

async fn window(db: &EdgeDb) -> (String, Option<i64>, Option<i64>, i64) {
    let row = sqlx::query(
        "SELECT connectivity_mode,expected_wake_at,overdue_at,missed_wake_count FROM devices",
    )
    .fetch_one(db.pool())
    .await
    .unwrap();
    (
        row.get("connectivity_mode"),
        row.get("expected_wake_at"),
        row.get("overdue_at"),
        row.get("missed_wake_count"),
    )
}

async fn wake_missed_events(db: &EdgeDb) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM device_events WHERE kind='device_wake_missed'")
        .fetch_one(db.pool())
        .await
        .unwrap()
}

/// A client that is never connected. Every publish fails, which is deliberate:
/// the liveness transition must not depend on the broker being reachable.
fn offline_client() -> rumqttc::AsyncClient {
    rumqttc::AsyncClient::new(
        rumqttc::MqttOptions::new("restart-replay-test", "127.0.0.1", 9),
        4,
    )
    .0
}

/// `Metrics::new` hands out one process-wide registry, so the fleet-wide
/// `device_wake_missed_total` counter is shared by every test in this binary.
/// The tests that assert on it therefore take turns; the database assertions
/// beside them are per-test and need no such care.
static COUNTER: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn sweep(db: &EdgeDb, now_ms: i64, metrics: &Metrics) {
    let clock = rhizo_testkit::TestClock::new(Utc.timestamp_millis_opt(now_ms).single().unwrap());
    health::tick(db, &clock, &offline_client(), metrics)
        .await
        .unwrap();
}

/// The whole sleep lifecycle across two process restarts.
///
/// Split into one test rather than six because the interesting property is that
/// the *sequence* survives: each restart has to pick up exactly the state the
/// previous process left, and a per-step test with a fresh database would prove
/// none of that.
#[tokio::test]
async fn a_sleep_window_survives_restart_and_is_missed_exactly_once() {
    let _counter = COUNTER.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("edge.sqlite3");
    let metrics = Metrics::new().unwrap();

    let db = EdgeDb::connect(&path).await.unwrap();
    db.migrate().await.unwrap();
    let base = online();
    ingest::persist_status(&db, &base, 1_000_000).await.unwrap();
    let announcement = sleep_announcement(&base);
    ingest::persist_status(&db, &announcement, SLEEP_AT)
        .await
        .unwrap();
    assert_eq!(
        window(&db).await,
        (
            "sleeping".into(),
            Some(EXPECTED_WAKE_AT),
            Some(OVERDUE_AT),
            0
        )
    );

    // 1. Restart while the device is asleep: the same deadline comes back.
    let db = restart(db, &path).await;
    assert_eq!(
        window(&db).await,
        (
            "sleeping".into(),
            Some(EXPECTED_WAKE_AT),
            Some(OVERDUE_AT),
            0
        ),
        "an edge restart must restore the sleep deadline, not recompute it"
    );
    // And it is still reported as sleeping while the window is genuinely open.
    let (mode, expected, overdue, _) = window(&db).await;
    assert_eq!(
        connectivity::from_projection(&mode, expected, overdue, EXPECTED_WAKE_AT).api_name(),
        "sleeping"
    );

    // 2. The broker redelivers the retained announcement to the fresh
    //    subscriber. It is the same message, so it is a duplicate, and a
    //    duplicate must not buy the device another wake interval.
    assert_eq!(
        ingest::persist_status(&db, &announcement, SLEEP_AT + 30_000)
            .await
            .unwrap(),
        ingest::Dedup::Duplicate
    );
    assert_eq!(
        window(&db).await,
        (
            "sleeping".into(),
            Some(EXPECTED_WAKE_AT),
            Some(OVERDUE_AT),
            0
        ),
        "a retained redelivery must not extend expected_wake_at"
    );

    // 3. Before the deadline the timer changes nothing, however often it runs.
    let before = metrics.device_wake_missed.get();
    for tick in 0..3 {
        sweep(&db, OVERDUE_AT - 1_000 - tick, &metrics).await;
    }
    assert_eq!(
        window(&db).await,
        (
            "sleeping".into(),
            Some(EXPECTED_WAKE_AT),
            Some(OVERDUE_AT),
            0
        )
    );
    assert_eq!(metrics.device_wake_missed.get(), before);
    assert_eq!(wake_missed_events(&db).await, 0);

    // 4. Past the deadline: one transition, one event, one increment -- and the
    //    timer keeps running, because a five-second timer will hit this row
    //    hundreds of times before anyone notices.
    for _ in 0..5 {
        sweep(&db, OVERDUE_AT + 1_000, &metrics).await;
    }
    assert_eq!(
        window(&db).await,
        (
            "isolated".into(),
            Some(EXPECTED_WAKE_AT),
            Some(OVERDUE_AT),
            1
        ),
        "a missed wake is counted once, not once per tick"
    );
    assert_eq!(wake_missed_events(&db).await, 1);
    assert_eq!(metrics.device_wake_missed.get(), before + 1);

    // 5. Restarting after a missed wake must not count it again. The row still
    //    carries the elapsed `overdue_at`, so the guard is the stored
    //    connectivity, not the timestamp.
    let db = restart(db, &path).await;
    for _ in 0..3 {
        sweep(&db, OVERDUE_AT + 60_000, &metrics).await;
    }
    assert_eq!(
        window(&db).await,
        (
            "isolated".into(),
            Some(EXPECTED_WAKE_AT),
            Some(OVERDUE_AT),
            1
        ),
        "a restart must not re-count a wake that was already missed"
    );
    assert_eq!(wake_missed_events(&db).await, 1);
    assert_eq!(metrics.device_wake_missed.get(), before + 1);

    // 6. The device finally wakes, late. It recovers to connected, the window is
    //    cleared, and the missed-wake count resets.
    let mut wake = base.clone();
    wake.message_id = MessageId::from_uuid(uuid::Uuid::new_v4());
    wake.data.boot_generation += 1;
    wake.sequence = Some(1);
    ingest::persist_status(&db, &wake, OVERDUE_AT + 120_000)
        .await
        .unwrap();
    assert_eq!(window(&db).await, ("connected".into(), None, None, 0));

    // 7. The Last Will the broker was holding from the pre-sleep session finally
    //    arrives. It belongs to an older boot generation and must not regress
    //    the device that has just come back.
    let mut stale_will = base.clone();
    stale_will.message_id = MessageId::from_uuid(uuid::Uuid::new_v4());
    stale_will.sequence = Some(0);
    stale_will.data.status = DeviceStatusValue::Offline;
    stale_will.data.reason = Some("connection_lost".into());
    assert_eq!(
        ingest::persist_status(&db, &stale_will, OVERDUE_AT + 180_000)
            .await
            .unwrap(),
        ingest::Dedup::Duplicate
    );
    assert_eq!(
        window(&db).await,
        ("connected".into(), None, None, 0),
        "an older-generation will must never regress a newer wake"
    );

    // 8. And a delayed copy of the original sleep announcement cannot reopen a
    //    window on a device that is awake again.
    assert_eq!(
        ingest::persist_status(&db, &announcement, OVERDUE_AT + 240_000)
            .await
            .unwrap(),
        ingest::Dedup::Duplicate
    );
    assert_eq!(window(&db).await, ("connected".into(), None, None, 0));
}

/// `device_wake_missed_total` counts *missed wakes*, not overdue rows.
///
/// The counter is the number the operator will read as "how often has this node
/// failed to come back", so it has to survive the device recovering and failing
/// again. A device that sleeps, misses, wakes, sleeps and misses again has
/// missed two -- while a device that simply stays overdue has still missed one,
/// however long the timer keeps sweeping it.
#[tokio::test]
async fn each_sleep_cycle_can_miss_at_most_once() {
    let _counter = COUNTER.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("edge.sqlite3");
    let db = EdgeDb::connect(&path).await.unwrap();
    db.migrate().await.unwrap();
    let metrics = Metrics::new().unwrap();
    let before = metrics.device_wake_missed.get();

    let base = online();
    ingest::persist_status(&db, &base, 1_000_000).await.unwrap();
    let mut boot = base.data.boot_generation;
    let mut sequence = base.sequence.unwrap();
    let mut at = SLEEP_AT;

    for cycle in 1..=3_i64 {
        // Sleep.
        let mut announcement = sleep_announcement(&base);
        announcement.message_id = MessageId::from_uuid(uuid::Uuid::new_v4());
        announcement.data.boot_generation = boot;
        sequence += 1;
        announcement.sequence = Some(sequence);
        ingest::persist_status(&db, &announcement, at)
            .await
            .unwrap();
        let overdue = at + 2 * WAKE_INTERVAL_SECONDS * 1_000;
        assert_eq!(window(&db).await.0, "sleeping", "cycle {cycle}");

        // Miss it, and keep sweeping well past the deadline.
        for sweep_at in [overdue, overdue + 5_000, overdue + 600_000] {
            sweep(&db, sweep_at, &metrics).await;
        }
        assert_eq!(
            (
                wake_missed_events(&db).await,
                metrics.device_wake_missed.get()
            ),
            (cycle, before + u64::try_from(cycle).unwrap()),
            "cycle {cycle}: one missed wake, however many sweeps"
        );
        assert_eq!(window(&db).await.3, 1, "cycle {cycle}: consecutive misses");

        // Come back, which resets the per-device count but not the counter.
        at = overdue + 900_000;
        boot += 1;
        sequence = 1;
        let mut wake = base.clone();
        wake.message_id = MessageId::from_uuid(uuid::Uuid::new_v4());
        wake.data.boot_generation = boot;
        wake.sequence = Some(sequence);
        ingest::persist_status(&db, &wake, at).await.unwrap();
        assert_eq!(window(&db).await, ("connected".into(), None, None, 0));
        at += 60_000;
    }
}

/// The negative control for the derivation: with the liveness timer never run
/// at all, an overdue sleeper is still reported as `isolated` after a restart.
///
/// This is the failure the invariant is actually about. If the reported state
/// were simply read back from the row, a supervisor that had not yet started
/// the timer -- or a timer that had died -- would show a flat battery as
/// peacefully asleep for as long as the process stayed up.
#[tokio::test]
async fn safety_021_an_overdue_sleeper_is_isolated_after_a_restart_with_no_timer() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("edge.sqlite3");
    let db = EdgeDb::connect(&path).await.unwrap();
    db.migrate().await.unwrap();
    let base = online();
    ingest::persist_status(&db, &base, 1_000_000).await.unwrap();
    ingest::persist_status(&db, &sleep_announcement(&base), SLEEP_AT)
        .await
        .unwrap();

    let db = restart(db, &path).await;
    let (mode, expected, overdue, count) = window(&db).await;
    assert_eq!(mode, "sleeping", "the stored projection is untouched");
    assert_eq!(count, 0, "no timer has run in this process");

    let state = connectivity::from_projection(&mode, expected, overdue, OVERDUE_AT);
    assert_eq!(state.api_name(), "isolated");
    assert_eq!(state.expected_wake_at(), None);
    // One millisecond earlier the same row is legitimately asleep, which is what
    // makes the assertion above about the deadline and not about the row.
    let open = connectivity::from_projection(&mode, expected, overdue, OVERDUE_AT - 1);
    assert_eq!(open.api_name(), "sleeping");
    assert_eq!(open.expected_wake_at(), Some(EXPECTED_WAKE_AT));
}
