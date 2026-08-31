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
/// How long a delivered event stays in the outbox before retention removes it.
///
/// PRD 070 F-070-29. The row has already reached the cloud, which is the durable
/// copy; what is left here is a local receipt, kept for a day so an operator can
/// still answer "did this actually sync?" after an overnight incident.
pub const SYNCED_RETENTION_MS: i64 = 24 * 3_600_000;

/// Deletes synced rows older than [`SYNCED_RETENTION_MS`], oldest first.
///
/// The **only** implementation of F-070-29. Both the hourly retention worker and
/// the drain call it: the drain because it is what creates synced rows and must
/// not compute the cap against a table full of rows that are already due for
/// deletion, the worker because a cloud that is switched off leaves a drain that
/// never runs. It is bounded and idempotent, so both calling it is harmless.
///
/// The boundary is `<=`, so a row synced exactly `SYNCED_RETENTION_MS` ago goes.
/// "Pruned after 24 h" is a deadline, not a half-open interval, and a `<` here
/// would leave the exact-boundary row alive until the next pass — reachable
/// whenever the clock is injected rather than sampled, which is every test.
///
/// A `synced` row with a `NULL` `synced_at` is deleted regardless of age. It is
/// unreachable through [`synced`], which is the only writer of that status, but
/// if one ever existed the `<=` comparison would be false for ever and the row
/// would be the one thing this module must not have: an unbounded class. Its
/// status already says the cloud has it.
pub async fn prune_synced(db: &EdgeDb, now: i64, limit: u32) -> Result<u64, StorageError> {
    let before = now.saturating_sub(SYNCED_RETENTION_MS);
    let limit = i64::from(limit);
    // `query!` rather than `query`: the retention module delegates its outbox
    // arm here, and its contract (M3-004) is that every retention statement is
    // checked against the migrated schema at compile time.
    let result = sqlx::query!(
        "DELETE FROM pending_cloud_events WHERE event_id IN(SELECT event_id FROM pending_cloud_events WHERE status='synced' AND (synced_at IS NULL OR synced_at<=?) ORDER BY synced_at,event_id LIMIT ?)",
        before,
        limit
    )
    .execute(db.pool())
    .await
    .map_err(StorageError::from_sqlx)?;
    Ok(result.rows_affected())
}

/// Enforces `outbox_max_rows` by dropping the cheapest history, oldest first.
///
/// Three rules, and the order between them is the whole design:
///
/// 1. **Nothing high tier is ever deleted.** F-070-27, and the reason the tier
///    column exists at all: the ledger of what the machine did to a living plant
///    is not disposable, and a cap that could delete it would be a cap on
///    honesty. Only `measurement.sample` is low tier.
/// 2. **The cap is measured over unsynced rows** — `pending` *and*
///    `quarantined`. Synced rows are [`prune_synced`]'s business and are on
///    their way out anyway; counting them would prune live history to make room
///    for receipts.
/// 3. **Every row counted as pressure that is low tier is also prunable.** The
///    prunable set and the counted set differ only by tier. An earlier version
///    counted `status!='synced'` but deleted only `status='pending'`, which made
///    low-tier *quarantined* rows pure pressure: they inflated the excess, so
///    each one evicted an extra live pending row, and nothing could ever remove
///    them — an unbounded class hiding inside the mechanism that exists to bound
///    growth.
///
/// **The cap can therefore be exceeded, and that is correct.** Under pressure
/// that is entirely high tier there is nothing this function is allowed to
/// delete, and it deletes nothing rather than deleting something. Preservation
/// wins over the cap; `the_cap_yields_to_preservation_under_high_tier_pressure`
/// asserts it so nobody later "fixes" the cap into a data-loss bug.
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
    if excess <= 0 {
        return Ok(0);
    }
    let result=sqlx::query("DELETE FROM pending_cloud_events WHERE event_id IN(SELECT event_id FROM pending_cloud_events WHERE value_tier='low' AND status!='synced' ORDER BY created_at,event_id LIMIT ?)").bind(excess).execute(db.pool()).await.map_err(StorageError::from_sqlx)?;
    Ok(result.rows_affected())
}

