//! Transaction-only ingestion writes.
#![allow(missing_docs)]

use rhizo_mqtt_contract::Envelope;
use rhizo_mqtt_contract::payload::{
    ActuatorState, DeviceEventBatch, DeviceStatus, DeviceStatusValue, EventDetail, EventKind,
    EventTier, MeasurementSample, MeasurementValue,
};
use sqlx::{Sqlite, Transaction};

use crate::{EdgeDb, StorageError};

/// Durable message deduplication result.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Dedup {
    New,
    Duplicate,
}

fn json_name<T: serde::Serialize>(value: &T) -> Result<String, StorageError> {
    let v = serde_json::to_value(value).map_err(|e| StorageError::Serialization(e.to_string()))?;
    Ok(v.as_str().map_or_else(|| v.to_string(), ToOwned::to_owned))
}

/// Inserts the marker that must share the effects transaction.
pub async fn mark_processed(
    tx: &mut Transaction<'_, Sqlite>,
    message_id: &str,
    device_id: &str,
    kind: &str,
    received_at: i64,
) -> Result<Dedup, StorageError> {
    let n = sqlx::query!(
        "INSERT INTO processed_messages(message_id,device_id,kind,received_at) VALUES(?,?,?,?) ON CONFLICT(message_id) DO NOTHING",
        message_id,
        device_id,
        kind,
        received_at
    )
    .execute(&mut **tx)
    .await
    .map_err(StorageError::from_sqlx)?
    .rows_affected();
    Ok(if n == 0 { Dedup::Duplicate } else { Dedup::New })
}

async fn touch_device(
    tx: &mut Transaction<'_, Sqlite>,
    device: &str,
    boot: Option<String>,
    seq: Option<u64>,
    received: i64,
) -> Result<(), StorageError> {
    let previous = sqlx::query(
        "SELECT boot_id,last_sequence,last_seen_at,telemetry_interval_seconds FROM devices WHERE device_id=?",
    ).bind(device)
    .fetch_optional(&mut **tx)
    .await
    .map_err(StorageError::from_sqlx)?;
    use sqlx::Row as _;
    if let Some(row) = previous.as_ref() {
        let old_boot: Option<String> = row.get("boot_id");
        let old_sequence: Option<i64> = row.get("last_sequence");
        if boot != old_boot {
            let event_id = format!("boot:{device}:{}", boot.as_deref().unwrap_or("unknown"));
            sqlx::query!(
                "INSERT INTO device_events(event_id,device_id,kind,severity,occurred_at,received_at) VALUES(?,?,'boot','info',?,?) ON CONFLICT(event_id) DO NOTHING",
                event_id,
                device,
                received,
                received
            )
            .execute(&mut **tx)
            .await
            .map_err(StorageError::from_sqlx)?;
            if old_boot.is_some()
                && row
                    .get::<Option<i64>, _>("last_seen_at")
                    .is_some_and(|last| {
                        received.saturating_sub(last)
                            < row
                                .get::<i64, _>("telemetry_interval_seconds")
                                .saturating_mul(2_000)
                    })
            {
                let conflict_id = format!("boot-thrash:{device}:{received}");
                sqlx::query("INSERT OR IGNORE INTO device_events(event_id,device_id,kind,severity,occurred_at,received_at,origin) VALUES(?,?,'boot_id_thrash','warning',?,?,'edge')")
                .bind(conflict_id).bind(device).bind(received).bind(received)
                .execute(&mut **tx).await.map_err(StorageError::from_sqlx)?;
            }
        } else if let (Some(old), Some(new)) = (old_sequence, seq)
            && (new as i64) < old
        {
            let event_id = format!(
                "sequence-regression:{device}:{}:{new}",
                boot.as_deref().unwrap_or("unknown")
            );
            sqlx::query!(
                "INSERT INTO device_events(event_id,device_id,kind,severity,occurred_at,received_at) VALUES(?,?,'sequence_regression','warning',?,?) ON CONFLICT(event_id) DO NOTHING",
                event_id,
                device,
                received,
                received
            )
            .execute(&mut **tx)
            .await
            .map_err(StorageError::from_sqlx)?;
        }
    }
    let is_new = previous.is_none();
    let sequence = seq.map(|v| v as i64);
    sqlx::query!(
        "INSERT INTO devices(device_id,boot_id,last_sequence,last_seen_at,created_at) VALUES(?,?,?,?,?) ON CONFLICT(device_id) DO UPDATE SET boot_id=excluded.boot_id,last_sequence=excluded.last_sequence,last_seen_at=excluded.last_seen_at",
        device,
        boot,
        sequence,
        received,
        received
    )
    .execute(&mut **tx)
    .await
    .map_err(StorageError::from_sqlx)?;
    if is_new {
        insert_edge_event(tx, device, "device_registered", "info", None, received).await?;
    }
    Ok(())
}

