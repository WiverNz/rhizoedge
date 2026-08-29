//! Read models restored from SQLite.
#![allow(missing_docs)]
use crate::{EdgeDb, StorageError};
use sqlx::FromRow;

/// Latest persisted sample; SQLite remains authoritative.
#[derive(Clone, Debug, FromRow)]
pub struct LatestSample {
    pub device_id: String,
    pub point: String,
    pub kind: String,
    pub value_num: Option<f64>,
    pub value_bool: Option<i64>,
    pub received_at: i64,
}
/// Loads the latest row for every device/point/kind tuple.
pub async fn latest_samples(db: &EdgeDb) -> Result<Vec<LatestSample>, StorageError> {
    sqlx::query_as!(
        LatestSample,
        r#"SELECT m.device_id AS "device_id!", m.point AS "point!", m.kind AS "kind!", m.value_num, m.value_bool, m.received_at AS "received_at!"
           FROM measurements m
           WHERE m.id = (SELECT m2.id FROM measurements m2
                         WHERE m2.device_id = m.device_id AND m2.point = m.point AND m2.kind = m.kind
                         ORDER BY m2.received_at DESC, m2.id DESC LIMIT 1)"#
    )
    .fetch_all(db.pool())
    .await
    .map_err(StorageError::from_sqlx)
}
/// The database's actual footprint on disk, in bytes.
///
/// This is the `storage_bytes` gauge [ADR-004](../../../../docs/adr/004-sqlite-edge-persistence-model.md)
/// and [failure-model.md](../../../../docs/architecture/failure-model.md) §3.6
/// name as the signal for a filling disk, so it has to be what the filesystem
/// would report, not what SQLite has logically allocated. In WAL mode those
/// differ by a lot: the write-ahead log can be larger than the main database
/// between checkpoints, and it is the sum that exhausts the volume. All three
/// files — `db`, `db-wal`, and `db-shm` — are therefore counted.
///
/// Bounded by construction: three `stat` calls, no table scan, no page walk.
/// An in-memory database has no files, and falls back to SQLite's own page
/// accounting so tests and the metric agree on a non-zero value.
pub async fn storage_bytes(db: &EdgeDb) -> Result<i64, StorageError> {
    if let Some(path) = db.path() {
        let mut total: i64 = 0;
        let mut found = false;
        for suffix in ["", "-wal", "-shm"] {
            let mut file = path.as_os_str().to_owned();
            file.push(suffix);
            // A missing WAL or SHM is normal, not an error: they exist only
            // while the database is open in WAL mode and has been written to.
            if let Ok(meta) = std::fs::metadata(std::path::Path::new(&file)) {
                total = total.saturating_add(i64::try_from(meta.len()).unwrap_or(i64::MAX));
                found = true;
            }
        }
        if found {
            return Ok(total);
        }
    }
    // Not `query_scalar!`: a PRAGMA is a statement, not a query over the
    // schema, so sqlx cannot describe it and the macro cannot check it. There
    // is nothing here for a schema change to invalidate — no table, no column.
    let pages: i64 = sqlx::query_scalar("PRAGMA page_count")
        .fetch_one(db.pool())
        .await
        .map_err(StorageError::from_sqlx)?;
    let page_size: i64 = sqlx::query_scalar("PRAGMA page_size")
        .fetch_one(db.pool())
        .await
        .map_err(StorageError::from_sqlx)?;
    Ok(pages.saturating_mul(page_size))
}

/// Number of persisted device rows restored on startup.
pub async fn device_count(db: &EdgeDb) -> Result<i64, StorageError> {
    sqlx::query_scalar!("SELECT count(*) FROM devices")
        .fetch_one(db.pool())
        .await
        .map_err(StorageError::from_sqlx)
}

/// One stored measurement, as the plant endpoints and the plant tick read it.
///
/// A narrow typed-kind row ([ADR-017](../../../../docs/adr/017-extensible-measurement-model.md)):
/// `value_num` and `value_bool` are mutually exclusive, and both are `NULL` for
/// a sample that failed validation on the way in. That third case is not a
/// value of zero — it is the absence of a reading, and every consumer has to
/// treat it that way (SAFETY-012).
#[derive(Clone, Debug, PartialEq)]
pub struct MeasurementRow {
    pub device_id: String,
    pub sensor_id: Option<String>,
    pub point: String,
    pub kind: String,
    pub value_num: Option<f64>,
    pub value_bool: Option<i64>,
    pub unit: String,
    pub quality: String,
    pub received_at: i64,
}

fn to_measurement(row: &sqlx::sqlite::SqliteRow) -> MeasurementRow {
    use sqlx::Row as _;
    MeasurementRow {
        device_id: row.get("device_id"),
        sensor_id: row.get("sensor_id"),
        point: row.get("point"),
        kind: row.get("kind"),
        value_num: row.get("value_num"),
        value_bool: row.get("value_bool"),
        unit: row.get("unit"),
        quality: row.get("quality"),
        received_at: row.get("received_at"),
    }
}

