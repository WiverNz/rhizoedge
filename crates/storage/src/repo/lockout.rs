//! Independent plant lockout records: one row per reason (`plant_lockouts`).
//!
//! # Why a table, and not the column
//!
//! `plants.lockout_reason` holds **one** reason and every writer replaces it, so
//! a temporary condition could overwrite a latched one and then clear itself,
//! taking the latch with it: a forward clock step over a leak, a device's
//! `clock_unsynced` refusal over a leak, a detected leak over a latched
//! `no_delivery_detected`. Here no writer can displace another's row:
//!
//! - a **latch** (F-060-41) is inserted once, keeps the instant it was first
//!   latched, and is removed only by an operator clearing *that* reason;
//! - a **hold** (F-060-51's clock step, `clock_unsynced`) only ever lengthens,
//!   expires by the edge clock, and is never cleared by an operator.
//!
//! Derived conditions (stale data, tank low, ...) are re-evaluated on every
//! pass and are never recorded.
//!
//! # The authority for every recorded lockout
//!
//! The gate's latched and held inputs are read from here ([`active`]) and
//! nowhere else. `plants.lockout_reason` is only a **display summary** of the
//! best-ranked current reason, written by
//! [`super::command::set_summary`]; no watering decision reads it.
//!
//! # Fail-closed classification
//!
//! A reason this module does not recognise is a **latch**: an unreadable reason
//! is not an absent one (SAFETY-012), and a hold would expire on its own. It is
//! cleared by its exact stored name, which [`active`] reports.

use sqlx::Row as _;
use sqlx::{Sqlite, Transaction};

use crate::{EdgeDb, StorageError};

/// Explicit-clear reasons (F-060-41). The edge's `LockoutReason` names;
/// `edge-controller` checks the three lists against the enum.
pub const LATCH_REASONS: [&str; 5] = [
    "leak",
    "pump_fault",
    "no_delivery_detected",
    "max_doses_reached",
    "unknown",
];

/// Reasons a writer may record as a time-bounded hold.
pub const HOLD_REASONS: [&str; 2] = ["uncertain", "clock_unsynced"];

/// Reasons re-derived from live inputs on every pass, and never latched.
pub const DERIVED_REASONS: [&str; 5] = [
    "stale_data",
    "sensor_fault",
    "tank_low",
    "daily_limit",
    "no_actuator",
];

/// What a stored row is.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LockoutKind {
    /// Explicit-clear; removed only by an operator.
    Latch,
    /// Time-bounded; expires by the edge clock.
    Hold,
}

impl LockoutKind {
    /// The stored name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Latch => "latch",
            Self::Hold => "hold",
        }
    }
}

/// One stored lockout.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LockoutRecord {
    /// The stored reason name, exactly as written.
    pub reason: String,
    /// Latch or hold.
    pub kind: LockoutKind,
    /// When it began: first latched, or when the current hold became active.
    pub since: i64,
    /// A hold's deadline; `None` for a latch.
    pub hold_until: Option<i64>,
}

/// How a legacy `(reason, hold_until)` pair is recorded, or `None` for a known
/// derived reason that is not recorded at all. The same rule migration `0002`
/// applies in SQL.
#[must_use]
pub fn classify_legacy(reason: &str, hold_until: Option<i64>) -> Option<LockoutKind> {
    let known_non_latch = HOLD_REASONS.contains(&reason) || DERIVED_REASONS.contains(&reason);
    match (known_non_latch, hold_until) {
        (true, Some(_)) => Some(LockoutKind::Hold),
        (true, None) => None,
        // A known latch, or a name nobody recognises: both need a person.
        (false, _) => Some(LockoutKind::Latch),
    }
}