#[cfg(test)]
mod tests {
    use super::*;
    /// Inserts one outbox row in an exact state. The column list is spelled out
    /// so a schema change breaks these tests loudly instead of shifting a
    /// positional `VALUES` list into the wrong columns.
    async fn row(
        db: &EdgeDb,
        id: &str,
        tier: &str,
        status: &str,
        created_at: i64,
        synced_at: Option<i64>,
    ) {
        sqlx::query(
            "INSERT INTO pending_cloud_events(event_id,kind,value_tier,payload_json,status,attempts,next_attempt_at,created_at,synced_at) \
             VALUES(?,'device.event',?,'{}',?,0,0,?,?)",
        )
        .bind(id)
        .bind(tier)
        .bind(status)
        .bind(created_at)
        .bind(synced_at)
        .execute(db.pool())
        .await
        .unwrap();
    }

    async fn ids(db: &EdgeDb) -> Vec<String> {
        sqlx::query_scalar("SELECT event_id FROM pending_cloud_events ORDER BY event_id")
            .fetch_all(db.pool())
            .await
            .unwrap()
    }

    async fn fresh() -> EdgeDb {
        let db = EdgeDb::in_memory().await.unwrap();
        db.migrate().await.unwrap();
        db
    }

    /// **F-070-29.** The deadline is 24 h on `synced_at`, and the boundary row
    /// goes: "pruned after 24 h" is a deadline, not a half-open interval.
    #[tokio::test]
    async fn synced_rows_are_retired_at_twenty_four_hours_and_not_before() {
        let db = fresh().await;
        let now = 100 * 86_400_000i64;
        row(
            &db,
            "just-under",
            "high",
            "synced",
            0,
            Some(now - SYNCED_RETENTION_MS + 1),
        )
        .await;
        row(
            &db,
            "exactly",
            "high",
            "synced",
            0,
            Some(now - SYNCED_RETENTION_MS),
        )
        .await;
        row(
            &db,
            "well-over",
            "high",
            "synced",
            0,
            Some(now - SYNCED_RETENTION_MS - 1),
        )
        .await;

        assert_eq!(prune_synced(&db, now, 500).await.unwrap(), 2);
        assert_eq!(ids(&db).await, vec!["just-under".to_owned()]);
    }

    /// Retention retires *delivered* rows. A row still waiting to be delivered,
    /// or quarantined for an operator to look at, is history the edge still
    /// holds — age is not a reason to lose it, and only `status` says otherwise.
    #[tokio::test]
    async fn retention_never_removes_pending_or_quarantined_rows() {
        let db = fresh().await;
        let now = 100 * 86_400_000i64;
        let ancient = now - 365 * 86_400_000;
        row(&db, "pending-low", "low", "pending", ancient, None).await;
        row(&db, "pending-high", "high", "pending", ancient, None).await;
        row(&db, "quarantined-low", "low", "quarantined", ancient, None).await;
        row(
            &db,
            "quarantined-high",
            "high",
            "quarantined",
            ancient,
            None,
        )
        .await;
        // A quarantined row that carries a stale `synced_at` from an earlier
        // life must not be caught either: the predicate is `status`, not age.
        row(
            &db,
            "quarantined-stale",
            "high",
            "quarantined",
            ancient,
            Some(ancient),
        )
        .await;

        assert_eq!(prune_synced(&db, now, 500).await.unwrap(), 0);
        assert_eq!(ids(&db).await.len(), 5);
    }

    /// Tier decides what the *cap* may drop. It has nothing to say about
    /// retention: a delivered measurement and a delivered watering event are
    /// both receipts for something the cloud already has.
    #[tokio::test]
    async fn retention_retires_both_tiers_once_delivered() {
        let db = fresh().await;
        let now = 100 * 86_400_000i64;
        let old = now - SYNCED_RETENTION_MS - 1;
        row(&db, "low", "low", "synced", 0, Some(old)).await;
        row(&db, "high", "high", "synced", 0, Some(old)).await;

        assert_eq!(prune_synced(&db, now, 500).await.unwrap(), 2);
        assert!(ids(&db).await.is_empty());
    }