/// Persists a live telemetry envelope atomically with its marker and outbox row.
pub async fn persist_telemetry(
    db: &EdgeDb,
    envelope: &Envelope<rhizo_mqtt_contract::payload::TelemetryBatch>,
    received_at: i64,
) -> Result<(Dedup, usize), StorageError> {
    let mut tx = db.begin().await?;
    let device = envelope.device_id.to_string();
    let message = envelope.message_id.to_string();
    if mark_processed(&mut tx, &message, &device, "telemetry.batch", received_at).await?
        == Dedup::Duplicate
    {
        tx.rollback().await.map_err(StorageError::from_sqlx)?;
        return Ok((Dedup::Duplicate, 0));
    }
    let batch_id = envelope.data.batch_id.to_string();
    if sqlx::query_scalar!(
        r#"SELECT EXISTS(SELECT 1 FROM measurements WHERE device_id=? AND batch_id=?) AS "present!: i64""#,
        device,
        batch_id
    )
    .fetch_one(&mut *tx)
    .await
    .map_err(StorageError::from_sqlx)?
        != 0
    {
        tx.rollback().await.map_err(StorageError::from_sqlx)?;
        return Ok((Dedup::Duplicate, 0));
    }
    touch_device(
        &mut tx,
        &device,
        envelope.boot_id.as_ref().map(ToString::to_string),
        envelope.sequence,
        received_at,
    )
    .await?;
    let mut accepted = 0;
    for (sample_index, sample) in envelope.data.samples.iter().enumerate() {
        let valid = sample.validate().is_valid();
        insert_sample(
            &mut tx,
            &device,
            &batch_id,
            sample,
            valid,
            received_at,
            envelope.device_time_ms.map(|v| v.0),
            envelope.boot_id.as_ref().map(ToString::to_string),
            envelope.sequence,
            "live",
            Some(&message),
            Some(sample_index as i64),
        )
        .await?;
        if valid {
            accepted += 1
        } else {
            record_invalid(&mut tx, &device, &message, sample, received_at).await?;
        }
    }
    enqueue(&mut tx, &message, "telemetry.batch", received_at).await?;
    tx.commit().await.map_err(StorageError::from_sqlx)?;
    Ok((Dedup::New, accepted))
}

#[allow(
    clippy::too_many_arguments,
    reason = "one explicit value per narrow measurement column keeps the SQL reviewable"
)]
async fn insert_sample(
    tx: &mut Transaction<'_, Sqlite>,
    device: &str,
    batch: &str,
    s: &MeasurementSample,
    valid: bool,
    received: i64,
    device_time: Option<i64>,
    boot: Option<String>,
    sequence: Option<u64>,
    origin: &str,
    source_message_id: Option<&str>,
    sample_index: Option<i64>,
) -> Result<(), StorageError> {
    let (num, bval) = if valid {
        match s.value {
            Some(MeasurementValue::Scalar(v)) => (Some(v), None),
            Some(MeasurementValue::Boolean(v)) => (None, Some(i64::from(v))),
            None => (None, None),
        }
    } else {
        (None, None)
    };
    let sensor_id = s.sensor_id.as_ref().map(|v| v.as_str());
    let point = s.point.as_str();
    let kind = json_name(&s.kind)?;
    let unit = json_name(&s.unit)?;
    let quality = json_name(&s.quality)?;
    let calibration_ref = s.calibration_ref.as_ref().map(|v| v.as_str());
    let sequence = sequence.map(|v| v as i64);
    sqlx::query!(
        "INSERT INTO measurements(device_id,sensor_id,point,kind,value_num,value_bool,unit,quality,calibration_ref,received_at,device_time_ms,boot_id,sequence,batch_id,origin,source_message_id,sample_index) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)",
        device,
        sensor_id,
        point,
        kind,
        num,
        bval,
        unit,
        quality,
        calibration_ref,
        received,
        device_time,
        boot,
        sequence,
        batch,
        origin,
        source_message_id,
        sample_index
    )
    .execute(&mut **tx)
    .await
    .map_err(StorageError::from_sqlx)?;
    Ok(())
}
async fn record_invalid(
    tx: &mut Transaction<'_, Sqlite>,
    device: &str,
    message: &str,
    s: &MeasurementSample,
    at: i64,
) -> Result<(), StorageError> {
    let id = format!(
        "invalid:{message}:{}:{}",
        s.point.as_str(),
        json_name(&s.kind)?
    );
    let detail =
        serde_json::to_string(s).map_err(|e| StorageError::Serialization(e.to_string()))?;
    sqlx::query!(
        "INSERT INTO device_events(event_id,device_id,kind,severity,detail_json,occurred_at,received_at) VALUES(?,?,'sensor_invalid','warning',?,?,?)",
        id,
        device,
        detail,
        at,
        at
    )
    .execute(&mut **tx)
    .await
    .map_err(StorageError::from_sqlx)?;
    Ok(())
}
async fn enqueue(
    tx: &mut Transaction<'_, Sqlite>,
    id: &str,
    kind: &str,
    at: i64,
) -> Result<(), StorageError> {
    sqlx::query!(
        "INSERT INTO pending_cloud_events(event_id,kind,value_tier,payload_json,status,next_attempt_at,created_at) VALUES(?,?,'low','{}','pending',?,?) ON CONFLICT(event_id) DO NOTHING",
        id,
        kind,
        at,
        at
    )
    .execute(&mut **tx)
    .await
    .map_err(StorageError::from_sqlx)?;
    Ok(())
}

