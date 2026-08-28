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
    sqlx::query_as("SELECT m.device_id,m.point,m.kind,m.value_num,m.value_bool,m.received_at FROM measurements m WHERE m.id=(SELECT m2.id FROM measurements m2 WHERE m2.device_id=m.device_id AND m2.point=m.point AND m2.kind=m.kind ORDER BY m2.received_at DESC,m2.id DESC LIMIT 1)").fetch_all(db.pool()).await.map_err(StorageError::from_sqlx)
}
/// Size of the main database in bytes, sampled from SQLite's own page counters.
///
/// This is the `storage_bytes` gauge ADR-004 and the failure model rely on to
/// see a disk filling before `SQLITE_FULL` arrives. It reports the main
/// database only, so a large uncheckpointed WAL is not counted.
pub async fn storage_bytes(db: &EdgeDb) -> Result<i64, StorageError> {
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