    /// A `synced` row with no `synced_at` cannot age out of a `<=` comparison,
    /// so it would live for ever — the one thing this module must not have.
    /// Unreachable through `synced()`, deleted anyway if it ever appears.
    #[tokio::test]
    async fn a_synced_row_with_no_timestamp_is_not_an_unbounded_class() {
        let db = fresh().await;
        row(&db, "orphan", "high", "synced", 0, None).await;
        assert_eq!(prune_synced(&db, 0, 500).await.unwrap(), 1);
        assert!(ids(&db).await.is_empty());
    }

    /// Retention is bounded per pass so a backlog cannot stall the writer.
    #[tokio::test]
    async fn retention_is_bounded_per_pass_and_takes_the_oldest_first() {
        let db = fresh().await;
        let now = 100 * 86_400_000i64;
        for i in 1..=5i64 {
            row(
                &db,
                &format!("e{i}"),
                "high",
                "synced",
                0,
                Some(now - SYNCED_RETENTION_MS - i),
            )
            .await;
        }
        // Oldest first means the largest subtracted offset goes first.
        assert_eq!(prune_synced(&db, now, 2).await.unwrap(), 2);
        assert_eq!(
            ids(&db).await,
            vec!["e1".to_owned(), "e2".to_owned(), "e3".to_owned()]
        );
    }

    /// **The defect this correction fixes.** A low-tier *quarantined* row used
    /// to count as pressure while being un-prunable: it inflated the excess, so
    /// each one evicted an extra live pending row, and nothing could ever remove
    /// it. Counted and prunable are now the same set, differing only by tier.
    #[tokio::test]
    async fn low_tier_quarantined_rows_are_prunable_not_permanent_pressure() {
        let db = fresh().await;
        configure(&db, true, 2).await.unwrap();
        row(&db, "q-oldest", "low", "quarantined", 1, None).await;
        row(&db, "p-second", "low", "pending", 2, None).await;
        row(&db, "p-third", "low", "pending", 3, None).await;
        row(&db, "p-fourth", "low", "pending", 4, None).await;

        // Four unsynced rows against a cap of two: two must go, oldest first,
        // and the quarantined one is first in line rather than immortal.
        assert_eq!(prune_low(&db).await.unwrap(), 2);
        assert_eq!(
            ids(&db).await,
            vec!["p-fourth".to_owned(), "p-third".to_owned()]
        );
    }

    /// **Preservation wins over the cap, and the cap loses.** Under pressure
    /// that is entirely high tier there is nothing the cap is allowed to delete,
    /// so the backlog exceeds `outbox_max_rows` and stays there. This is not a
    /// leak to be tidied up later: the alternative is deleting the ledger of
    /// what the machine did to a living plant in order to satisfy a number.
    #[tokio::test]
    async fn the_cap_yields_to_preservation_under_high_tier_pressure() {
        let db = fresh().await;
        configure(&db, true, 2).await.unwrap();
        for i in 1..=10i64 {
            row(&db, &format!("h{i:02}"), "high", "pending", i, None).await;
        }
        assert_eq!(prune_low(&db).await.unwrap(), 0);
        let (pending, _) = counts(&db).await.unwrap();
        assert_eq!(pending, 10, "the backlog is allowed to exceed the cap of 2");

        // One low-tier row arriving does not make the high-tier ones eligible;
        // it is the only thing that can go, and the cap is still exceeded after.
        row(&db, "l01", "low", "pending", 0, None).await;
        assert_eq!(prune_low(&db).await.unwrap(), 1);
        let survivors = ids(&db).await;
        assert_eq!(survivors.len(), 10);
        assert!(survivors.iter().all(|id| id.starts_with('h')));
    }

    /// Synced rows are retention's business, not the cap's. Counting them as
    /// pressure would evict live history to make room for delivered receipts.
    #[tokio::test]
    async fn the_cap_ignores_already_delivered_rows() {
        let db = fresh().await;
        configure(&db, true, 2).await.unwrap();
        for i in 1..=5i64 {
            row(&db, &format!("s{i}"), "low", "synced", i, Some(i)).await;
        }
        row(&db, "p1", "low", "pending", 10, None).await;
        row(&db, "p2", "low", "pending", 11, None).await;

        assert_eq!(prune_low(&db).await.unwrap(), 0);
        assert_eq!(ids(&db).await.len(), 7);
    }

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
