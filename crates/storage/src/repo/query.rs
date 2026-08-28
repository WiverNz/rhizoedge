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
