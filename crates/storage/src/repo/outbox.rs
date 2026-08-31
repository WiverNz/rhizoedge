//! Durable optional-cloud outbox operations.
#![allow(missing_docs)]
use crate::{EdgeDb, StorageError};
use sqlx::{Row, Sqlite, Transaction};

/// The complete immutable V1 cloud event catalogue from ADR-005.
pub const EVENT_KINDS: &[&str] = &[
    "measurement.sample",
    "device.status",
    "device.event",
    "device.config_applied",
    "device.capabilities",
    "device.policy_applied",
    "device.isolated",
    "device.reconciled",
    "history.gap",
    "plant.created",
    "plant.updated",
    "plant.state_changed",
    "plant.binding_changed",
    "plant.policy_changed",
    "watering.started",
    "watering.completed",
    "watering.detected",
    "watering.offline_autonomous",
    "command.issued",
    "command.settled",
    "lockout.set",
    "lockout.cleared",
    "threshold.warning",
    "threshold.critical",
    "fertilisation.applied",
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ValueTier {
    Low,
    High,
}
impl ValueTier {
    fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::High => "high",
        }
    }
}
pub fn tier_for(kind: &str) -> ValueTier {
    if kind == "measurement.sample" {
        ValueTier::Low
    } else {
        ValueTier::High
    }
}

