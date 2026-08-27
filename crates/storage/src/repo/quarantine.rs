//! Bounded quarantine persistence.
use crate::{EdgeDb, StorageError};

/// Stores at most the first KiB and evicts rows beyond the fixed 1000 cap.
pub async fn insert(
    db: &EdgeDb,
    device: Option<&str>,
    topic: &str,
    payload: &[u8],
    error: &str,
    at: i64,
) -> Result<(), StorageError> {
    let mut tx = db.begin().await?;
    let data = &payload[..payload.len().min(1024)];
    sqlx::query("INSERT INTO quarantined_messages(device_id,topic,payload,error,received_at) VALUES(?,?,?,?,?)").bind(device).bind(topic).bind(data).bind(error).bind(at).execute(&mut *tx).await.map_err(StorageError::from_sqlx)?;
    sqlx::query("DELETE FROM quarantined_messages WHERE id IN (SELECT id FROM quarantined_messages ORDER BY received_at DESC,id DESC LIMIT -1 OFFSET 1000)").execute(&mut *tx).await.map_err(StorageError::from_sqlx)?;
    tx.commit().await.map_err(StorageError::from_sqlx)
}
