//! M3 persistence integration tests.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use rhizo_mqtt_contract::{
    BootId, Envelope, MessageId,
    payload::{
        ActuatorState, CommandResult, DeviceEventBatch, DeviceStatus, DeviceStatusValue,
        TelemetryBatch,
    },
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

async fn remove_marker(db: &EdgeDb, message_id: impl ToString) {
    sqlx::query("DELETE FROM processed_messages WHERE message_id=?")
        .bind(message_id.to_string())
        .execute(db.pool())
        .await
        .unwrap();
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
async fn telemetry_effect_identity_survives_transport_marker_pruning() {
    let db = db().await;
    let e = Envelope::<TelemetryBatch>::from_json(include_bytes!(
        "../../../test/fixtures/protocol/valid/telemetry-batch.json"
    ))
    .unwrap();
    let first = ingest::persist_telemetry(&db, &e, 10).await.unwrap();
    remove_marker(&db, e.message_id).await;
    let mut resealed = e.clone();
    resealed.message_id = rhizo_mqtt_contract::MessageId::from_uuid(uuid::Uuid::new_v4());
    let replay = ingest::persist_telemetry(&db, &resealed, 20).await.unwrap();
    assert_eq!((first.0, replay.0), (Dedup::New, Dedup::Duplicate));
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM measurements")
            .fetch_one(db.pool())
            .await
            .unwrap(),
        e.data.samples.len() as i64
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT last_seen_at FROM devices WHERE device_id=?")
            .bind(e.device_id.to_string())
            .fetch_one(db.pool())
            .await
            .unwrap(),
        10
    );
}