/// The only SQL write site for new outbox rows. The caller's mutation and this
/// insert share the supplied transaction; disabled cloud returns no id/write.
pub async fn emit(
    tx: &mut Transaction<'_, Sqlite>,
    kind: &str,
    payload: &serde_json::Value,
    at: i64,
) -> Result<Option<String>, StorageError> {
    let event_id = uuid::Uuid::now_v7().to_string();
    let payload =
        serde_json::to_string(payload).map_err(|e| StorageError::Serialization(e.to_string()))?;
    let result=sqlx::query("INSERT INTO pending_cloud_events(event_id,kind,value_tier,payload_json,status,next_attempt_at,created_at) SELECT ?,?,?,?,'pending',?,? FROM cloud_sync_settings WHERE singleton=1 AND enabled=1")
        .bind(&event_id).bind(kind).bind(tier_for(kind).as_str()).bind(payload).bind(at).bind(at).execute(&mut **tx).await.map_err(StorageError::from_sqlx)?;
    Ok((result.rows_affected() == 1).then_some(event_id))
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct RowData {
    pub event_id: String,
    pub kind: String,
    pub payload_json: String,
    pub attempts: i64,
    pub created_at: i64,
}
pub async fn configure(db: &EdgeDb, enabled: bool, max_rows: u64) -> Result<(), StorageError> {
    sqlx::query("UPDATE cloud_sync_settings SET enabled=?,outbox_max_rows=? WHERE singleton=1")
        .bind(i64::from(enabled))
        .bind(max_rows as i64)
        .execute(db.pool())
        .await
        .map_err(StorageError::from_sqlx)?;
    Ok(())
}
pub async fn ready(db: &EdgeDb, now: i64, limit: u32) -> Result<Vec<RowData>, StorageError> {
    let rows=sqlx::query("SELECT event_id,kind,payload_json,attempts,created_at FROM pending_cloud_events WHERE status='pending' AND next_attempt_at<=? ORDER BY created_at,event_id LIMIT ?").bind(now).bind(i64::from(limit)).fetch_all(db.pool()).await.map_err(StorageError::from_sqlx)?;
    Ok(rows
        .iter()
        .map(|r| RowData {
            event_id: r.get("event_id"),
            kind: r.get("kind"),
            payload_json: r.get("payload_json"),
            attempts: r.get("attempts"),
            created_at: r.get("created_at"),
        })
        .collect())
}
pub async fn synced(db: &EdgeDb, id: &str, now: i64) -> Result<(), StorageError> {
    sqlx::query("UPDATE pending_cloud_events SET status='synced',synced_at=?,last_error=NULL WHERE event_id=?").bind(now).bind(id).execute(db.pool()).await.map_err(StorageError::from_sqlx)?;
    Ok(())
}
pub async fn quarantine(db: &EdgeDb, id: &str, error: &str) -> Result<(), StorageError> {
    sqlx::query(
        "UPDATE pending_cloud_events SET status='quarantined',last_error=? WHERE event_id=?",
    )
    .bind(error)
    .bind(id)
    .execute(db.pool())
    .await
    .map_err(StorageError::from_sqlx)?;
    Ok(())
}
pub async fn retry(
    db: &EdgeDb,
    ids: &[String],
    next: i64,
    error: &str,
) -> Result<(), StorageError> {
    let mut tx = db.begin().await?;
    for id in ids {
        sqlx::query("UPDATE pending_cloud_events SET attempts=attempts+1,next_attempt_at=?,last_error=? WHERE event_id=? AND status='pending'").bind(next).bind(error).bind(id).execute(&mut *tx).await.map_err(StorageError::from_sqlx)?;
    }
    tx.commit().await.map_err(StorageError::from_sqlx)
}
pub async fn counts(db: &EdgeDb) -> Result<(i64, i64), StorageError> {
    let pending =
        sqlx::query_scalar("SELECT count(*) FROM pending_cloud_events WHERE status='pending'")
            .fetch_one(db.pool())
            .await
            .map_err(StorageError::from_sqlx)?;
    let quarantined =
        sqlx::query_scalar("SELECT count(*) FROM pending_cloud_events WHERE status='quarantined'")
            .fetch_one(db.pool())
            .await
            .map_err(StorageError::from_sqlx)?;
    Ok((pending, quarantined))
}
pub async fn settings(db: &EdgeDb) -> Result<(bool, i64), StorageError> {
    let (enabled, max): (i64, i64) =
        sqlx::query_as("SELECT enabled,outbox_max_rows FROM cloud_sync_settings WHERE singleton=1")
            .fetch_one(db.pool())
            .await
            .map_err(StorageError::from_sqlx)?;
    Ok((enabled != 0, max))
}
pub async fn quarantined(db: &EdgeDb, limit: u32) -> Result<Vec<RowData>, StorageError> {
    let rows=sqlx::query("SELECT event_id,kind,payload_json,attempts,created_at FROM pending_cloud_events WHERE status='quarantined' ORDER BY created_at LIMIT ?").bind(i64::from(limit.min(1000))).fetch_all(db.pool()).await.map_err(StorageError::from_sqlx)?;
    Ok(rows
        .iter()
        .map(|r| RowData {
            event_id: r.get("event_id"),
            kind: r.get("kind"),
            payload_json: r.get("payload_json"),
            attempts: r.get("attempts"),
            created_at: r.get("created_at"),
        })
        .collect())
}
pub async fn prune_low(db: &EdgeDb) -> Result<u64, StorageError> {
    let max: i64 =
        sqlx::query_scalar("SELECT outbox_max_rows FROM cloud_sync_settings WHERE singleton=1")
            .fetch_one(db.pool())
            .await
            .map_err(StorageError::from_sqlx)?;
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM pending_cloud_events WHERE status!='synced'")
            .fetch_one(db.pool())
            .await
            .map_err(StorageError::from_sqlx)?;
    let excess = count.saturating_sub(max);
    if excess == 0 {
        return Ok(0);
    }
    let result=sqlx::query("DELETE FROM pending_cloud_events WHERE event_id IN(SELECT event_id FROM pending_cloud_events WHERE value_tier='low' AND status='pending' ORDER BY created_at,event_id LIMIT ?)").bind(excess).execute(db.pool()).await.map_err(StorageError::from_sqlx)?;
    Ok(result.rows_affected())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn prune_never_removes_high_tier_and_is_oldest_first() {
        let db = EdgeDb::in_memory().await.unwrap();
        db.migrate().await.unwrap();
        configure(&db, true, 3).await.unwrap();
        for (i, tier) in [(1, "low"), (2, "high"), (3, "low"), (4, "high"), (5, "low")] {
            sqlx::query(
                "INSERT INTO pending_cloud_events VALUES(?,?,?,'{}','pending',0,?,NULL,?,NULL)",
            )
            .bind(format!("{i:032x}"))
            .bind("x")
            .bind(tier)
            .bind(i)
            .bind(i)
            .execute(db.pool())
            .await
            .unwrap();
        }
        assert_eq!(prune_low(&db).await.unwrap(), 2);
        let ids: Vec<String> =
            sqlx::query_scalar("SELECT event_id FROM pending_cloud_events ORDER BY created_at")
                .fetch_all(db.pool())
                .await
                .unwrap();
        assert_eq!(
            ids,
            vec![
                format!("{:032x}", 2),
                format!("{:032x}", 4),
                format!("{:032x}", 5)
            ]
        );
    }
    #[test]
    fn complete_catalogue_has_safe_tiers() {
        assert_eq!(EVENT_KINDS.len(), 25);
        for kind in EVENT_KINDS {
            assert_eq!(
                tier_for(kind),
                if *kind == "measurement.sample" {
                    ValueTier::Low
                } else {
                    ValueTier::High
                }
            );
        }
    }
    #[tokio::test]
    async fn canonical_writer_emits_the_complete_catalogue_and_disabled_is_a_noop() {
        let db = EdgeDb::in_memory().await.unwrap();
        db.migrate().await.unwrap();
        configure(&db, true, 500_000).await.unwrap();
        let mut tx = db.begin().await.unwrap();
        for kind in EVENT_KINDS {
            assert!(
                emit(&mut tx, kind, &serde_json::json!({"kind":kind}), 1_000)
                    .await
                    .unwrap()
                    .is_some()
            );
        }
        tx.commit().await.unwrap();
        let rows: Vec<(String, String)> =
            sqlx::query_as("SELECT kind,value_tier FROM pending_cloud_events ORDER BY kind")
                .fetch_all(db.pool())
                .await
                .unwrap();
        assert_eq!(rows.len(), EVENT_KINDS.len());
        for (kind, tier) in rows {
            assert_eq!(
                tier,
                if kind == "measurement.sample" {
                    "low"
                } else {
                    "high"
                }
            );
        }
        configure(&db, false, 500_000).await.unwrap();
        let mut tx = db.begin().await.unwrap();
        assert!(
            emit(&mut tx, "lockout.set", &serde_json::json!({}), 2_000)
                .await
                .unwrap()
                .is_none()
        );
        tx.commit().await.unwrap();
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM pending_cloud_events")
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert_eq!(count, EVENT_KINDS.len() as i64);
    }
}