/// Latches `reason` on a live plant. `true` when it was not already latched.
///
/// A latch already present keeps its original `since`.
///
/// # Errors
///
/// [`StorageError::Constraint`] for a hold or derived reason: latching one
/// would turn a self-clearing condition into a permanent block, which is a
/// caller bug rather than something to record.
pub async fn latch(
    db: &EdgeDb,
    plant_id: &str,
    reason: &str,
    now: i64,
) -> Result<bool, StorageError> {
    if HOLD_REASONS.contains(&reason) || DERIVED_REASONS.contains(&reason) {
        return Err(StorageError::Constraint(format!(
            "`{reason}` is not an explicit-clear lockout"
        )));
    }
    let mut tx = db.begin().await?;
    let inserted = insert_latch_row(&mut tx, plant_id, reason, now).await?;
    if inserted {
        record_set(
            &mut tx,
            plant_id,
            reason,
            LockoutKind::Latch,
            now,
            None,
            now,
        )
        .await?;
    }
    tx.commit().await.map_err(StorageError::from_sqlx)?;
    Ok(inserted)
}

/// Holds a live plant on `reason` until at least `until`. `true` when the hold
/// became active (it was absent or had expired); extending an active hold is
/// not a new block and records nothing.
///
/// A hold never shortens: the stored deadline becomes the later of the two.
///
/// # Errors
///
/// [`StorageError::Constraint`] for a reason outside [`HOLD_REASONS`]: a latch
/// recorded as a hold would expire on its own.
pub async fn hold(
    db: &EdgeDb,
    plant_id: &str,
    reason: &str,
    until: i64,
    now: i64,
) -> Result<bool, StorageError> {
    let mut tx = db.begin().await?;
    let activated = hold_in_tx(&mut tx, plant_id, reason, until, now).await?;
    tx.commit().await.map_err(StorageError::from_sqlx)?;
    Ok(activated)
}

/// [`hold`], inside a caller's transaction — so a settlement can hold a plant
/// in the same transaction that records why.
pub(crate) async fn hold_in_tx(
    tx: &mut Transaction<'_, Sqlite>,
    plant_id: &str,
    reason: &str,
    until: i64,
    now: i64,
) -> Result<bool, StorageError> {
    if !HOLD_REASONS.contains(&reason) {
        return Err(StorageError::Constraint(format!(
            "`{reason}` is not a hold reason"
        )));
    }
    let activated = extend_hold_row(tx, plant_id, reason, until, now, now).await?;
    if activated {
        record_set(
            tx,
            plant_id,
            reason,
            LockoutKind::Hold,
            now,
            Some(until),
            now,
        )
        .await?;
    }
    Ok(activated)
}

/// The leak reading a `leak` latch may only be cleared against: the bound
/// stream, and the receipt time a reading must be newer than to be fresh.
#[derive(Clone, Copy, Debug)]
pub struct LeakEvidence<'a> {
    /// The bound leak sensor's device.
    pub device_id: &'a str,
    /// Its sensor id.
    pub sensor_id: &'a str,
    /// Its point.
    pub point: &'a str,
    /// A reading received at or before this edge instant is stale.
    pub fresh_after: i64,
}

/// What [`clear_latch`] did.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClearOutcome {
    /// The latch is gone; it had been latched at this instant.
    Cleared {
        /// When it was first latched.
        since: i64,
    },
    /// The plant has no latch by that name — never latched, already cleared,
    /// or a hold, which is not an operator's to clear.
    NotLatched,
    /// A `leak` latch whose evidence is not a fresh, positively dry reading
    /// at the moment of clearing (SAFETY-003). Nothing changed.
    LeakNotClear,
}

