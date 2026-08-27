//! M3 persistence integration tests.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use rhizo_mqtt_contract::{
    Envelope,
    payload::{DeviceEventBatch, TelemetryBatch},
};
use rhizo_storage::{
    EdgeDb,
    repo::{
        ingest::{self, Dedup},
        quarantine, retention,
    },
};

async fn db() -> EdgeDb {
    let db = EdgeDb::in_memory().await.unwrap();
    db.migrate().await.unwrap();
    db
}

#[tokio::test]
async fn dedup_marker_and_effects_commit_or_rollback_together() {
    let db = db().await;
    let mut tx = db.begin().await.unwrap();
    assert_eq!(
        ingest::mark_processed(&mut tx, "m1", "node-01", "test", 1)
            .await
            .unwrap(),
        Dedup::New
    );
    sqlx::query("INSERT INTO devices(device_id,created_at) VALUES('node-01',1)")
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.rollback().await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM processed_messages")
            .fetch_one(db.pool())
            .await
            .unwrap(),
        0
    );
    let mut tx = db.begin().await.unwrap();
    assert_eq!(
        ingest::mark_processed(&mut tx, "m1", "node-01", "test", 1)
            .await
            .unwrap(),
        Dedup::New
    );
    tx.commit().await.unwrap();
    let mut tx = db.begin().await.unwrap();
    assert_eq!(
        ingest::mark_processed(&mut tx, "m1", "node-01", "test", 1)
            .await
            .unwrap(),
        Dedup::Duplicate
    );
    tx.rollback().await.unwrap();
}

#[tokio::test]
async fn mixed_batch_keeps_rows_and_nulls_only_invalid_sample() {
    let db = db().await;
    let raw = include_str!("../../../test/fixtures/protocol/valid/telemetry-partial.json")
        .replace("30.9", "150.0");
    let e = Envelope::<TelemetryBatch>::from_json(raw.as_bytes()).unwrap();
    let (d, n) = ingest::persist_telemetry(&db, &e, 9_000).await.unwrap();
    assert_eq!((d, n), (Dedup::New, 1));
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM measurements")
            .fetch_one(db.pool())
            .await
            .unwrap(),
        2
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM measurements WHERE value_num IS NULL AND kind='soil_moisture'"
        )
        .fetch_one(db.pool())
        .await
        .unwrap(),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM device_events WHERE kind='sensor_invalid'"
        )
        .fetch_one(db.pool())
        .await
        .unwrap(),
        1
    );
    assert_eq!(
        ingest::persist_telemetry(&db, &e, 10_000).await.unwrap().0,
        Dedup::Duplicate
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM measurements")
            .fetch_one(db.pool())
            .await
            .unwrap(),
        2
    );
}

#[tokio::test]
async fn replay_and_gap_are_idempotent_by_event_id() {
    let db = db().await;
    let e = Envelope::<DeviceEventBatch>::from_json(include_bytes!(
        "../../../test/fixtures/protocol/valid/events-replay-gap.json"
    ))
    .unwrap();
    ingest::persist_replay(&db, &e, 10).await.unwrap();
    let mut second = e.clone();
    second.message_id = rhizo_mqtt_contract::MessageId::from_uuid(uuid::Uuid::new_v4());
    ingest::persist_replay(&db, &second, 11).await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM device_events WHERE origin='offline_replay'"
        )
        .fetch_one(db.pool())
        .await
        .unwrap(),
        4
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM history_gaps")
            .fetch_one(db.pool())
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM watering_events WHERE origin='offline_autonomous'"
        )
        .fetch_one(db.pool())
        .await
        .unwrap(),
        1
    );
}

#[tokio::test]
async fn quarantine_is_truncated_and_capped() {
    let db = db().await;
    for i in 0..1005 {
        quarantine::insert(&db, Some("node-01"), "topic", &vec![7; 2048], "bad", i)
            .await
            .unwrap();
    }
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM quarantined_messages")
            .fetch_one(db.pool())
            .await
            .unwrap(),
        1000
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT length(payload) FROM quarantined_messages LIMIT 1")
            .fetch_one(db.pool())
            .await
            .unwrap(),
        1024
    );
}

#[tokio::test]
async fn retention_prunes_bounded_data_and_never_ledgers() {
    let db = db().await;
    let now = 200 * 86_400_000i64;
    sqlx::query("INSERT INTO processed_messages VALUES('old','node-01','x',0)")
        .execute(db.pool())
        .await
        .unwrap();
    sqlx::query("INSERT INTO devices(device_id,created_at) VALUES('node-01',0)")
        .execute(db.pool())
        .await
        .unwrap();
    sqlx::query("INSERT INTO measurements(device_id,point,kind,unit,quality,received_at,batch_id) VALUES('node-01','default','x','unknown','ok',0,'b')").execute(db.pool()).await.unwrap();
    sqlx::query("INSERT INTO device_events(event_id,device_id,kind,severity,occurred_at) VALUES('ledger','node-01','boot','info',0)").execute(db.pool()).await.unwrap();
    let p = retention::run_batch(&db, now, 500).await.unwrap();
    assert_eq!((p.processed, p.measurements), (1, 1));
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM device_events")
            .fetch_one(db.pool())
            .await
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn restart_reopen_preserves_history_and_registry() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("edge.sqlite");
    let db = EdgeDb::connect(&path).await.unwrap();
    db.migrate().await.unwrap();
    sqlx::query("INSERT INTO devices(device_id,created_at) VALUES('node-01',1)")
        .execute(db.pool())
        .await
        .unwrap();
    sqlx::query("INSERT INTO measurements(device_id,point,kind,unit,quality,received_at,batch_id) VALUES('node-01','default','soil_moisture','vwc_percent','ok',1,'batch')").execute(db.pool()).await.unwrap();
    db.close().await;
    let reopened = EdgeDb::connect(&path).await.unwrap();
    reopened.migrate().await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM measurements")
            .fetch_one(reopened.pool())
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        rhizo_storage::repo::query::device_count(&reopened)
            .await
            .unwrap(),
        1
    );
}
