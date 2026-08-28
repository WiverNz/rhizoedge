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

/// `storage_bytes` must be the footprint the filesystem would report, because
/// that is what exhausts the volume. In WAL mode the log can outgrow the main
/// database between checkpoints, so all three files are counted.
#[tokio::test]
async fn storage_bytes_reports_the_real_on_disk_footprint_and_grows() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("edge.sqlite");
    let db = EdgeDb::connect(&path).await.unwrap();
    db.migrate().await.unwrap();

    let empty = rhizo_storage::repo::query::storage_bytes(&db)
        .await
        .unwrap();
    assert!(empty > 0, "a migrated database is never zero bytes");

    let on_disk = |suffix: &str| {
        let mut file = path.clone().into_os_string();
        file.push(suffix);
        std::fs::metadata(std::path::Path::new(&file)).map_or(0, |m| m.len() as i64)
    };
    assert_eq!(
        empty,
        on_disk("") + on_disk("-wal") + on_disk("-shm"),
        "the gauge is the sum of the database, its write-ahead log, and its shared-memory index"
    );
    assert!(on_disk("-wal") > 0, "WAL mode is active, so the log exists");

    let e = Envelope::<TelemetryBatch>::from_json(include_bytes!(
        "../../../test/fixtures/protocol/valid/telemetry-batch.json"
    ))
    .unwrap();
    for i in 0..300 {
        let mut batch = e.clone();
        batch.message_id = MessageId::from_uuid(uuid::Uuid::new_v4());
        batch.data.batch_id = uuid::Uuid::new_v4();
        ingest::persist_telemetry(&db, &batch, 1_000 + i)
            .await
            .unwrap();
    }
    let grown = rhizo_storage::repo::query::storage_bytes(&db)
        .await
        .unwrap();
    assert!(
        grown > empty,
        "the gauge must move as rows are written: {empty} -> {grown}"
    );
    db.close().await;
}

/// An in-memory database has no files, so the gauge falls back to SQLite's own
/// page accounting rather than silently reporting zero.
#[tokio::test]
async fn storage_bytes_falls_back_to_page_accounting_in_memory() {
    let db = db().await;
    assert!(
        rhizo_storage::repo::query::storage_bytes(&db)
            .await
            .unwrap()
            > 0,
        "an in-memory database still reports a real allocated size"
    );
}

// ---------------------------------------------------------------------------
// Replay acknowledgement prefix semantics (protocol section 5.13).
//
// `device_seq` is zero-based, so "nothing is committed" and "sequence 0 is
// committed" are different facts. `ReplayCommit::through_device_seq` is
// `Option<u64>` for that reason, and `None` means the edge publishes no
// acknowledgement at all.
// ---------------------------------------------------------------------------

/// Builds a replay batch carrying exactly `seqs`, with an `event_id` derived
/// from the sequence so a repartitioned replay is recognisably the same events.
fn replay_batch(seqs: &[u64], complete: bool) -> Envelope<DeviceEventBatch> {
    let events: Vec<serde_json::Value> = seqs
        .iter()
        .map(|seq| {
            serde_json::json!({
                "event_id": format!("018fd7c0-0000-7000-8000-{seq:012}"),
                "device_seq": seq,
                "tier": "audit",
                "kind": "policy.activated",
                "monotonic_ms": 1_000 + seq,
                "detail": {"detail_type": "policy_activated", "policy_version": 7},
            })
        })
        .collect();
    let value = serde_json::json!({
        "v": 1,
        "kind": "device.events",
        "message_id": uuid::Uuid::new_v4(),
        "device_id": "plant-node-01",
        "boot_id": "018fd6b0-1122-4000-8000-aabbccddeeff",
        "sequence": 41,
        "device_time_ms": 1_756_121_400_000i64,
        "clock_synced": true,
        "data": {"replay": true, "complete": complete, "events": events},
    });
    Envelope::from_json(&serde_json::to_vec(&value).unwrap()).unwrap()
}

/// A replay of exactly sequence 0 is acknowledged with 0, not confused with
/// "nothing". This is the case the old `u64` representation could not express.
#[tokio::test]
async fn a_replay_of_sequence_zero_is_acknowledged_with_zero() {
    let db = db().await;
    let commit = ingest::persist_replay(&db, &replay_batch(&[0], true), 10)
        .await
        .unwrap();
    assert_eq!(commit.dedup, Dedup::New);
    assert_eq!(commit.through_device_seq, Some(0));
}

