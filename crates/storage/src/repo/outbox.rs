//! Durable optional-cloud outbox operations.
#![allow(missing_docs)]
use crate::{EdgeDb, StorageError};
use sqlx::{Row, Sqlite, Transaction};

/// One kind from the ADR-005 V1 cloud event catalogue.
///
/// A newtype over a `&'static str` rather than a bare string, because the whole
/// catalogue is a set of exact names that two independent programs have to agree
/// on. `emit` takes this type and nothing else, so a call site cannot name a
/// kind the catalogue does not contain: the `device.capabilities_changed` the
/// edge used to emit for ADR-005's `device.capabilities` would not compile
/// through this type, which is the point of introducing it.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct EventKind(&'static str);

impl EventKind {
    pub const MEASUREMENT_SAMPLE: Self = Self("measurement.sample");
    pub const DEVICE_STATUS: Self = Self("device.status");
    pub const DEVICE_EVENT: Self = Self("device.event");
    pub const DEVICE_CONFIG_APPLIED: Self = Self("device.config_applied");
    pub const DEVICE_CAPABILITIES: Self = Self("device.capabilities");
    pub const DEVICE_POLICY_APPLIED: Self = Self("device.policy_applied");
    pub const DEVICE_ISOLATED: Self = Self("device.isolated");
    pub const DEVICE_RECONCILED: Self = Self("device.reconciled");
    pub const HISTORY_GAP: Self = Self("history.gap");
    pub const PLANT_CREATED: Self = Self("plant.created");
    pub const PLANT_UPDATED: Self = Self("plant.updated");
    pub const PLANT_STATE_CHANGED: Self = Self("plant.state_changed");
    pub const PLANT_BINDING_CHANGED: Self = Self("plant.binding_changed");
    pub const PLANT_POLICY_CHANGED: Self = Self("plant.policy_changed");
    pub const WATERING_STARTED: Self = Self("watering.started");
    pub const WATERING_COMPLETED: Self = Self("watering.completed");
    pub const WATERING_DETECTED: Self = Self("watering.detected");
    pub const WATERING_OFFLINE_AUTONOMOUS: Self = Self("watering.offline_autonomous");
    pub const COMMAND_ISSUED: Self = Self("command.issued");
    pub const COMMAND_SETTLED: Self = Self("command.settled");
    pub const LOCKOUT_SET: Self = Self("lockout.set");
    pub const LOCKOUT_CLEARED: Self = Self("lockout.cleared");
    pub const THRESHOLD_WARNING: Self = Self("threshold.warning");
    pub const THRESHOLD_CRITICAL: Self = Self("threshold.critical");
    pub const FERTILISATION_APPLIED: Self = Self("fertilisation.applied");

    /// The complete immutable V1 catalogue, in ADR-005's order.
    ///
    /// `catalogue_matches_adr_005` reads the ADR itself and asserts this list is
    /// exactly it, so the document and the code cannot drift apart silently.
    pub const ALL: &'static [Self] = &[
        Self::MEASUREMENT_SAMPLE,
        Self::DEVICE_STATUS,
        Self::DEVICE_EVENT,
        Self::DEVICE_CONFIG_APPLIED,
        Self::DEVICE_CAPABILITIES,
        Self::DEVICE_POLICY_APPLIED,
        Self::DEVICE_ISOLATED,
        Self::DEVICE_RECONCILED,
        Self::HISTORY_GAP,
        Self::PLANT_CREATED,
        Self::PLANT_UPDATED,
        Self::PLANT_STATE_CHANGED,
        Self::PLANT_BINDING_CHANGED,
        Self::PLANT_POLICY_CHANGED,
        Self::WATERING_STARTED,
        Self::WATERING_COMPLETED,
        Self::WATERING_DETECTED,
        Self::WATERING_OFFLINE_AUTONOMOUS,
        Self::COMMAND_ISSUED,
        Self::COMMAND_SETTLED,
        Self::LOCKOUT_SET,
        Self::LOCKOUT_CLEARED,
        Self::THRESHOLD_WARNING,
        Self::THRESHOLD_CRITICAL,
        Self::FERTILISATION_APPLIED,
    ];

    /// Catalogue kinds the V1 edge deliberately never emits.
    ///
    /// The catalogue is the *vocabulary* the cloud accepts, and it was written
    /// ahead of the features that fill it. Fertilisation has no edge feature at
    /// all in v1 — no table, no route, no ingest path — so there is nothing
    /// truthful to emit, and inventing an emitter would be worse than saying so.
    /// The cloud still recognises and projects the kind, so a later milestone
    /// adds an emitter and nothing else.
    ///
    /// `every_catalogue_kind_has_an_edge_emitter` treats this as the exhaustive
    /// exception list: a kind that quietly loses its emitter fails that test.
    pub const WITHOUT_EDGE_EMITTER: &'static [Self] = &[Self::FERTILISATION_APPLIED];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        self.0
    }
}