/// Persists current actuator state separately from measurements.
pub async fn persist_actuator(
    db: &EdgeDb,
    e: &Envelope<ActuatorState>,
    at: i64,
) -> Result<Dedup, StorageError> {
    let mut tx = db.begin().await?;
    let id = e.message_id.to_string();
    let dev = e.device_id.to_string();
    if mark_processed(&mut tx, &id, &dev, "actuator.state", at).await? == Dedup::Duplicate {
        tx.rollback().await.map_err(StorageError::from_sqlx)?;
        return Ok(Dedup::Duplicate);
    }
    if sqlx::query_scalar!(
        r#"SELECT EXISTS(SELECT 1 FROM actuator_states WHERE message_id=?) AS "present!: i64""#,
        id
    )
    .fetch_one(&mut *tx)
    .await
    .map_err(StorageError::from_sqlx)?
        != 0
    {
        tx.rollback().await.map_err(StorageError::from_sqlx)?;
        return Ok(Dedup::Duplicate);
    }
    touch_device(
        &mut tx,
        &dev,
        e.boot_id.as_ref().map(ToString::to_string),
        e.sequence,
        at,
    )
    .await?;
    let actuator_id = e.data.actuator_id.as_str();
    let kind = json_name(&e.data.kind)?;
    let state_json =
        serde_json::to_string(&e.data).map_err(|x| StorageError::Serialization(x.to_string()))?;
    let device_time_ms = e.device_time_ms.map(|v| v.0);
    let boot_id = e.boot_id.as_ref().map(ToString::to_string);
    let sequence = e.sequence.map(|v| v as i64);
    sqlx::query!(
        "INSERT INTO actuator_states(message_id,device_id,actuator_id,kind,state_json,received_at,device_time_ms,boot_id,sequence) VALUES(?,?,?,?,?,?,?,?,?)",
        id,
        dev,
        actuator_id,
        kind,
        state_json,
        at,
        device_time_ms,
        boot_id,
        sequence
    )
    .execute(&mut *tx)
    .await
    .map_err(StorageError::from_sqlx)?;
    tx.commit().await.map_err(StorageError::from_sqlx)?;
    Ok(Dedup::New)
}