/// Clears exactly one latch, named by its stored reason. Never touches a hold
/// or any other latch.
///
/// **The check and the delete are one transaction**, begun `IMMEDIATE` so it
/// holds SQLite's write lock from its first statement: no measurement can be
/// ingested, and no other writer can latch or clear, between reading the
/// evidence and removing the latch. Clearing `leak` requires `evidence`, and
/// the latest reading on that stream must be `ok`, positively dry, and fresh;
/// with no evidence, or any other answer, it is refused (SAFETY-003,
/// SAFETY-012). Other latches have no live condition to check: the faults they
/// record are not something a reading can disprove.
///
/// # Errors
///
/// A database failure, including `Busy` when another writer holds the lock.
pub async fn clear_latch(
    db: &EdgeDb,
    plant_id: &str,
    reason: &str,
    cleared_by: &str,
    evidence: Option<LeakEvidence<'_>>,
    now: i64,
) -> Result<ClearOutcome, StorageError> {
    let mut tx = db
        .pool()
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(StorageError::from_sqlx)?;
    if reason == "leak" {
        let Some(evidence) = evidence else {
            return Ok(ClearOutcome::LeakNotClear);
        };
        // The same row the gate reads: latest by receipt, then by id.
        let latest: Option<(Option<i64>, String, i64)> = sqlx::query_as(
            "SELECT value_bool,quality,received_at FROM measurements \
             WHERE device_id=? AND sensor_id=? AND point=? AND kind='leak_state' \
             ORDER BY received_at DESC, id DESC LIMIT 1",
        )
        .bind(evidence.device_id)
        .bind(evidence.sensor_id)
        .bind(evidence.point)
        .fetch_optional(&mut *tx)
        .await
        .map_err(StorageError::from_sqlx)?;
        let dry = matches!(
            latest,
            Some((Some(0), quality, received_at))
                if quality == "ok" && received_at > evidence.fresh_after
        );
        if !dry {
            return Ok(ClearOutcome::LeakNotClear);
        }
    }
    let since = delete_latch_row(&mut tx, plant_id, reason).await?;
    let Some(since) = since else {
        return Ok(ClearOutcome::NotLatched);
    };
    let detail = serde_json::json!({
        "reason": null,
        "cleared_reason": reason,
        "kind": LockoutKind::Latch.as_str(),
        "since": since,
        "cleared_by": cleared_by,
    });
    record_event(
        &mut tx,
        plant_id,
        &format!("plant_lockout:{plant_id}:{reason}:cleared:{now}"),
        "lockout_cleared",
        "info",
        &detail,
        now,
    )
    .await?;
    crate::repo::outbox::emit(&mut tx, crate::repo::outbox::EventKind::LOCKOUT_CLEARED, &serde_json::json!({"plant_id":plant_id,"reason":null,"cleared_reason":reason,"since":since,"cleared_by":cleared_by}), now).await?;
    tx.commit().await.map_err(StorageError::from_sqlx)?;
    Ok(ClearOutcome::Cleared { since })
}

/// The stored names of every lockout blocking `plant_id` at `now` — each
/// latch, and each hold whose deadline is ahead — inside a caller's
/// transaction, sorted. Not joined to `plants`: a soft-deleted plant is not
/// one to water either, so its records still refuse.
pub(crate) async fn blocking_in_tx(
    tx: &mut Transaction<'_, Sqlite>,
    plant_id: &str,
    now: i64,
) -> Result<Vec<String>, StorageError> {
    sqlx::query_scalar(
        "SELECT reason FROM plant_lockouts \
         WHERE plant_id=? AND (kind='latch' OR hold_until>?) ORDER BY reason",
    )
    .bind(plant_id)
    .bind(now)
    .fetch_all(&mut **tx)
    .await
    .map_err(StorageError::from_sqlx)
}

/// Whether the plant has any latch, inside a transaction.
pub(crate) async fn has_latch_in_tx(
    tx: &mut Transaction<'_, Sqlite>,
    plant_id: &str,
) -> Result<bool, StorageError> {
    let found: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM plant_lockouts WHERE plant_id=? AND kind='latch' LIMIT 1",
    )
    .bind(plant_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(StorageError::from_sqlx)?;
    Ok(found.is_some())
}

/// Whether `reason` is an active hold on the plant, inside a transaction.
pub(crate) async fn hold_is_active_in_tx(
    tx: &mut Transaction<'_, Sqlite>,
    plant_id: &str,
    reason: &str,
    now: i64,
) -> Result<bool, StorageError> {
    let found: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM plant_lockouts WHERE plant_id=? AND reason=? AND kind='hold' AND hold_until>?",
    )
    .bind(plant_id)
    .bind(reason)
    .bind(now)
    .fetch_optional(&mut **tx)
    .await
    .map_err(StorageError::from_sqlx)?;
    Ok(found.is_some())
}

