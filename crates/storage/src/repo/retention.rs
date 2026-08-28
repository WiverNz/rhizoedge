//! Deterministic batched retention. Ledger tables are intentionally absent.
#![allow(missing_docs)]
use crate::{EdgeDb, StorageError};
/// Per-table prune counts.
#[derive(Default, Debug, PartialEq, Eq)]
pub struct Pruned {
    pub processed: u64,
    pub outbox: u64,
    pub quarantine: u64,
    pub measurements: u64,
}
async fn del(db: &EdgeDb, sql: &'static str, cut: i64, batch: u32) -> Result<u64, StorageError> {
    Ok(sqlx::query(sql)
        .bind(cut)
        .bind(batch)
        .execute(db.pool())
        .await
        .map_err(StorageError::from_sqlx)?
        .rows_affected())
}
/// Applies one bounded batch using an injected authoritative timestamp.
pub async fn run_batch(db: &EdgeDb, now: i64, batch: u32) -> Result<Pruned, StorageError> {
    // Status is a mutable current projection with no independent effect row.
    // Its marker must survive retained/LWT redelivery for the lifetime of the
    // device. Other paths have stable identities on their durable effects.
    let processed=del(db,"DELETE FROM processed_messages WHERE message_id IN (SELECT message_id FROM processed_messages WHERE kind <> 'device.status' AND received_at < ? ORDER BY received_at LIMIT ?)",now-7*86_400_000,batch).await?;
    let outbox=del(db,"DELETE FROM pending_cloud_events WHERE event_id IN (SELECT event_id FROM pending_cloud_events WHERE status='synced' AND synced_at < ? ORDER BY synced_at LIMIT ?)",now-86_400_000,batch).await?;
    let measurements=del(db,"DELETE FROM measurements WHERE id IN (SELECT id FROM measurements WHERE received_at < ? ORDER BY received_at LIMIT ?)",now-90*86_400_000,batch).await?;
    let quarantine=sqlx::query("DELETE FROM quarantined_messages WHERE id IN (SELECT id FROM (SELECT id FROM quarantined_messages ORDER BY received_at DESC,id DESC LIMIT -1 OFFSET 1000) LIMIT ?)").bind(batch).execute(db.pool()).await.map_err(StorageError::from_sqlx)?.rows_affected();
    Ok(Pruned {
        processed,
        outbox,
        quarantine,
        measurements,
    })
}