/// Persists a device command result as transport history without applying M6 semantics.
pub async fn persist_command_result(
    db: &EdgeDb,
    e: &Envelope<rhizo_mqtt_contract::payload::CommandResult>,
    at: i64,
) -> Result<Dedup, StorageError> {
    let mut tx = db.begin().await?;
    let id = e.message_id.to_string();
    let dev = e.device_id.to_string();
    if mark_processed(&mut tx, &id, &dev, "command.result", at).await? == Dedup::Duplicate {
        tx.rollback().await.map_err(StorageError::from_sqlx)?;
        return Ok(Dedup::Duplicate);
    }
    let command_id = e.data.command_id.to_string();
    if sqlx::query_scalar!(
        r#"SELECT EXISTS(SELECT 1 FROM command_results WHERE command_id=?) AS "present!: i64""#,
        command_id
    )
    .fetch_one(&mut *tx)
    .await
    .map_err(StorageError::from_sqlx)?
        != 0
    {
        tx.rollback().await.map_err(StorageError::from_sqlx)?;
        return Ok(Dedup::Duplicate);
    }
    touch_device(
        &mut tx,
        &dev,
        e.boot_id.as_ref().map(ToString::to_string),
        e.sequence,
        at,
    )
    .await?;
    let result_json =
        serde_json::to_string(&e.data).map_err(|x| StorageError::Serialization(x.to_string()))?;
    let device_time_ms = e.device_time_ms.map(|v| v.0);
    let boot_id = e.boot_id.as_ref().map(ToString::to_string);
    let sequence = e.sequence.map(|v| v as i64);
    sqlx::query!(
        "INSERT INTO command_results(message_id,command_id,device_id,result_json,received_at,device_time_ms,boot_id,sequence) VALUES(?,?,?,?,?,?,?,?)",
        id,
        command_id,
        dev,
        result_json,
        at,
        device_time_ms,
        boot_id,
        sequence
    )
    .execute(&mut *tx)
    .await
    .map_err(StorageError::from_sqlx)?;
    tx.commit().await.map_err(StorageError::from_sqlx)?;
    Ok(Dedup::New)
}

/// Replay commit result used to publish an acknowledgement only after commit.
pub struct ReplayCommit {
    pub dedup: Dedup,
    /// The highest contiguous committed `device_seq`, or `None` when no
    /// contiguous prefix is committed at all.
    ///
    /// `device_seq` is zero-based, so `Some(0)` and "nothing" are different
    /// facts and cannot share a representation. Protocol section 5.13 makes
    /// `through_device_seq` a prefix, and a device that receives one discards
    /// everything at or below it; acknowledging 0 to mean "nothing" would tell
    /// the device to discard the single event the edge does not hold. `None`
    /// means the edge publishes no acknowledgement and the device replays again.
    pub through_device_seq: Option<u64>,
    pub complete: bool,
}