/// Every lockout blocking a live plant at `now`: all latches, and holds whose
/// deadline is still ahead. Latches first, then by reason.
///
/// # Errors
///
/// A database failure.
pub async fn active(
    db: &EdgeDb,
    plant_id: &str,
    now: i64,
) -> Result<Vec<LockoutRecord>, StorageError> {
    let rows = sqlx::query(
        "SELECT l.reason,l.kind,l.since,l.hold_until FROM plant_lockouts l \
         JOIN plants p ON p.plant_id=l.plant_id AND p.deleted_at IS NULL \
         WHERE l.plant_id=? AND (l.kind='latch' OR l.hold_until>?) \
         ORDER BY l.kind DESC, l.reason",
    )
    .bind(plant_id)
    .bind(now)
    .fetch_all(db.pool())
    .await
    .map_err(StorageError::from_sqlx)?;
    Ok(rows
        .iter()
        .map(|row| LockoutRecord {
            reason: row.get("reason"),
            // The CHECK constraint admits only these two; anything else would
            // be a corrupt row, and a corrupt row is read as a latch.
            kind: if row.get::<String, _>("kind") == "hold" {
                LockoutKind::Hold
            } else {
                LockoutKind::Latch
            },
            since: row.get("since"),
            hold_until: row.get("hold_until"),
        })
        .collect())
}

/// Every live plant's blocking lockouts at `now`, as `(plant_id, record)`, in
/// one read — for views that list many plants. Same rule as [`active`].
///
/// # Errors
///
/// A database failure.
pub async fn active_all(
    db: &EdgeDb,
    now: i64,
) -> Result<Vec<(String, LockoutRecord)>, StorageError> {
    let rows = sqlx::query(
        "SELECT l.plant_id,l.reason,l.kind,l.since,l.hold_until FROM plant_lockouts l \
         JOIN plants p ON p.plant_id=l.plant_id AND p.deleted_at IS NULL \
         WHERE l.kind='latch' OR l.hold_until>? ORDER BY l.plant_id, l.reason",
    )
    .bind(now)
    .fetch_all(db.pool())
    .await
    .map_err(StorageError::from_sqlx)?;
    Ok(rows
        .iter()
        .map(|row| {
            (
                row.get("plant_id"),
                LockoutRecord {
                    reason: row.get("reason"),
                    kind: if row.get::<String, _>("kind") == "hold" {
                        LockoutKind::Hold
                    } else {
                        LockoutKind::Latch
                    },
                    since: row.get("since"),
                    hold_until: row.get("hold_until"),
                },
            )
        })
        .collect())
}

/// Inserts a latch on a live plant; `true` if it was not already there.
async fn insert_latch_row(
    tx: &mut Transaction<'_, Sqlite>,
    plant_id: &str,
    reason: &str,
    since: i64,
) -> Result<bool, StorageError> {
    // `WHERE` before the upsert clause is required on INSERT ... SELECT.
    let done = sqlx::query(
        "INSERT INTO plant_lockouts(plant_id,reason,kind,since,hold_until) \
         SELECT plant_id,?,'latch',?,NULL FROM plants WHERE plant_id=? AND deleted_at IS NULL \
         ON CONFLICT(plant_id,reason) DO NOTHING",
    )
    .bind(reason)
    .bind(since)
    .bind(plant_id)
    .execute(&mut **tx)
    .await
    .map_err(StorageError::from_sqlx)?;
    Ok(done.rows_affected() == 1)
}