/// Measurements for one bound stream inside a window, oldest first.
///
/// Ordered ascending because every consumer — the trend fit, the dry-duration
/// accumulator, and the detection pair — reads time forwards. Sorting at the
/// call site instead would be one more place to get it wrong.
pub async fn measurements_for(
    db: &EdgeDb,
    device_id: &str,
    point: &str,
    kind: &str,
    from: i64,
    to: i64,
    limit: i64,
) -> Result<Vec<MeasurementRow>, StorageError> {
    let rows = sqlx::query(
        "SELECT device_id,sensor_id,point,kind,value_num,value_bool,unit,quality,received_at \
         FROM measurements WHERE device_id=? AND point=? AND kind=? AND received_at>=? AND received_at<=? \
         ORDER BY received_at, id LIMIT ?",
    )
    .bind(device_id)
    .bind(point)
    .bind(kind)
    .bind(from)
    .bind(to)
    .bind(limit.max(1))
    .fetch_all(db.pool())
    .await
    .map_err(StorageError::from_sqlx)?;
    Ok(rows.iter().map(to_measurement).collect())
}

/// How many rows a window holds, so a caller can refuse rather than truncate.
pub async fn count_measurements_for(
    db: &EdgeDb,
    device_id: &str,
    point: &str,
    kind: &str,
    from: i64,
    to: i64,
) -> Result<i64, StorageError> {
    sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM measurements WHERE device_id=? AND point=? AND kind=? \
         AND received_at>=? AND received_at<=?",
    )
    .bind(device_id)
    .bind(point)
    .bind(kind)
    .bind(from)
    .bind(to)
    .fetch_one(db.pool())
    .await
    .map_err(StorageError::from_sqlx)
}

/// The most recent row for one bound stream, whatever its validity.
pub async fn latest_measurement(
    db: &EdgeDb,
    device_id: &str,
    point: &str,
    kind: &str,
) -> Result<Option<MeasurementRow>, StorageError> {
    Ok(sqlx::query(
        "SELECT device_id,sensor_id,point,kind,value_num,value_bool,unit,quality,received_at \
         FROM measurements WHERE device_id=? AND point=? AND kind=? ORDER BY received_at DESC, id DESC LIMIT 1",
    )
    .bind(device_id)
    .bind(point)
    .bind(kind)
    .fetch_optional(db.pool())
    .await
    .map_err(StorageError::from_sqlx)?
    .as_ref()
    .map(to_measurement))
}

/// The last `limit` rows for one bound stream, oldest first.
pub async fn recent_measurements(
    db: &EdgeDb,
    device_id: &str,
    point: &str,
    kind: &str,
    limit: i64,
) -> Result<Vec<MeasurementRow>, StorageError> {
    let mut rows = sqlx::query(
        "SELECT device_id,sensor_id,point,kind,value_num,value_bool,unit,quality,received_at \
         FROM measurements WHERE device_id=? AND point=? AND kind=? ORDER BY received_at DESC, id DESC LIMIT ?",
    )
    .bind(device_id)
    .bind(point)
    .bind(kind)
    .bind(limit.max(1))
    .fetch_all(db.pool())
    .await
    .map_err(StorageError::from_sqlx)?
    .iter()
    .map(to_measurement)
    .collect::<Vec<_>>();
    rows.reverse();
    Ok(rows)
}

/// The telemetry cadence a device is configured for, which is the only input
/// SAFETY-005's control-freshness threshold may take.
pub async fn telemetry_interval_seconds(
    db: &EdgeDb,
    device_id: &str,
) -> Result<Option<i64>, StorageError> {
    sqlx::query_scalar::<_, i64>("SELECT telemetry_interval_seconds FROM devices WHERE device_id=?")
        .bind(device_id)
        .fetch_optional(db.pool())
        .await
        .map_err(StorageError::from_sqlx)
}

/// Whether a device declares the named sensor healthy and present.
///
/// `None` means the device has never said — which is not the same as healthy,
/// and every caller must treat it as an absence rather than as permission.
pub async fn sensor_healthy(
    db: &EdgeDb,
    device_id: &str,
    sensor_id: &str,
) -> Result<Option<bool>, StorageError> {
    let sensors: Option<String> =
        sqlx::query_scalar("SELECT sensors_json FROM devices WHERE device_id=?")
            .bind(device_id)
            .fetch_optional(db.pool())
            .await
            .map_err(StorageError::from_sqlx)?;
    let Some(sensors) = sensors else {
        return Ok(None);
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&sensors) else {
        return Ok(None);
    };
    Ok(value.as_array().and_then(|list| {
        list.iter()
            .find(|s| s.get("sensor_id").and_then(serde_json::Value::as_str) == Some(sensor_id))
            .map(|s| {
                s.get("healthy").and_then(serde_json::Value::as_bool) == Some(true)
                    && s.get("present").and_then(serde_json::Value::as_bool) == Some(true)
            })
    }))
}