/// Persists replay events idempotently by stable event id.
pub async fn persist_replay(
    db: &EdgeDb,
    e: &Envelope<DeviceEventBatch>,
    at: i64,
) -> Result<ReplayCommit, StorageError> {
    let mut tx = db.begin().await?;
    let msg = e.message_id.to_string();
    let dev = e.device_id.to_string();
    let boot = e
        .boot_id
        .as_ref()
        .map(ToString::to_string)
        .unwrap_or_default();
    if mark_processed(&mut tx, &msg, &dev, "device.events", at).await? == Dedup::Duplicate {
        tx.rollback().await.map_err(StorageError::from_sqlx)?;
        let through = replay_through(db, &dev, &boot).await?;
        return Ok(ReplayCommit {
            dedup: Dedup::Duplicate,
            through_device_seq: through,
            complete: e.data.complete,
        });
    }
    if !e.data.events.is_empty() {
        let mut all_events_exist = true;
        for event in &e.data.events {
            let event_id = event.event_id.to_string();
            let exists = sqlx::query_scalar!(
                r#"SELECT EXISTS(SELECT 1 FROM device_events WHERE event_id=?) AS "present!: i64""#,
                event_id
            )
            .fetch_one(&mut *tx)
            .await
            .map_err(StorageError::from_sqlx)?;
            if exists == 0 {
                all_events_exist = false;
                break;
            }
        }
        if all_events_exist {
            tx.rollback().await.map_err(StorageError::from_sqlx)?;
            let through = replay_through(db, &dev, &boot).await?;
            return Ok(ReplayCommit {
                dedup: Dedup::Duplicate,
                through_device_seq: through,
                complete: e.data.complete,
            });
        }
    }
    for event in &e.data.events {
        let eid = event.event_id.to_string();
        let kind = json_name(&event.kind)?;
        let detail = serde_json::to_string(&event.detail)
            .map_err(|x| StorageError::Serialization(x.to_string()))?;
        let severity = if event.tier == EventTier::Audit {
            "warning"
        } else {
            "info"
        };
        let occurred_at = event.device_time_ms.map_or(at, |v| v.0);
        let device_seq = event.device_seq as i64;
        sqlx::query!(
            "INSERT INTO device_events(event_id,device_id,kind,severity,detail_json,occurred_at,received_at,boot_id,device_seq,origin) VALUES(?,?,?,?,?,?,?,?,?,'offline_replay') ON CONFLICT(event_id) DO NOTHING",
            eid,
            dev,
            kind,
            severity,
            detail,
            occurred_at,
            at,
            boot,
            device_seq
        )
        .execute(&mut *tx)
        .await
        .map_err(StorageError::from_sqlx)?;
        if let EventDetail::Gap {
            from_seq,
            to_seq,
            lost_count,
            lost_tier,
        } = event.detail
        {
            let (from_seq, to_seq, lost_count) =
                (from_seq as i64, to_seq as i64, lost_count as i64);
            let tier = json_name(&lost_tier)?;
            sqlx::query!(
                "INSERT INTO history_gaps(gap_id,device_id,boot_id,from_seq,to_seq,lost_count,tier,reported_at) VALUES(?,?,?,?,?,?,?,?) ON CONFLICT(gap_id) DO NOTHING",
                eid,
                dev,
                boot,
                from_seq,
                to_seq,
                lost_count,
                tier,
                at
            )
            .execute(&mut *tx)
            .await
            .map_err(StorageError::from_sqlx)?;
        }
        if let (EventKind::WateringOfflineAutonomous, EventDetail::Watering { delivered_ml, .. }) =
            (&event.kind, &event.detail)
        {
            let happened_at = event.device_time_ms.map_or(at, |v| v.0);
            let delivered = f64::from(*delivered_ml);
            sqlx::query!(
                "INSERT INTO watering_events(watering_event_id,device_id,mode,origin,started_at,completed_at,delivered_ml,status) VALUES(?,?,'automatic','offline_autonomous',?,?,?,'completed') ON CONFLICT(watering_event_id) DO NOTHING",
                eid,
                dev,
                happened_at,
                happened_at,
                delivered
            )
            .execute(&mut *tx)
            .await
            .map_err(StorageError::from_sqlx)?;
        }
    }
    let through = contiguous_in_tx(&mut tx, &dev, &boot).await?;
    // SQLite's `max(a,b)` returns NULL if either argument is NULL, so the
    // no-progress case has to be spelled out rather than folded into it.
    let progress = through.map(|v| v as i64);
    let complete = i64::from(e.data.complete);
    sqlx::query!(
        "INSERT INTO replay_progress(device_id,boot_id,through_device_seq,complete,updated_at) VALUES(?,?,?,?,?) ON CONFLICT(device_id,boot_id) DO UPDATE SET through_device_seq=CASE WHEN excluded.through_device_seq IS NULL THEN through_device_seq WHEN through_device_seq IS NULL THEN excluded.through_device_seq ELSE max(through_device_seq,excluded.through_device_seq) END,complete=max(complete,excluded.complete),updated_at=excluded.updated_at",
        dev,
        boot,
        progress,
        complete,
        at
    )
    .execute(&mut *tx)
    .await
    .map_err(StorageError::from_sqlx)?;
    tx.commit().await.map_err(StorageError::from_sqlx)?;
    Ok(ReplayCommit {
        dedup: Dedup::New,
        through_device_seq: through,
        complete: e.data.complete,
    })
}
/// The highest `device_seq` such that every sequence at or below it is committed.
///
/// Returns `None` when the committed events form no prefix at all — a
/// suffix-only replay, where the device's buffer starts above anything the edge
/// holds. Protocol section 5.13 is explicit that a prefix which skips a hole is
/// a lie about what the edge holds, so the honest answer there is "nothing",
/// not zero.
async fn contiguous_in_tx(
    tx: &mut Transaction<'_, Sqlite>,
    dev: &str,
    boot: &str,
) -> Result<Option<u64>, StorageError> {
    let seqs = sqlx::query_scalar!(
        r#"SELECT device_seq AS "device_seq!: i64" FROM device_events WHERE device_id=? AND boot_id=? AND device_seq IS NOT NULL ORDER BY device_seq"#,
        dev,
        boot
    )
    .fetch_all(&mut **tx)
    .await
    .map_err(StorageError::from_sqlx)?;
    let mut through = progress_in_tx(tx, dev, boot).await?;
    let mut expected = through.map_or(0, |v| v + 1);
    for s in seqs {
        let s = s as u64;
        if s == expected {
            through = Some(s);
            expected += 1;
        } else if s > expected {
            // A hole. Everything past it stays unacknowledged and the device
            // replays it, which is the whole point of a cumulative prefix.
            break;
        }
        // s < expected: already covered by an earlier batch, so skip it.
    }
    Ok(through)
}
async fn progress_in_tx(
    tx: &mut Transaction<'_, Sqlite>,
    dev: &str,
    boot: &str,
) -> Result<Option<u64>, StorageError> {
    Ok(sqlx::query_scalar!(
        "SELECT through_device_seq FROM replay_progress WHERE device_id=? AND boot_id=?",
        dev,
        boot
    )
    .fetch_optional(&mut **tx)
    .await
    .map_err(StorageError::from_sqlx)?
    .flatten()
    .map(|v| v as u64))
}
async fn replay_through(db: &EdgeDb, dev: &str, boot: &str) -> Result<Option<u64>, StorageError> {
    Ok(sqlx::query_scalar!(
        "SELECT through_device_seq FROM replay_progress WHERE device_id=? AND boot_id=?",
        dev,
        boot
    )
    .fetch_optional(db.pool())
    .await
    .map_err(StorageError::from_sqlx)?
    .flatten()
    .map(|v| v as u64))
}