/// Inserts or lengthens a hold on a live plant; `true` if it became active
/// (absent, or expired at `now`).
///
/// A row that is already a latch under the same name is left alone: the
/// reason lists are disjoint, and a latch must never be turned into something
/// that expires.
async fn extend_hold_row(
    tx: &mut Transaction<'_, Sqlite>,
    plant_id: &str,
    reason: &str,
    until: i64,
    since: i64,
    now: i64,
) -> Result<bool, StorageError> {
    let prior: Option<(String, Option<i64>)> =
        sqlx::query_as("SELECT kind,hold_until FROM plant_lockouts WHERE plant_id=? AND reason=?")
            .bind(plant_id)
            .bind(reason)
            .fetch_optional(&mut **tx)
            .await
            .map_err(StorageError::from_sqlx)?;
    let done = sqlx::query(
        "INSERT INTO plant_lockouts(plant_id,reason,kind,since,hold_until) \
         SELECT plant_id,?,'hold',?,? FROM plants WHERE plant_id=? AND deleted_at IS NULL \
         ON CONFLICT(plant_id,reason) DO UPDATE SET \
           since=CASE WHEN plant_lockouts.hold_until<=? THEN excluded.since ELSE plant_lockouts.since END, \
           hold_until=max(plant_lockouts.hold_until,excluded.hold_until) \
         WHERE plant_lockouts.kind='hold'",
    )
    .bind(reason)
    .bind(since)
    .bind(until)
    .bind(plant_id)
    .bind(now)
    .execute(&mut **tx)
    .await
    .map_err(StorageError::from_sqlx)?;
    let was_active =
        matches!(&prior, Some((kind, Some(prior_until))) if kind == "hold" && *prior_until > now);
    Ok(done.rows_affected() == 1 && !was_active && until > now)
}

/// Deletes one latch; returns when it was latched.
async fn delete_latch_row(
    tx: &mut Transaction<'_, Sqlite>,
    plant_id: &str,
    reason: &str,
) -> Result<Option<i64>, StorageError> {
    sqlx::query_scalar(
        "DELETE FROM plant_lockouts WHERE plant_id=? AND reason=? AND kind='latch' RETURNING since",
    )
    .bind(plant_id)
    .bind(reason)
    .fetch_optional(&mut **tx)
    .await
    .map_err(StorageError::from_sqlx)
}

/// The local and cloud records of a new block, in the caller's transaction.
async fn record_set(
    tx: &mut Transaction<'_, Sqlite>,
    plant_id: &str,
    reason: &str,
    kind: LockoutKind,
    since: i64,
    hold_until: Option<i64>,
    now: i64,
) -> Result<(), StorageError> {
    let detail = serde_json::json!({
        "reason": reason,
        "kind": kind.as_str(),
        "since": since,
        "hold_until": hold_until,
    });
    record_event(
        tx,
        plant_id,
        &format!("plant_lockout:{plant_id}:{reason}:set:{now}"),
        "lockout_set",
        "warning",
        &detail,
        now,
    )
    .await?;
    crate::repo::outbox::emit(tx, crate::repo::outbox::EventKind::LOCKOUT_SET, &serde_json::json!({"plant_id":plant_id,"reason":reason,"kind":kind.as_str(),"since":since,"hold_until":hold_until}), now).await?;
    Ok(())
}