impl std::fmt::Display for EventKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

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
/// The pruning tier for a kind.
///
/// Only `measurement.sample` is low tier and therefore prunable. Everything else
/// is high tier by *safe default*, which is why this is written as a single
/// negative match rather than a per-kind table: a kind added to the catalogue
/// and forgotten here becomes unprunable history, never prunable ledger data.
#[must_use]
pub fn tier_for(kind: EventKind) -> ValueTier {
    if kind == EventKind::MEASUREMENT_SAMPLE {
        ValueTier::Low
    } else {
        ValueTier::High
    }
}

/// The only SQL write site for new outbox rows. The caller's mutation and this
/// insert share the supplied transaction; disabled cloud returns no id/write.
pub async fn emit(
    tx: &mut Transaction<'_, Sqlite>,
    kind: EventKind,
    payload: &serde_json::Value,
    at: i64,
) -> Result<Option<String>, StorageError> {
    let event_id = uuid::Uuid::now_v7().to_string();
    let payload =
        serde_json::to_string(payload).map_err(|e| StorageError::Serialization(e.to_string()))?;
    let result=sqlx::query("INSERT INTO pending_cloud_events(event_id,kind,value_tier,payload_json,status,next_attempt_at,created_at) SELECT ?,?,?,?,'pending',?,? FROM cloud_sync_settings WHERE singleton=1 AND enabled=1")
        .bind(&event_id).bind(kind.as_str()).bind(tier_for(kind).as_str()).bind(payload).bind(at).bind(at).execute(&mut **tx).await.map_err(StorageError::from_sqlx)?;
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
        assert_eq!(EventKind::ALL.len(), 25);
        for kind in EventKind::ALL {
            assert_eq!(
                tier_for(*kind),
                if *kind == EventKind::MEASUREMENT_SAMPLE {
                    ValueTier::Low
                } else {
                    ValueTier::High
                }
            );
        }
    }

    /// ADR-005 is canonical, so the code reads it rather than restating it.
    ///
    /// This is the test that makes a name like `device.capabilities_changed`
    /// impossible to keep: the ADR says `device.capabilities`, and any constant
    /// that does not appear there — or any ADR kind with no constant — fails
    /// here rather than surviving to a live deployment.
    #[test]
    fn catalogue_matches_adr_005() {
        let adr = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../docs/adr/005-cloud-event-model-and-idempotency.md"),
        )
        .expect("ADR-005 is readable from the storage crate");
        let block = adr
            .split("Event kinds in V1:")
            .nth(1)
            .and_then(|rest| rest.split("```").nth(1))
            .expect("ADR-005 still carries a fenced V1 event-kind block");
        let documented: Vec<&str> = block
            .split_whitespace()
            .filter(|token| {
                token.contains('.')
                    && token
                        .bytes()
                        .all(|c| c.is_ascii_lowercase() || c == b'.' || c == b'_')
            })
            .collect();
        let implemented: Vec<&str> = EventKind::ALL.iter().map(|k| k.as_str()).collect();
        assert_eq!(documented, implemented);
    }

    /// Every catalogue kind is reachable from an edge emitter, or is listed as
    /// deliberately unemitted. Nothing may be silently neither.
    ///
    /// A source scan rather than a runtime probe: emission is spread across
    /// ingest, plant, binding, command, and reconciliation paths that no single
    /// unit test can drive, but every one of them has to name its constant.
    /// The constant has to be spelled `EventKind::NAME` at the call site — an
    /// aliased import would hide it from this scan, which is a small price for a
    /// check that needs no database, no broker, and no cloud to run.
    #[test]
    fn every_catalogue_kind_has_an_edge_emitter() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut sources = String::new();
        for crate_dir in ["crates/storage/src", "crates/edge-controller/src"] {
            collect_rust_sources(&root.join(crate_dir), &mut sources);
        }
        // The declaration site itself must not count as a use.
        let declarations = std::fs::read_to_string(root.join("crates/storage/src/repo/outbox.rs"))
            .expect("this file is readable");
        assert!(!sources.is_empty(), "found no edge sources to scan");
        for kind in EventKind::ALL {
            let constant = constant_name(*kind);
            let reference = format!("EventKind::{constant}");
            let uses =
                sources.matches(&reference).count() - declarations.matches(&reference).count();
            let exempt = EventKind::WITHOUT_EDGE_EMITTER.contains(kind);
            assert_eq!(
                uses > 0,
                !exempt,
                "{kind} has {uses} emitter references but exempt={exempt}"
            );
        }
    }

    /// The catalogue can only be bypassed by writing the table directly, so
    /// nothing else is allowed to.
    ///
    /// `emit` takes `EventKind`, which makes an undocumented kind a compile
    /// error — but only for callers that go through `emit`. A hand-rolled
    /// `INSERT INTO pending_cloud_events` elsewhere would put any string it
    /// liked in the `kind` column and reach the cloud as an unknown kind. This
    /// asserts the single-writer property the module doc claims.
    #[test]
    fn emit_is_the_only_production_writer_of_the_outbox_table() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut offenders = Vec::new();
        for crate_dir in ["crates/storage/src", "crates/edge-controller/src"] {
            visit_rust_sources(&root.join(crate_dir), &mut |path, body| {
                if path.ends_with("outbox.rs") {
                    return;
                }
                if body.contains("INSERT INTO pending_cloud_events") {
                    offenders.push(path.display().to_string());
                }
            });
        }
        assert!(
            offenders.is_empty(),
            "only outbox::emit may insert outbox rows; found {offenders:?}"
        );
    }

    fn visit_rust_sources(dir: &std::path::Path, visit: &mut impl FnMut(&std::path::Path, &str)) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                visit_rust_sources(&path, visit);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let body = std::fs::read_to_string(&path).unwrap_or_default();
                visit(&path, &body);
            }
        }
    }

    fn constant_name(kind: EventKind) -> String {
        kind.as_str().replace('.', "_").to_uppercase()
    }

    fn collect_rust_sources(dir: &std::path::Path, out: &mut String) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_rust_sources(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push_str(&std::fs::read_to_string(&path).unwrap_or_default());
            }
        }
    }
    #[tokio::test]
    async fn canonical_writer_emits_the_complete_catalogue_and_disabled_is_a_noop() {
        let db = EdgeDb::in_memory().await.unwrap();
        db.migrate().await.unwrap();
        configure(&db, true, 500_000).await.unwrap();
        let mut tx = db.begin().await.unwrap();
        for kind in EventKind::ALL {
            assert!(
                emit(
                    &mut tx,
                    *kind,
                    &serde_json::json!({ "kind": kind.as_str() }),
                    1_000
                )
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
        assert_eq!(rows.len(), EventKind::ALL.len());
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
            emit(
                &mut tx,
                EventKind::LOCKOUT_SET,
                &serde_json::json!({}),
                2_000
            )
            .await
            .unwrap()
            .is_none()
        );
        tx.commit().await.unwrap();
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM pending_cloud_events")
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert_eq!(count, EventKind::ALL.len() as i64);
    }
}
