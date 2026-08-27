//! Transaction-only ingestion writes.
#![allow(missing_docs)]

use rhizo_mqtt_contract::payload::{
    ActuatorState, DeviceEventBatch, EventDetail, EventKind, EventTier, MeasurementSample,
    MeasurementValue,
};
use rhizo_mqtt_contract::{Envelope, MessageKind};
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
    let n=sqlx::query("INSERT INTO processed_messages(message_id,device_id,kind,received_at) VALUES(?,?,?,?) ON CONFLICT(message_id) DO NOTHING")
        .bind(message_id).bind(device_id).bind(kind).bind(received_at).execute(&mut **tx).await.map_err(StorageError::from_sqlx)?.rows_affected();
    Ok(if n == 0 { Dedup::Duplicate } else { Dedup::New })
}

async fn touch_device(
    tx: &mut Transaction<'_, Sqlite>,
    device: &str,
    boot: Option<String>,
    seq: Option<u64>,
    received: i64,
) -> Result<(), StorageError> {
    let previous: Option<(Option<String>, Option<i64>)> =
        sqlx::query_as("SELECT boot_id,last_sequence FROM devices WHERE device_id=?")
            .bind(device)
            .fetch_optional(&mut **tx)
            .await
            .map_err(StorageError::from_sqlx)?;
    if let Some((old_boot, old_seq)) = previous {
        if boot != old_boot {
            let event_id = format!("boot:{device}:{}", boot.as_deref().unwrap_or("unknown"));
            sqlx::query("INSERT INTO device_events(event_id,device_id,kind,severity,occurred_at,received_at) VALUES(?,?,'boot','info',?,?) ON CONFLICT(event_id) DO NOTHING")
                .bind(event_id).bind(device).bind(received).bind(received).execute(&mut **tx).await.map_err(StorageError::from_sqlx)?;
        } else if let (Some(old), Some(new)) = (old_seq, seq)
            && (new as i64) < old
        {
            let event_id = format!(
                "sequence-regression:{device}:{}:{new}",
                boot.as_deref().unwrap_or("unknown")
            );
            sqlx::query("INSERT INTO device_events(event_id,device_id,kind,severity,occurred_at,received_at) VALUES(?,?,'sequence_regression','warning',?,?) ON CONFLICT(event_id) DO NOTHING")
                    .bind(event_id).bind(device).bind(received).bind(received).execute(&mut **tx).await.map_err(StorageError::from_sqlx)?;
        }
    }
    sqlx::query("INSERT INTO devices(device_id,boot_id,last_sequence,last_seen_at,created_at) VALUES(?,?,?,?,?) ON CONFLICT(device_id) DO UPDATE SET boot_id=excluded.boot_id,last_sequence=excluded.last_sequence,last_seen_at=excluded.last_seen_at")
        .bind(device).bind(boot).bind(seq.map(|v|v as i64)).bind(received).bind(received).execute(&mut **tx).await.map_err(StorageError::from_sqlx)?;
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
    touch_device(
        &mut tx,
        &device,
        envelope.boot_id.as_ref().map(ToString::to_string),
        envelope.sequence,
        received_at,
    )
    .await?;
    let batch_id = envelope.data.batch_id.to_string();
    let mut accepted = 0;
    for sample in &envelope.data.samples {
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
    sqlx::query("INSERT INTO measurements(device_id,sensor_id,point,kind,value_num,value_bool,unit,quality,calibration_ref,received_at,device_time_ms,boot_id,sequence,batch_id,origin) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)")
        .bind(device).bind(s.sensor_id.as_ref().map(|v|v.as_str())).bind(s.point.as_str()).bind(json_name(&s.kind)?).bind(num).bind(bval).bind(json_name(&s.unit)?).bind(json_name(&s.quality)?).bind(s.calibration_ref.as_ref().map(|v|v.as_str())).bind(received).bind(device_time).bind(boot).bind(sequence.map(|v|v as i64)).bind(batch).bind(origin).execute(&mut **tx).await.map_err(StorageError::from_sqlx)?;
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
    sqlx::query("INSERT INTO device_events(event_id,device_id,kind,severity,detail_json,occurred_at,received_at) VALUES(?,?,'sensor_invalid','warning',?,?,?)").bind(id).bind(device).bind(detail).bind(at).bind(at).execute(&mut **tx).await.map_err(StorageError::from_sqlx)?;
    Ok(())
}
async fn enqueue(
    tx: &mut Transaction<'_, Sqlite>,
    id: &str,
    kind: &str,
    at: i64,
) -> Result<(), StorageError> {
    sqlx::query("INSERT INTO pending_cloud_events(event_id,kind,value_tier,payload_json,status,next_attempt_at,created_at) VALUES(?,?,'low','{}','pending',?,?) ON CONFLICT(event_id) DO NOTHING").bind(id).bind(kind).bind(at).bind(at).execute(&mut **tx).await.map_err(StorageError::from_sqlx)?;
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
    touch_device(
        &mut tx,
        &dev,
        e.boot_id.as_ref().map(ToString::to_string),
        e.sequence,
        at,
    )
    .await?;
    sqlx::query("INSERT INTO actuator_states(message_id,device_id,actuator_id,kind,state_json,received_at,device_time_ms,boot_id,sequence) VALUES(?,?,?,?,?,?,?,?,?)").bind(&id).bind(&dev).bind(e.data.actuator_id.as_str()).bind(json_name(&e.data.kind)?).bind(serde_json::to_string(&e.data).map_err(|x|StorageError::Serialization(x.to_string()))?).bind(at).bind(e.device_time_ms.map(|v|v.0)).bind(e.boot_id.as_ref().map(ToString::to_string)).bind(e.sequence.map(|v|v as i64)).execute(&mut *tx).await.map_err(StorageError::from_sqlx)?;
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
    touch_device(
        &mut tx,
        &dev,
        e.boot_id.as_ref().map(ToString::to_string),
        e.sequence,
        at,
    )
    .await?;
    sqlx::query("INSERT INTO command_results(message_id,command_id,device_id,result_json,received_at,device_time_ms,boot_id,sequence) VALUES(?,?,?,?,?,?,?,?)").bind(&id).bind(e.data.command_id.to_string()).bind(&dev).bind(serde_json::to_string(&e.data).map_err(|x|StorageError::Serialization(x.to_string()))?).bind(at).bind(e.device_time_ms.map(|v|v.0)).bind(e.boot_id.as_ref().map(ToString::to_string)).bind(e.sequence.map(|v|v as i64)).execute(&mut *tx).await.map_err(StorageError::from_sqlx)?;
    tx.commit().await.map_err(StorageError::from_sqlx)?;
    Ok(Dedup::New)
}

/// Replay commit result used to publish an acknowledgement only after commit.
pub struct ReplayCommit {
    pub dedup: Dedup,
    pub through_device_seq: u64,
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
    for event in &e.data.events {
        let eid = event.event_id.to_string();
        let kind = json_name(&event.kind)?;
        let detail = serde_json::to_string(&event.detail)
            .map_err(|x| StorageError::Serialization(x.to_string()))?;
        sqlx::query("INSERT INTO device_events(event_id,device_id,kind,severity,detail_json,occurred_at,received_at,boot_id,device_seq,origin) VALUES(?,?,?,?,?,?,?,?,?,'offline_replay') ON CONFLICT(event_id) DO NOTHING").bind(&eid).bind(&dev).bind(&kind).bind(if event.tier==EventTier::Audit{"warning"}else{"info"}).bind(&detail).bind(event.device_time_ms.map_or(at,|v|v.0)).bind(at).bind(&boot).bind(event.device_seq as i64).execute(&mut *tx).await.map_err(StorageError::from_sqlx)?;
        if let EventDetail::Gap {
            from_seq,
            to_seq,
            lost_count,
            lost_tier,
        } = event.detail
        {
            sqlx::query("INSERT INTO history_gaps(gap_id,device_id,boot_id,from_seq,to_seq,lost_count,tier,reported_at) VALUES(?,?,?,?,?,?,?,?) ON CONFLICT(gap_id) DO NOTHING").bind(&eid).bind(&dev).bind(&boot).bind(from_seq as i64).bind(to_seq as i64).bind(lost_count as i64).bind(json_name(&lost_tier)?).bind(at).execute(&mut *tx).await.map_err(StorageError::from_sqlx)?;
        }
        if let (EventKind::WateringOfflineAutonomous, EventDetail::Watering { delivered_ml, .. }) =
            (&event.kind, &event.detail)
        {
            sqlx::query("INSERT INTO watering_events(watering_event_id,device_id,mode,origin,started_at,completed_at,delivered_ml,status) VALUES(?,?,'automatic','offline_autonomous',?,?,?,'completed') ON CONFLICT(watering_event_id) DO NOTHING").bind(&eid).bind(&dev).bind(event.device_time_ms.map_or(at,|v|v.0)).bind(event.device_time_ms.map_or(at,|v|v.0)).bind(*delivered_ml as f64).execute(&mut *tx).await.map_err(StorageError::from_sqlx)?;
        }
    }
    let through = contiguous_in_tx(&mut tx, &dev, &boot).await?;
    sqlx::query("INSERT INTO replay_progress(device_id,boot_id,through_device_seq,complete,updated_at) VALUES(?,?,?,?,?) ON CONFLICT(device_id,boot_id) DO UPDATE SET through_device_seq=max(through_device_seq,excluded.through_device_seq),complete=max(complete,excluded.complete),updated_at=excluded.updated_at").bind(&dev).bind(&boot).bind(through as i64).bind(i64::from(e.data.complete)).bind(at).execute(&mut *tx).await.map_err(StorageError::from_sqlx)?;
    tx.commit().await.map_err(StorageError::from_sqlx)?;
    Ok(ReplayCommit {
        dedup: Dedup::New,
        through_device_seq: through,
        complete: e.data.complete,
    })
}
async fn contiguous_in_tx(
    tx: &mut Transaction<'_, Sqlite>,
    dev: &str,
    boot: &str,
) -> Result<u64, StorageError> {
    let seqs:Vec<i64>=sqlx::query_scalar("SELECT device_seq FROM device_events WHERE device_id=? AND boot_id=? AND device_seq IS NOT NULL ORDER BY device_seq").bind(dev).bind(boot).fetch_all(&mut **tx).await.map_err(StorageError::from_sqlx)?;
    let prior: Option<i64> = sqlx::query_scalar(
        "SELECT through_device_seq FROM replay_progress WHERE device_id=? AND boot_id=?",
    )
    .bind(dev)
    .bind(boot)
    .fetch_optional(&mut **tx)
    .await
    .map_err(StorageError::from_sqlx)?
    .flatten();
    let mut expected = prior.map_or(0, |v| v as u64 + 1);
    let mut through = prior.unwrap_or(-1);
    for s in seqs {
        if s as u64 == expected {
            through = s;
            expected += 1;
        } else if s as u64 > expected {
            break;
        }
    }
    Ok(through.max(0) as u64)
}
async fn replay_through(db: &EdgeDb, dev: &str, boot: &str) -> Result<u64, StorageError> {
    Ok(sqlx::query_scalar::<_, Option<i64>>(
        "SELECT through_device_seq FROM replay_progress WHERE device_id=? AND boot_id=?",
    )
    .bind(dev)
    .bind(boot)
    .fetch_one(db.pool())
    .await
    .map_err(StorageError::from_sqlx)?
    .unwrap_or(0) as u64)
}

/// Persists status as raw prerequisite state without M4 health behaviour.
pub async fn persist_raw<T: serde::Serialize>(
    db: &EdgeDb,
    e: &Envelope<T>,
    kind: MessageKind,
    at: i64,
) -> Result<Dedup, StorageError> {
    let mut tx = db.begin().await?;
    let id = e.message_id.to_string();
    let dev = e.device_id.to_string();
    let k = json_name(&kind)?;
    if mark_processed(&mut tx, &id, &dev, &k, at).await? == Dedup::Duplicate {
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
    sqlx::query("UPDATE devices SET status_json=? WHERE device_id=?")
        .bind(
            serde_json::to_string(&e.data)
                .map_err(|x| StorageError::Serialization(x.to_string()))?,
        )
        .bind(&dev)
        .execute(&mut *tx)
        .await
        .map_err(StorageError::from_sqlx)?;
    tx.commit().await.map_err(StorageError::from_sqlx)?;
    Ok(Dedup::New)
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