/// One `plant_events` row; idempotent on `event_id`.
async fn record_event(
    tx: &mut Transaction<'_, Sqlite>,
    plant_id: &str,
    event_id: &str,
    kind: &str,
    severity: &str,
    detail: &serde_json::Value,
    now: i64,
) -> Result<(), StorageError> {
    sqlx::query(
        "INSERT INTO plant_events(event_id,plant_id,kind,severity,detail_json,occurred_at) \
         VALUES(?,?,?,?,?,?) ON CONFLICT(event_id) DO NOTHING",
    )
    .bind(event_id)
    .bind(plant_id)
    .bind(kind)
    .bind(severity)
    .bind(detail.to_string())
    .bind(now)
    .execute(&mut **tx)
    .await
    .map_err(StorageError::from_sqlx)?;
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    const PLANT: &str = "monstera-01";

    async fn seed(db: &EdgeDb) {
        crate::repo::outbox::configure(db, true, 500_000)
            .await
            .unwrap();
        sqlx::query("INSERT INTO plants(plant_id,name,created_at) VALUES(?,'Monstera',0)")
            .bind(PLANT)
            .execute(db.pool())
            .await
            .unwrap();
    }

    async fn db() -> EdgeDb {
        let db = EdgeDb::in_memory().await.unwrap();
        db.migrate().await.unwrap();
        seed(&db).await;
        db
    }

    /// `(reason, kind)` of every row, expired holds included, sorted.
    async fn rows(db: &EdgeDb) -> Vec<(String, String)> {
        sqlx::query_as("SELECT reason,kind FROM plant_lockouts WHERE plant_id=? ORDER BY reason")
            .bind(PLANT)
            .fetch_all(db.pool())
            .await
            .unwrap()
    }

    async fn active_reasons(db: &EdgeDb, now: i64) -> Vec<String> {
        let mut reasons: Vec<String> = active(db, PLANT, now)
            .await
            .unwrap()
            .into_iter()
            .map(|r| r.reason)
            .collect();
        reasons.sort();
        reasons
    }

    /// `(kind, detail)` of every plant lockout event, oldest first.
    async fn events(db: &EdgeDb) -> Vec<(String, serde_json::Value)> {
        sqlx::query_as::<_, (String, String)>(
            "SELECT kind,detail_json FROM plant_events \
             WHERE plant_id=? AND kind IN('lockout_set','lockout_cleared') ORDER BY occurred_at,rowid",
        )
        .bind(PLANT)
        .fetch_all(db.pool())
        .await
        .unwrap()
        .into_iter()
        .map(|(kind, detail)| (kind, serde_json::from_str(&detail).unwrap()))
        .collect()
    }

    async fn outbox(db: &EdgeDb) -> Vec<(String, serde_json::Value)> {
        sqlx::query_as::<_, (String, String)>(
            "SELECT kind,payload_json FROM pending_cloud_events ORDER BY created_at,rowid",
        )
        .fetch_all(db.pool())
        .await
        .unwrap()
        .into_iter()
        .map(|(kind, payload)| (kind, serde_json::from_str(&payload).unwrap()))
        .collect()
    }

    /// One leak reading on the bound stream, received at `at`.
    async fn reading(db: &EdgeDb, wet: bool, at: i64) {
        sqlx::query(
            "INSERT INTO devices(device_id,created_at) VALUES('node',0) ON CONFLICT DO NOTHING",
        )
        .execute(db.pool())
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO measurements(device_id,sensor_id,point,kind,value_bool,unit,quality,received_at,batch_id,origin) \
             VALUES('node','leak-0','tray','leak_state',?,'boolean','ok',?,?,'live')",
        )
        .bind(i64::from(wet))
        .bind(at)
        .bind(format!("batch-{at}"))
        .execute(db.pool())
        .await
        .unwrap();
    }

    fn evidence(fresh_after: i64) -> LeakEvidence<'static> {
        LeakEvidence {
            device_id: "node",
            sensor_id: "leak-0",
            point: "tray",
            fresh_after,
        }
    }

    /// SAFETY-003's explicit reset, checked inside the clearing transaction:
    /// no evidence, a wet reading, a stale reading, or no reading at all each
    /// refuse and change nothing; only a fresh dry reading clears.
    #[tokio::test]
    async fn a_leak_latch_clears_only_against_a_fresh_dry_reading() {
        let db = db().await;
        latch(&db, PLANT, "leak", 10).await.unwrap();
        let refuse = |evidence: Option<LeakEvidence<'static>>| {
            let db = db.clone();
            async move {
                clear_latch(&db, PLANT, "leak", "operator", evidence, 1_000)
                    .await
                    .unwrap()
            }
        };
        assert_eq!(refuse(None).await, ClearOutcome::LeakNotClear);
        assert_eq!(
            refuse(Some(evidence(0))).await,
            ClearOutcome::LeakNotClear,
            "no reading"
        );
        reading(&db, true, 900).await;
        assert_eq!(
            refuse(Some(evidence(0))).await,
            ClearOutcome::LeakNotClear,
            "wet"
        );
        reading(&db, false, 950).await;
        assert_eq!(
            refuse(Some(evidence(950))).await,
            ClearOutcome::LeakNotClear,
            "dry but stale: received at the freshness boundary"
        );
        assert_eq!(active_reasons(&db, 1_000).await, ["leak"]);
        assert_eq!(events(&db).await.len(), 1, "a refusal records nothing");
        assert_eq!(
            refuse(Some(evidence(900))).await,
            ClearOutcome::Cleared { since: 10 }
        );
        assert!(active(&db, PLANT, 1_000).await.unwrap().is_empty());
    }

    /// A latch keeps the instant it was *first* latched; re-asserting it is
    /// not a new block and records nothing.
    #[tokio::test]
    async fn a_latch_keeps_its_original_since_and_is_recorded_once() {
        let db = db().await;
        assert!(latch(&db, PLANT, "leak", 1_000).await.unwrap());
        assert!(!latch(&db, PLANT, "leak", 9_000).await.unwrap());
        let records = active(&db, PLANT, 9_000).await.unwrap();
        assert_eq!(
            records,
            [LockoutRecord {
                reason: "leak".into(),
                kind: LockoutKind::Latch,
                since: 1_000,
                hold_until: None,
            }]
        );
        assert_eq!(events(&db).await.len(), 1);
        let outbox = outbox(&db).await;
        assert_eq!(outbox.len(), 1);
        assert_eq!(outbox[0].0, "lockout.set");
        assert_eq!(outbox[0].1["reason"], "leak");
        assert_eq!(outbox[0].1["kind"], "latch");
    }

    /// The core property: several reasons coexist, and clearing one touches
    /// nothing else — not another latch, and not a hold.
    #[tokio::test]
    async fn reasons_are_independent_and_clearing_one_removes_only_it() {
        let db = db().await;
        for reason in [
            "leak",
            "pump_fault",
            "no_delivery_detected",
            "max_doses_reached",
        ] {
            assert!(latch(&db, PLANT, reason, 100).await.unwrap());
        }
        assert!(hold(&db, PLANT, "uncertain", 10_000, 200).await.unwrap());

        reading(&db, false, 290).await;
        assert_eq!(
            clear_latch(&db, PLANT, "leak", "operator", Some(evidence(0)), 300)
                .await
                .unwrap(),
            ClearOutcome::Cleared { since: 100 }
        );
        assert_eq!(
            active_reasons(&db, 300).await,
            [
                "max_doses_reached",
                "no_delivery_detected",
                "pump_fault",
                "uncertain"
            ]
        );
        // A hold is not an operator's to clear.
        assert_eq!(
            clear_latch(&db, PLANT, "uncertain", "operator", None, 300)
                .await
                .unwrap(),
            ClearOutcome::NotLatched
        );
        // Clearing a latch that is not there changes and records nothing.
        let before = events(&db).await.len();
        assert_eq!(
            clear_latch(&db, PLANT, "leak", "operator", Some(evidence(0)), 400)
                .await
                .unwrap(),
            ClearOutcome::NotLatched
        );
        assert_eq!(events(&db).await.len(), before);

        let (kind, detail) = events(&db).await.into_iter().nth(5).unwrap();
        assert_eq!(kind, "lockout_cleared");
        assert_eq!(detail["cleared_reason"], "leak");
        assert_eq!(detail["cleared_by"], "operator");
        let cleared = outbox(&db)
            .await
            .into_iter()
            .find(|(kind, _)| kind == "lockout.cleared")
            .unwrap();
        assert_eq!(cleared.1["cleared_reason"], "leak");
    }

    /// A hold only ever lengthens, expires by the clock, records one event per
    /// activation, and expiring touches no latch.
    #[tokio::test]
    async fn a_hold_never_shortens_and_its_expiry_leaves_latches_alone() {
        let db = db().await;
        assert!(latch(&db, PLANT, "leak", 0).await.unwrap());
        assert!(hold(&db, PLANT, "uncertain", 5_000, 1_000).await.unwrap());
        assert!(!hold(&db, PLANT, "uncertain", 3_000, 2_000).await.unwrap());
        assert!(!hold(&db, PLANT, "uncertain", 8_000, 2_000).await.unwrap());
        let held = active(&db, PLANT, 2_000).await.unwrap();
        let uncertain = held.iter().find(|r| r.reason == "uncertain").unwrap();
        assert_eq!(uncertain.hold_until, Some(8_000), "the later deadline wins");
        assert_eq!(uncertain.since, 1_000);

        assert_eq!(
            active_reasons(&db, 8_000).await,
            ["leak"],
            "expired at the deadline"
        );
        assert_eq!(events(&db).await.len(), 2, "one latch, one hold activation");

        // An expired hold re-activates as a new block, with a new `since`.
        assert!(hold(&db, PLANT, "uncertain", 20_000, 9_000).await.unwrap());
        let held = active(&db, PLANT, 9_000).await.unwrap();
        assert_eq!(
            held.iter().find(|r| r.reason == "uncertain").unwrap().since,
            9_000
        );
        assert_eq!(events(&db).await.len(), 3);
    }

    /// Fail-closed classification at the API: a latch cannot be recorded as
    /// something that expires, and a self-clearing condition cannot be latched.
    #[tokio::test]
    async fn a_reason_cannot_be_recorded_as_the_wrong_kind() {
        let db = db().await;
        for reason in ["leak", "max_doses_reached", "something_new", "stale_data"] {
            assert!(
                matches!(
                    hold(&db, PLANT, reason, 9_000, 0).await,
                    Err(StorageError::Constraint(_))
                ),
                "{reason}"
            );
        }
        for reason in ["uncertain", "clock_unsynced", "tank_low", "no_actuator"] {
            assert!(
                matches!(
                    latch(&db, PLANT, reason, 0).await,
                    Err(StorageError::Constraint(_))
                ),
                "{reason}"
            );
        }
        assert!(rows(&db).await.is_empty());
        assert!(events(&db).await.is_empty());
    }

    /// An unrecognised reason is a latch, is reported by its exact name, and
    /// can be cleared by that name — the diagnosable way out.
    #[tokio::test]
    async fn an_unrecognised_reason_latches_and_clears_by_its_exact_name() {
        let db = db().await;
        assert!(latch(&db, PLANT, "flood_v9", 10).await.unwrap());
        let records = active(&db, PLANT, 1_000_000).await.unwrap();
        assert_eq!(records[0].reason, "flood_v9");
        assert_eq!(records[0].kind, LockoutKind::Latch);
        assert_eq!(
            clear_latch(&db, PLANT, "flood_v9", "operator", None, 20)
                .await
                .unwrap(),
            ClearOutcome::Cleared { since: 10 }
        );
        assert!(active(&db, PLANT, 20).await.unwrap().is_empty());
    }

    /// A soft-deleted or unknown plant gets no row and no event, and a deleted
    /// plant's rows are not reported.
    #[tokio::test]
    async fn only_live_plants_are_recorded_and_reported() {
        let db = db().await;
        assert!(!latch(&db, "no-such-plant", "leak", 0).await.unwrap());
        assert!(latch(&db, PLANT, "leak", 0).await.unwrap());
        sqlx::query("UPDATE plants SET deleted_at=1 WHERE plant_id=?")
            .bind(PLANT)
            .execute(db.pool())
            .await
            .unwrap();
        assert!(active(&db, PLANT, 2).await.unwrap().is_empty());
        assert!(!latch(&db, PLANT, "pump_fault", 3).await.unwrap());
        assert_eq!(rows(&db).await, [("leak".into(), "latch".into())]);
    }

    /// Requirement 4: a restart loses nothing. File-backed, closed, reopened,
    /// and migrated again.
    #[tokio::test]
    async fn every_lockout_survives_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("edge.sqlite");
        {
            let db = EdgeDb::connect(&path).await.unwrap();
            db.migrate().await.unwrap();
            seed(&db).await;
            latch(&db, PLANT, "leak", 10).await.unwrap();
            latch(&db, PLANT, "no_delivery_detected", 20).await.unwrap();
            hold(&db, PLANT, "uncertain", 50_000, 30).await.unwrap();
            db.pool().close().await;
        }
        let db = EdgeDb::connect(&path).await.unwrap();
        db.migrate().await.unwrap();
        assert_eq!(
            active(&db, PLANT, 40).await.unwrap(),
            [
                LockoutRecord {
                    reason: "leak".into(),
                    kind: LockoutKind::Latch,
                    since: 10,
                    hold_until: None,
                },
                LockoutRecord {
                    reason: "no_delivery_detected".into(),
                    kind: LockoutKind::Latch,
                    since: 20,
                    hold_until: None,
                },
                LockoutRecord {
                    reason: "uncertain".into(),
                    kind: LockoutKind::Hold,
                    since: 30,
                    hold_until: Some(50_000),
                },
            ]
        );
    }
}