#[tokio::test]
async fn actuator_effect_identity_survives_transport_marker_pruning() {
    let db = db().await;
    let e = Envelope::<ActuatorState>::from_json(include_bytes!(
        "../../../test/fixtures/protocol/valid/actuator.json"
    ))
    .unwrap();
    assert_eq!(
        ingest::persist_actuator(&db, &e, 10).await.unwrap(),
        Dedup::New
    );
    remove_marker(&db, e.message_id).await;
    assert_eq!(
        ingest::persist_actuator(&db, &e, 20).await.unwrap(),
        Dedup::Duplicate
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM actuator_states")
            .fetch_one(db.pool())
            .await
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn command_result_effect_identity_survives_transport_marker_pruning() {
    let db = db().await;
    let e = Envelope::<CommandResult>::from_json(include_bytes!(
        "../../../test/fixtures/protocol/valid/command-result.json"
    ))
    .unwrap();
    assert_eq!(
        ingest::persist_command_result(&db, &e, 10).await.unwrap(),
        Dedup::New
    );
    remove_marker(&db, e.message_id).await;
    let mut resealed = e.clone();
    resealed.message_id = rhizo_mqtt_contract::MessageId::from_uuid(uuid::Uuid::new_v4());
    assert_eq!(
        ingest::persist_command_result(&db, &resealed, 20)
            .await
            .unwrap(),
        Dedup::Duplicate
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM command_results")
            .fetch_one(db.pool())
            .await
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn replay_effect_identities_survive_transport_marker_pruning() {
    let db = db().await;
    let e = Envelope::<DeviceEventBatch>::from_json(include_bytes!(
        "../../../test/fixtures/protocol/valid/events-replay-gap.json"
    ))
    .unwrap();
    assert_eq!(
        ingest::persist_replay(&db, &e, 10).await.unwrap().dedup,
        Dedup::New
    );
    remove_marker(&db, e.message_id).await;
    assert_eq!(
        ingest::persist_replay(&db, &e, 20).await.unwrap().dedup,
        Dedup::Duplicate
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM device_events")
            .fetch_one(db.pool())
            .await
            .unwrap(),
        e.data.events.len() as i64
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM history_gaps")
            .fetch_one(db.pool())
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM watering_events")
            .fetch_one(db.pool())
            .await
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn status_effect_is_ordered_after_transport_marker_pruning() {
    let db = db().await;
    let original = Envelope::<DeviceStatus>::from_json(include_bytes!(
        "../../../test/fixtures/protocol/valid/status-with-capabilities.json"
    ))
    .unwrap();
    assert_eq!(
        ingest::persist_status(&db, &original, 10).await.unwrap(),
        Dedup::New
    );
    remove_marker(&db, original.message_id).await;
    assert_eq!(
        ingest::persist_status(&db, &original, 20).await.unwrap(),
        Dedup::Duplicate
    );
    let mut newer = original.clone();
    newer.message_id = MessageId::from_uuid(uuid::Uuid::new_v4());
    newer.sequence = Some(original.sequence.unwrap() + 1);
    assert_eq!(
        ingest::persist_status(&db, &newer, 30).await.unwrap(),
        Dedup::New
    );

    let mut new_boot = original.clone();
    new_boot.message_id = MessageId::from_uuid(uuid::Uuid::new_v4());
    new_boot.boot_id = Some(BootId::from_uuid(uuid::Uuid::new_v4()));
    new_boot.sequence = Some(1);
    new_boot.data.boot_generation += 1;
    assert_eq!(
        ingest::persist_status(&db, &new_boot, 40).await.unwrap(),
        Dedup::New
    );

    let mut delayed_old_boot = original.clone();
    delayed_old_boot.message_id = MessageId::from_uuid(uuid::Uuid::new_v4());
    delayed_old_boot.sequence = Some(99);
    assert_eq!(
        ingest::persist_status(&db, &delayed_old_boot, 50)
            .await
            .unwrap(),
        Dedup::Duplicate
    );

    let mut lwt = new_boot.clone();
    lwt.message_id = MessageId::from_uuid(uuid::Uuid::new_v4());
    lwt.sequence = Some(0);
    lwt.data.status = DeviceStatusValue::Offline;
    lwt.data.reason = Some("connection_lost".into());
    assert_eq!(
        ingest::persist_status(&db, &lwt, 60).await.unwrap(),
        Dedup::New
    );
    remove_marker(&db, lwt.message_id).await;
    assert_eq!(
        ingest::persist_status(&db, &lwt, 70).await.unwrap(),
        Dedup::Duplicate
    );
    let mut delayed_same_boot = new_boot.clone();
    delayed_same_boot.message_id = MessageId::from_uuid(uuid::Uuid::new_v4());
    assert_eq!(
        ingest::persist_status(&db, &delayed_same_boot, 80)
            .await
            .unwrap(),
        Dedup::Duplicate
    );
    let mut reconnected = new_boot.clone();
    reconnected.message_id = MessageId::from_uuid(uuid::Uuid::new_v4());
    reconnected.sequence = Some(2);
    assert_eq!(
        ingest::persist_status(&db, &reconnected, 90).await.unwrap(),
        Dedup::New
    );
    assert_eq!(
        ingest::persist_status(&db, &lwt, 100).await.unwrap(),
        Dedup::Duplicate
    );

    let pruned = retention::run_batch(&db, 8 * 86_400_000, 500)
        .await
        .unwrap();
    assert_eq!(pruned.processed, 3);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT last_seen_at FROM devices WHERE device_id=?")
            .bind(original.device_id.to_string())
            .fetch_one(db.pool())
            .await
            .unwrap(),
        90
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT status_boot_generation FROM devices WHERE device_id=?"
        )
        .bind(original.device_id.to_string())
        .fetch_one(db.pool())
        .await
        .unwrap(),
        new_boot.data.boot_generation as i64
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

/// `storage_bytes` is the gauge ADR-004 and the failure model watch to see a
/// disk filling before `SQLITE_FULL` makes every write fatal, so it has to
/// report a real size and grow with the data.
#[tokio::test]
async fn storage_bytes_reports_a_real_size_that_grows() {
    let db = db().await;
    let empty = rhizo_storage::repo::query::storage_bytes(&db)
        .await
        .unwrap();
    assert!(empty > 0, "a migrated database is never zero bytes");

    let e = Envelope::<TelemetryBatch>::from_json(include_bytes!(
        "../../../test/fixtures/protocol/valid/telemetry-batch.json"
    ))
    .unwrap();
    for i in 0..200 {
        let mut batch = e.clone();
        batch.message_id = MessageId::from_uuid(uuid::Uuid::new_v4());
        batch.data.batch_id = uuid::Uuid::new_v4();
        ingest::persist_telemetry(&db, &batch, 1_000 + i)
            .await
            .unwrap();
    }
    assert!(
        rhizo_storage::repo::query::storage_bytes(&db)
            .await
            .unwrap()
            > empty,
        "the gauge must move as rows are written"
    );
}