/// Persists status as raw prerequisite state without M4 health behaviour.
///
/// Transport ids are only a short-lived QoS optimisation. The durable effect
/// is ordered by the device's persisted boot generation and per-boot sequence;
/// an LWT is a single terminal logical status within its boot and is remembered
/// by its fixed id. This leaves one bounded high-water row per device.
pub async fn persist_status(
    db: &EdgeDb,
    e: &Envelope<DeviceStatus>,
    at: i64,
) -> Result<Dedup, StorageError> {
    let mut tx = db.begin().await?;
    let id = e.message_id.to_string();
    let dev = e.device_id.to_string();
    if mark_processed(&mut tx, &id, &dev, "device.status", at).await? == Dedup::Duplicate {
        tx.rollback().await.map_err(StorageError::from_sqlx)?;
        return Ok(Dedup::Duplicate);
    }
    let generation = i64::try_from(e.data.boot_generation).map_err(|_| {
        StorageError::Constraint("status boot_generation exceeds SQLite INTEGER".into())
    })?;
    if generation == 0 {
        return Err(StorageError::Constraint(
            "status boot_generation must be positive".into(),
        ));
    }
    let sequence = i64::try_from(e.sequence.unwrap_or(0))
        .map_err(|_| StorageError::Constraint("status sequence exceeds SQLite INTEGER".into()))?;
    let is_lwt = sequence == 0
        && e.data.status == DeviceStatusValue::Offline
        && e.data.reason.as_deref() == Some("connection_lost");
    let previous = sqlx::query!(
        "SELECT status_boot_generation,status_sequence,status_lwt_message_id FROM devices WHERE device_id=?",
        dev
    )
    .fetch_optional(&mut *tx)
    .await
    .map_err(StorageError::from_sqlx)?
    .map(|row| {
        (
            row.status_boot_generation,
            row.status_sequence,
            row.status_lwt_message_id,
        )
    });
    let apply = match previous.as_ref() {
        None | Some((None, _, _)) => true,
        Some((Some(old_generation), old_sequence, old_lwt)) => {
            generation > *old_generation
                || (generation == *old_generation
                    && if is_lwt {
                        old_lwt.as_deref() != Some(id.as_str())
                    } else {
                        old_sequence.is_none_or(|old| sequence > old)
                    })
        }
    };
    if !apply {
        tx.rollback().await.map_err(StorageError::from_sqlx)?;
        return Ok(Dedup::Duplicate);
    }
    let prior =
        sqlx::query("SELECT status,boot_id,connectivity_mode FROM devices WHERE device_id=?")
            .bind(&dev)
            .fetch_optional(&mut *tx)
            .await
            .map_err(StorageError::from_sqlx)?;
    use sqlx::Row as _;
    let prior_status = prior.as_ref().map(|r| r.get::<String, _>("status"));
    let prior_boot = prior
        .as_ref()
        .and_then(|r| r.try_get::<Option<String>, _>("boot_id").ok().flatten());
    let prior_connectivity = prior
        .as_ref()
        .map(|r| r.get::<String, _>("connectivity_mode"));
    if prior.is_none() {
        sqlx::query("INSERT INTO devices(device_id,created_at) VALUES(?,?)")
            .bind(&dev)
            .bind(at)
            .execute(&mut *tx)
            .await
            .map_err(StorageError::from_sqlx)?;
        insert_edge_event(&mut tx, &dev, "device_registered", "info", None, at).await?;
    }
    let prior_sequence = previous.as_ref().and_then(|(old_generation, sequence, _)| {
        (*old_generation == Some(generation))
            .then_some(*sequence)
            .flatten()
    });
    let old_lwt = previous.and_then(|(old_generation, _, lwt)| {
        (old_generation == Some(generation))
            .then_some(lwt)
            .flatten()
    });
    let lwt_id = if is_lwt {
        Some(id.as_str())
    } else if old_lwt.is_some() {
        old_lwt.as_deref()
    } else {
        None
    };
    let status_json =
        serde_json::to_string(&e.data).map_err(|x| StorageError::Serialization(x.to_string()))?;
    let sensors_json = serde_json::to_string(&e.data.capabilities.sensors)
        .map_err(|x| StorageError::Serialization(x.to_string()))?;
    let status = json_name(&e.data.status)?;
    let connectivity = match e.data.connectivity.map(|c| c.mode) {
        Some(rhizo_mqtt_contract::payload::ConnectivityMode::Isolated) => "isolated",
        _ => "connected",
    };
    let boot = e.boot_id.as_ref().map(ToString::to_string);
    let seq = e.sequence.map(|v| v as i64);
    let last_seen = (e.data.status == DeviceStatusValue::Online).then_some(at);
    let firmware = e.data.firmware_version.as_deref();
    let protocol = e.data.protocol_version.map(i64::from);
    let applied = e.data.applied_config_version.map(i64::from);
    let uptime = e.data.uptime_ms.and_then(|v| i64::try_from(v).ok());
    let heap = e.data.free_heap_bytes.map(i64::from);
    let rssi = e.data.rssi_dbm.map(i64::from);
    let stored_sequence = if is_lwt {
        prior_sequence
    } else {
        Some(sequence)
    };
    sqlx::query("UPDATE devices SET status_json=?,status_boot_generation=?,status_sequence=?,status_lwt_message_id=?,status=?,firmware_version=?,protocol_version=?,boot_id=?,last_sequence=?,clock_synced=?,last_seen_at=COALESCE(?,last_seen_at),applied_config_version=?,uptime_ms=?,free_heap_bytes=?,rssi_dbm=?,sensors_json=?,connectivity_mode=? WHERE device_id=?")
    .bind(status_json).bind(generation).bind(stored_sequence).bind(lwt_id)
    .bind(&status).bind(firmware).bind(protocol).bind(&boot).bind(seq)
    .bind(e.clock_synced.unwrap_or(false)).bind(last_seen).bind(applied).bind(uptime)
    .bind(heap).bind(rssi).bind(sensors_json).bind(connectivity).bind(&dev)
    .execute(&mut *tx)
    .await
    .map_err(StorageError::from_sqlx)?;
    if prior_status.as_deref() != Some(status.as_str()) {
        let severity = if e.data.reason.as_deref() == Some("connection_lost") {
            "warning"
        } else {
            "info"
        };
        insert_edge_event(
            &mut tx,
            &dev,
            &status,
            severity,
            e.data.reason.as_deref(),
            at,
        )
        .await?;
    }
    if prior_boot.is_some() && prior_boot != boot {
        insert_edge_event(&mut tx, &dev, "device_restart", "info", None, at).await?;
    }
    replace_capabilities(&mut tx, &dev, &e.data.capabilities, at, prior_boot != boot).await?;
    if prior_connectivity.as_deref() != Some(connectivity) {
        let reported_isolated_ms = e.data.connectivity.map_or(0, |value| value.isolated_ms);
        apply_connectivity_transition(&mut tx, &dev, connectivity, at, reported_isolated_ms)
            .await?;
    }
    tx.commit().await.map_err(StorageError::from_sqlx)?;
    Ok(Dedup::New)
}

