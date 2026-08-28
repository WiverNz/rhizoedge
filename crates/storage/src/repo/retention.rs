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
/// Applies one bounded batch using an injected authoritative timestamp.
///
/// Every statement is compile-time checked against the migrated schema
/// (M3-004), and every one is `LIMIT`ed so a single pass cannot stall the
/// writer behind an unbounded delete.
pub async fn run_batch(db: &EdgeDb, now: i64, batch: u32) -> Result<Pruned, StorageError> {
    // Every durable effect, including the status high-water projection, has an
    // independent stable identity. Transport markers can therefore stay
    // uniformly bounded without making a late replay a new logical effect.
    let limit = i64::from(batch);
    let markers_before = now - 7 * 86_400_000;
    let processed = sqlx::query!(
        "DELETE FROM processed_messages WHERE message_id IN (SELECT message_id FROM processed_messages WHERE received_at < ? ORDER BY received_at LIMIT ?)",
        markers_before,
        limit
    )
    .execute(db.pool())
    .await
    .map_err(StorageError::from_sqlx)?
    .rows_affected();

    let synced_before = now - 86_400_000;
    let outbox = sqlx::query!(
        "DELETE FROM pending_cloud_events WHERE event_id IN (SELECT event_id FROM pending_cloud_events WHERE status='synced' AND synced_at < ? ORDER BY synced_at LIMIT ?)",
        synced_before,
        limit
    )
    .execute(db.pool())
    .await
    .map_err(StorageError::from_sqlx)?
    .rows_affected();

    let samples_before = now - 90 * 86_400_000;
    let measurements = sqlx::query!(
        "DELETE FROM measurements WHERE id IN (SELECT id FROM measurements WHERE received_at < ? ORDER BY received_at LIMIT ?)",
        samples_before,
        limit
    )
    .execute(db.pool())
    .await
    .map_err(StorageError::from_sqlx)?
    .rows_affected();

    // Quarantine is capped by row count rather than by age: it is a diagnostic
    // buffer, and the newest 1000 entries are the ones an operator wants.
    let quarantine = sqlx::query!(
        "DELETE FROM quarantined_messages WHERE id IN (SELECT id FROM (SELECT id FROM quarantined_messages ORDER BY received_at DESC,id DESC LIMIT -1 OFFSET 1000) LIMIT ?)",
        limit
    )
    .execute(db.pool())
    .await
    .map_err(StorageError::from_sqlx)?
    .rows_affected();

    Ok(Pruned {
        processed,
        outbox,
        quarantine,
        measurements,
    })
}