/// A suffix-only replay — the device's buffer starts above anything the edge
/// holds — is committed but acknowledges nothing. Acknowledging 0 here would
/// tell the device to discard sequence 0, which the edge does not have.
#[tokio::test]
async fn a_suffix_only_replay_commits_but_acknowledges_nothing() {
    let db = db().await;
    let commit = ingest::persist_replay(&db, &replay_batch(&[118, 119, 120, 121], true), 10)
        .await
        .unwrap();
    assert_eq!(commit.dedup, Dedup::New);
    assert_eq!(commit.through_device_seq, None);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM device_events WHERE device_seq >= 118")
            .fetch_one(db.pool())
            .await
            .unwrap(),
        4,
        "the events are still durably committed; only the acknowledgement is withheld"
    );
    assert!(
        sqlx::query_scalar::<_, Option<i64>>("SELECT through_device_seq FROM replay_progress")
            .fetch_one(db.pool())
            .await
            .unwrap()
            .is_none(),
        "no progress is recorded, so the column stays NULL rather than 0"
    );
}

#[tokio::test]
async fn a_contiguous_replay_from_zero_is_acknowledged_through_its_last_sequence() {
    let db = db().await;
    let commit = ingest::persist_replay(&db, &replay_batch(&[0, 1, 2, 3], true), 10)
        .await
        .unwrap();
    assert_eq!(commit.through_device_seq, Some(3));
}

/// The hole case: a suffix arrives first and is withheld, then the missing
/// prefix arrives and the acknowledgement jumps to cover both.
#[tokio::test]
async fn a_late_prefix_completes_a_withheld_suffix() {
    let db = db().await;
    let suffix = ingest::persist_replay(&db, &replay_batch(&[3, 4], false), 10)
        .await
        .unwrap();
    assert_eq!(suffix.through_device_seq, None, "3 and 4 skip a hole");

    let prefix = ingest::persist_replay(&db, &replay_batch(&[0, 1, 2], true), 20)
        .await
        .unwrap();
    assert_eq!(
        prefix.through_device_seq,
        Some(4),
        "the prefix closes the hole, so the whole run becomes acknowledgeable"
    );
}

/// A partial prefix is acknowledged only as far as it is contiguous.
#[tokio::test]
async fn a_prefix_with_a_hole_is_acknowledged_only_up_to_the_hole() {
    let db = db().await;
    let commit = ingest::persist_replay(&db, &replay_batch(&[0, 1, 3, 4], true), 10)
        .await
        .unwrap();
    assert_eq!(commit.through_device_seq, Some(1));
}

/// The same events resent in different batch shapes are one logical replay:
/// `event_id` deduplicates them and the prefix never moves backwards.
#[tokio::test]
async fn a_repartitioned_replay_is_idempotent_and_never_lowers_the_prefix() {
    let db = db().await;
    assert_eq!(
        ingest::persist_replay(&db, &replay_batch(&[0, 1, 2], false), 10)
            .await
            .unwrap()
            .through_device_seq,
        Some(2)
    );
    // Same events, different partitioning, fresh transport ids.
    let repartitioned = ingest::persist_replay(&db, &replay_batch(&[0, 1], false), 20)
        .await
        .unwrap();
    assert_eq!(repartitioned.dedup, Dedup::Duplicate);
    assert_eq!(
        repartitioned.through_device_seq,
        Some(2),
        "a re-sent earlier slice must not lower the acknowledged prefix"
    );
    let extended = ingest::persist_replay(&db, &replay_batch(&[2, 3, 4], true), 30)
        .await
        .unwrap();
    assert_eq!(extended.through_device_seq, Some(4));
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM device_events WHERE origin='offline_replay'"
        )
        .fetch_one(db.pool())
        .await
        .unwrap(),
        5,
        "five distinct event ids across four overlapping batches"
    );
}

/// An exact transport duplicate short-circuits on the marker but still reports
/// the prefix already committed, so a lost acknowledgement can be re-derived.
#[tokio::test]
async fn an_exact_duplicate_replay_reports_the_committed_prefix() {
    let db = db().await;
    let batch = replay_batch(&[0, 1], true);
    assert_eq!(
        ingest::persist_replay(&db, &batch, 10)
            .await
            .unwrap()
            .through_device_seq,
        Some(1)
    );
    let duplicate = ingest::persist_replay(&db, &batch, 20).await.unwrap();
    assert_eq!(duplicate.dedup, Dedup::Duplicate);
    assert_eq!(duplicate.through_device_seq, Some(1));
}

/// A duplicate arriving for a boot that never produced a committed prefix must
/// report `None`, not a fabricated zero.
#[tokio::test]
async fn a_duplicate_of_a_suffix_only_replay_still_reports_nothing() {
    let db = db().await;
    let batch = replay_batch(&[118, 119], true);
    assert_eq!(
        ingest::persist_replay(&db, &batch, 10)
            .await
            .unwrap()
            .through_device_seq,
        None
    );
    let duplicate = ingest::persist_replay(&db, &batch, 20).await.unwrap();
    assert_eq!(duplicate.dedup, Dedup::Duplicate);
    assert_eq!(duplicate.through_device_seq, None);
}