async fn insert_edge_event(
    tx: &mut Transaction<'_, Sqlite>,
    device: &str,
    kind: &str,
    severity: &str,
    detail: Option<&str>,
    at: i64,
) -> Result<(), StorageError> {
    let event_id = format!("edge:{device}:{kind}:{at}");
    let detail_json = detail.map(|value| serde_json::json!({"reason": value}).to_string());
    sqlx::query("INSERT OR IGNORE INTO device_events(event_id,device_id,kind,severity,detail_json,occurred_at,received_at,origin) VALUES(?,?,?,?,?,?,?,'edge')")
        .bind(event_id).bind(device).bind(kind).bind(severity).bind(detail_json).bind(at).bind(at)
        .execute(&mut **tx).await.map_err(StorageError::from_sqlx)?;
    Ok(())
}

async fn replace_capabilities(
    tx: &mut Transaction<'_, Sqlite>,
    device: &str,
    capabilities: &rhizo_mqtt_contract::payload::DeviceCapabilities,
    at: i64,
    reboot: bool,
) -> Result<(), StorageError> {
    let old: Vec<String> = sqlx::query_scalar("SELECT capability_id || ':' || class || ':' || kinds_json || ':' || COALESCE(point,'') FROM device_capabilities WHERE device_id=? ORDER BY capability_id,class")
        .bind(device).fetch_all(&mut **tx).await.map_err(StorageError::from_sqlx)?;
    sqlx::query("DELETE FROM device_capabilities WHERE device_id=?")
        .bind(device)
        .execute(&mut **tx)
        .await
        .map_err(StorageError::from_sqlx)?;
    for sensor in &capabilities.sensors {
        let kinds = serde_json::to_string(&sensor.kinds)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        sqlx::query("INSERT INTO device_capabilities(device_id,capability_id,class,kinds_json,point,limits_json,declared_at) VALUES(?,?,'sensor',?,?,NULL,?)")
            .bind(device).bind(sensor.sensor_id.as_str()).bind(kinds).bind(sensor.point.as_str()).bind(at)
            .execute(&mut **tx).await.map_err(StorageError::from_sqlx)?;
    }
    for actuator in &capabilities.actuators {
        let kind = json_name(&actuator.kind)?;
        sqlx::query("INSERT INTO device_capabilities(device_id,capability_id,class,kinds_json,point,limits_json,declared_at) VALUES(?,?,'actuator',?,NULL,NULL,?)")
            .bind(device).bind(actuator.actuator_id.as_str()).bind(serde_json::json!([kind]).to_string()).bind(at)
            .execute(&mut **tx).await.map_err(StorageError::from_sqlx)?;
    }
    let new: Vec<String> = sqlx::query_scalar("SELECT capability_id || ':' || class || ':' || kinds_json || ':' || COALESCE(point,'') FROM device_capabilities WHERE device_id=? ORDER BY capability_id,class")
        .bind(device).fetch_all(&mut **tx).await.map_err(StorageError::from_sqlx)?;
    if reboot && !old.is_empty() && old != new {
        insert_edge_event(tx, device, "capabilities_changed", "warning", None, at).await?;
    }
    Ok(())
}

async fn apply_connectivity_transition(
    tx: &mut Transaction<'_, Sqlite>,
    device: &str,
    mode: &str,
    at: i64,
    reported_isolated_ms: u64,
) -> Result<(), StorageError> {
    if mode == "isolated" {
        let reported = i64::try_from(reported_isolated_ms).unwrap_or(i64::MAX);
        let started_at = at.saturating_sub(reported);
        sqlx::query("INSERT INTO device_isolation_periods(device_id,started_at) VALUES(?,?)")
            .bind(device)
            .bind(started_at)
            .execute(&mut **tx)
            .await
            .map_err(StorageError::from_sqlx)?;
        sqlx::query("UPDATE devices SET isolation_started_at=? WHERE device_id=?")
            .bind(started_at)
            .bind(device)
            .execute(&mut **tx)
            .await
            .map_err(StorageError::from_sqlx)?;
        insert_edge_event(tx, device, "device.isolated", "warning", None, at).await?;
    } else {
        sqlx::query("UPDATE device_isolation_periods SET ended_at=?,duration_ms=?-started_at WHERE device_id=? AND ended_at IS NULL")
            .bind(at).bind(at).bind(device).execute(&mut **tx).await.map_err(StorageError::from_sqlx)?;
        sqlx::query("UPDATE devices SET isolation_started_at=NULL WHERE device_id=?")
            .bind(device)
            .execute(&mut **tx)
            .await
            .map_err(StorageError::from_sqlx)?;
        insert_edge_event(tx, device, "device.reconciled", "info", None, at).await?;
    }
    Ok(())
}
#[cfg(test)]
mod dedup {
    use super::*;
    #[tokio::test]
    async fn marker_is_durable_only_on_commit() {
        let db = EdgeDb::in_memory().await.unwrap();
        db.migrate().await.unwrap();
        let mut tx = db.begin().await.unwrap();
        assert_eq!(
            mark_processed(&mut tx, "id", "node-01", "x", 1)
                .await
                .unwrap(),
            Dedup::New
        );
        tx.rollback().await.unwrap();
        let mut retry = db.begin().await.unwrap();
        assert_eq!(
            mark_processed(&mut retry, "id", "node-01", "x", 1)
                .await
                .unwrap(),
            Dedup::New
        );
    }
}
