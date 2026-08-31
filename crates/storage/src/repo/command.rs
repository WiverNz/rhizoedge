//! The command ledger and the irrigation state it moves (M6-008, M6-010, M6-012).
//!
//! # Persist before publish
//!
//! [`issue`] commits the command row with status `issued`, the irrigation state
//! transition, and the outbox row in **one transaction**, and it returns before
//! anything is published. The reverse order would allow a pump to run with no
//! record that it was ever asked to (SAFETY-010, F-060-20).
//!
//! `command_id` is the **primary key**, not a surrogate id, so a duplicate
//! insert fails at the storage layer. The guarantee therefore holds even if a
//! caller later writes a check-then-insert race (SAFETY-001, F-060-21).
//!
//! # Terminal statuses are terminal
//!
//! `completed`, `rejected`, `expired`, `failed`, and `interrupted` are final.
//! [`settle`] updates nothing for a command already in one of them, which is
//! what makes a duplicate `command.result` a no-op rather than a second
//! watering event.
#![allow(
    missing_docs,
    reason = "row structs mirror table columns one for one; the columns are \n              documented in the migration and the module header, and repeating \n              them per field would drown the rules that matter"
)]

use sqlx::Row as _;
use sqlx::{Sqlite, Transaction};

use crate::{EdgeDb, StorageError};

/// A command as stored.
#[derive(Clone, Debug, PartialEq)]
pub struct CommandRow {
    pub command_id: String,
    pub device_id: String,
    pub plant_id: Option<String>,
    pub kind: String,
    pub requested_ml: Option<f64>,
    pub mode: String,
    pub issued_at: i64,
    pub expires_at: i64,
    pub status: String,
    pub published_at: Option<i64>,
    pub settled_at: Option<i64>,
    pub reason: Option<String>,
}

/// A command as a caller supplies it.
#[derive(Clone, Debug, PartialEq)]
pub struct NewCommand {
    pub command_id: String,
    pub device_id: String,
    pub plant_id: Option<String>,
    pub kind: String,
    pub requested_ml: Option<f64>,
    pub mode: String,
    pub issued_at: i64,
    pub expires_at: i64,
}

/// The irrigation state of one plant, as stored.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct IrrigationStateRow {
    pub state: String,
    pub state_since: i64,
    pub doses_this_cycle: i64,
    pub cycle_started_at: Option<i64>,
    pub last_cycle_completed_at: Option<i64>,
    pub wait_until: Option<i64>,
    pub active_command_id: Option<String>,
    pub pre_dose_vwc: Option<f64>,
    pub pre_dose_grams: Option<f64>,
}

/// The statuses from which a command may still change.
pub const OPEN_STATUSES: [&str; 2] = ["issued", "in_flight"];

/// The statuses a command never leaves.
pub const TERMINAL_STATUSES: [&str; 5] =
    ["completed", "rejected", "expired", "failed", "interrupted"];

/// Whether a status is terminal.
#[must_use]
pub fn is_terminal(status: &str) -> bool {
    TERMINAL_STATUSES.contains(&status)
}

fn row_to_command(row: &sqlx::sqlite::SqliteRow) -> CommandRow {
    CommandRow {
        command_id: row.get("command_id"),
        device_id: row.get("device_id"),
        plant_id: row.get("plant_id"),
        kind: row.get("kind"),
        requested_ml: row.get("requested_ml"),
        mode: row.get("mode"),
        issued_at: row.get("issued_at"),
        expires_at: row.get("expires_at"),
        status: row.get("status"),
        published_at: row.get("published_at"),
        settled_at: row.get("settled_at"),
        reason: row.get("reason"),
    }
}

fn row_to_state(row: &sqlx::sqlite::SqliteRow) -> IrrigationStateRow {
    IrrigationStateRow {
        state: row.get("state"),
        state_since: row.get("state_since"),
        doses_this_cycle: row.get("doses_this_cycle"),
        cycle_started_at: row.get("cycle_started_at"),
        last_cycle_completed_at: row.get("last_cycle_completed_at"),
        wait_until: row.get("wait_until"),
        active_command_id: row.get("active_command_id"),
        pre_dose_vwc: row.get("pre_dose_vwc"),
        pre_dose_grams: row.get("pre_dose_grams"),
    }
}

/// Reads the irrigation state of a plant, or `None` if it has never had one.
///
/// **Never construct a default for an existing plant.** A plant reset to
/// `Normal` would silently drop its cooldown, its dose count, and an
/// in-progress absorption wait (M6-012).
pub async fn irrigation_state(
    db: &EdgeDb,
    plant_id: &str,
) -> Result<Option<IrrigationStateRow>, StorageError> {
    Ok(
        sqlx::query("SELECT * FROM irrigation_state WHERE plant_id=?")
            .bind(plant_id)
            .fetch_optional(db.pool())
            .await
            .map_err(StorageError::from_sqlx)?
            .as_ref()
            .map(row_to_state),
    )
}

async fn put_state_in_tx(
    tx: &mut Transaction<'_, Sqlite>,
    plant_id: &str,
    state: &IrrigationStateRow,
    now: i64,
) -> Result<(), StorageError> {
    sqlx::query(
        "INSERT INTO irrigation_state(plant_id,state,state_since,doses_this_cycle,cycle_started_at,last_cycle_completed_at,wait_until,active_command_id,pre_dose_vwc,pre_dose_grams,updated_at) \
         VALUES(?,?,?,?,?,?,?,?,?,?,?) \
         ON CONFLICT(plant_id) DO UPDATE SET state=excluded.state,state_since=excluded.state_since,doses_this_cycle=excluded.doses_this_cycle,cycle_started_at=excluded.cycle_started_at,last_cycle_completed_at=excluded.last_cycle_completed_at,wait_until=excluded.wait_until,active_command_id=excluded.active_command_id,pre_dose_vwc=excluded.pre_dose_vwc,pre_dose_grams=excluded.pre_dose_grams,updated_at=excluded.updated_at",
    )
    .bind(plant_id)
    .bind(&state.state)
    .bind(state.state_since)
    .bind(state.doses_this_cycle)
    .bind(state.cycle_started_at)
    .bind(state.last_cycle_completed_at)
    .bind(state.wait_until)
    .bind(&state.active_command_id)
    .bind(state.pre_dose_vwc)
    .bind(state.pre_dose_grams)
    .bind(now)
    .execute(&mut **tx)
    .await
    .map_err(StorageError::from_sqlx)?;
    Ok(())
}

/// Writes the irrigation state outside a command transaction.
pub async fn put_irrigation_state(
    db: &EdgeDb,
    plant_id: &str,
    state: &IrrigationStateRow,
    now: i64,
) -> Result<(), StorageError> {
    let mut tx = db.begin().await?;
    put_state_in_tx(&mut tx, plant_id, state, now).await?;
    tx.commit().await.map_err(StorageError::from_sqlx)
}

/// Commits a command, its irrigation transition, and its outbox row.
///
/// **One transaction, committed before any publish.** A crash between this
/// commit and the publish leaves an `issued` row with no result, which is
/// exactly the state [`open_commands`] reconciles on the next boot — never a
/// pump that ran with no record.
///
/// # Errors
///
/// A duplicate `command_id` is a primary-key violation and is reported as
/// [`StorageError::Constraint`].
pub async fn issue(
    db: &EdgeDb,
    command: &NewCommand,
    next_state: &IrrigationStateRow,
    now: i64,
) -> Result<(), StorageError> {
    let mut tx = db.begin().await?;
    sqlx::query(
        "INSERT INTO commands(command_id,device_id,plant_id,kind,requested_ml,mode,issued_at,expires_at,status) \
         VALUES(?,?,?,?,?,?,?,?,'issued')",
    )
    .bind(&command.command_id)
    .bind(&command.device_id)
    .bind(&command.plant_id)
    .bind(&command.kind)
    .bind(command.requested_ml)
    .bind(&command.mode)
    .bind(command.issued_at)
    .bind(command.expires_at)
    .execute(&mut *tx)
    .await
    .map_err(StorageError::from_sqlx)?;

    if let Some(plant_id) = command.plant_id.as_deref() {
        put_state_in_tx(&mut tx, plant_id, next_state, now).await?;
    }

    // The cloud is an append-only sink and never an input, so the outbox row is
    // written here purely so history survives a later sync (M7). It shares the
    // transaction because a command the cloud never hears about is a hole in the
    // ledger, not a retryable failure.
    crate::repo::outbox::emit(&mut tx,"command.issued",&serde_json::json!({"command_id":command.command_id,"device_id":command.device_id,"plant_id":command.plant_id,"kind":command.kind,"requested_ml":command.requested_ml,"mode":command.mode,"issued_at":command.issued_at}),now).await?;
    if command.kind == "water" {
        crate::repo::outbox::emit(&mut tx,"watering.started",&serde_json::json!({"watering_event_id":command.command_id,"command_id":command.command_id,"device_id":command.device_id,"plant_id":command.plant_id,"requested_ml":command.requested_ml,"mode":command.mode,"started_at":command.issued_at}),now).await?;
    }

    tx.commit().await.map_err(StorageError::from_sqlx)
}

/// Reads one command.
pub async fn get(db: &EdgeDb, command_id: &str) -> Result<Option<CommandRow>, StorageError> {
    Ok(sqlx::query("SELECT * FROM commands WHERE command_id=?")
        .bind(command_id)
        .fetch_optional(db.pool())
        .await
        .map_err(StorageError::from_sqlx)?
        .as_ref()
        .map(row_to_command))
}

/// Records the moment a command reached the broker.
pub async fn mark_published(db: &EdgeDb, command_id: &str, at: i64) -> Result<bool, StorageError> {
    let done = sqlx::query(
        "UPDATE commands SET published_at=?,status='in_flight' WHERE command_id=? AND status='issued'",
    )
    .bind(at)
    .bind(command_id)
    .execute(db.pool())
    .await
    .map_err(StorageError::from_sqlx)?;
    Ok(done.rows_affected() == 1)
}

/// Every command that has neither settled nor expired, oldest first.
pub async fn open_commands(db: &EdgeDb) -> Result<Vec<CommandRow>, StorageError> {
    let rows = sqlx::query(
        "SELECT * FROM commands WHERE status IN ('issued','in_flight') ORDER BY issued_at",
    )
    .fetch_all(db.pool())
    .await
    .map_err(StorageError::from_sqlx)?;
    Ok(rows.iter().map(row_to_command).collect())
}

/// Settles a command and, when the outcome delivered water, records it.
///
/// Every write shares one transaction: the status change, the optional
/// `watering_event`, and the irrigation transition. Splitting them would
/// reintroduce duplicate watering on a crash (SAFETY-001, SAFETY-010).
///
/// Returns `false` when the command was already terminal, in which case
/// **nothing is written at all** — which is what makes a duplicate result a
/// no-op (F-060-25).
#[allow(clippy::too_many_arguments)]
pub async fn settle(
    db: &EdgeDb,
    command_id: &str,
    status: &str,
    reason: Option<&str>,
    watering: Option<&NewWateringEvent>,
    next_state: Option<(&str, &IrrigationStateRow)>,
    now: i64,
) -> Result<bool, StorageError> {
    let mut tx = db.begin().await?;
    let current: Option<String> =
        sqlx::query_scalar("SELECT status FROM commands WHERE command_id=?")
            .bind(command_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(StorageError::from_sqlx)?;
    match current {
        // A result for a command the edge never issued is logged and ignored by
        // the caller. The edge does not invent a command row to match it.
        None => {
            tx.rollback().await.map_err(StorageError::from_sqlx)?;
            return Ok(false);
        }
        Some(existing) if is_terminal(&existing) => {
            tx.rollback().await.map_err(StorageError::from_sqlx)?;
            return Ok(false);
        }
        Some(_) => {}
    }
    sqlx::query("UPDATE commands SET status=?,reason=?,settled_at=? WHERE command_id=?")
        .bind(status)
        .bind(reason)
        .bind(now)
        .bind(command_id)
        .execute(&mut *tx)
        .await
        .map_err(StorageError::from_sqlx)?;

    if let Some(event) = watering {
        sqlx::query(
            "INSERT INTO watering_events(watering_event_id,plant_id,device_id,command_id,mode,origin,started_at,completed_at,requested_ml,delivered_ml,status,reason_json) \
             VALUES(?,?,?,?,?,'edge_command',?,?,?,?,'completed',?) ON CONFLICT(watering_event_id) DO NOTHING",
        )
        .bind(&event.watering_event_id)
        .bind(&event.plant_id)
        .bind(&event.device_id)
        .bind(command_id)
        .bind(&event.mode)
        .bind(event.started_at)
        .bind(event.completed_at)
        .bind(event.requested_ml)
        .bind(event.delivered_ml)
        .bind(&event.reason_json)
        .execute(&mut *tx)
        .await
        .map_err(StorageError::from_sqlx)?;
        crate::repo::outbox::emit(&mut tx,"watering.completed",&serde_json::json!({"watering_event_id":event.watering_event_id,"command_id":command_id,"device_id":event.device_id,"plant_id":event.plant_id,"mode":event.mode,"started_at":event.started_at,"completed_at":event.completed_at,"requested_ml":event.requested_ml,"delivered_ml":event.delivered_ml}),now).await?;
    }

    if let Some((plant_id, state)) = next_state {
        put_state_in_tx(&mut tx, plant_id, state, now).await?;
    }

    crate::repo::outbox::emit(&mut tx,"command.settled",&serde_json::json!({"command_id":command_id,"status":status,"reason":reason,"occurred_at":now}),now).await?;

    tx.commit().await.map_err(StorageError::from_sqlx)?;
    Ok(true)
}

/// A watering event a completed command produced.
#[derive(Clone, Debug, PartialEq)]
pub struct NewWateringEvent {
    pub watering_event_id: String,
    pub plant_id: String,
    pub device_id: String,
    pub mode: String,
    pub started_at: i64,
    pub completed_at: i64,
    pub requested_ml: Option<f64>,
    pub delivered_ml: Option<f64>,
    pub reason_json: Option<String>,
}

/// The volume charged to the rolling window, **derived from rows**.
///
/// Sums `watering_events` inside the window for the budgeted modes, and adds the
/// conservative credit carried by commands that settled without delivering —
/// `interrupted` and `failed` charge their full `requested_ml` and create no
/// watering event, so the sum alone would under-count them (F-060-26).
///
/// There is no counter anywhere. A restart cannot reset this, because there is
/// nothing to reset (SAFETY-006).
pub async fn delivered_in_window(
    db: &EdgeDb,
    plant_id: &str,
    since: i64,
) -> Result<f64, StorageError> {
    let delivered: Option<f64> = sqlx::query_scalar(
        "SELECT sum(delivered_ml) FROM watering_events \
         WHERE plant_id=? AND mode IN ('automatic','recommended') \
           AND completed_at IS NOT NULL AND completed_at>=?",
    )
    .bind(plant_id)
    .bind(since)
    .fetch_one(db.pool())
    .await
    .map_err(StorageError::from_sqlx)?;
    let credited: Option<f64> = sqlx::query_scalar(
        "SELECT sum(requested_ml) FROM commands \
         WHERE plant_id=? AND kind='water' AND mode IN ('automatic','recommended') \
           AND status IN ('interrupted','failed') AND settled_at IS NOT NULL AND settled_at>=?",
    )
    .bind(plant_id)
    .bind(since)
    .fetch_one(db.pool())
    .await
    .map_err(StorageError::from_sqlx)?;
    Ok(delivered.unwrap_or(0.0) + credited.unwrap_or(0.0))
}

/// Sets or clears a plant's lockout, with its audit fields.
#[allow(clippy::too_many_arguments)]
pub async fn set_lockout(
    db: &EdgeDb,
    plant_id: &str,
    reason: Option<&str>,
    since: Option<i64>,
    hold_until: Option<i64>,
    cleared_by: Option<&str>,
    now: i64,
) -> Result<(), StorageError> {
    let mut tx = db.begin().await?;
    sqlx::query(
        "UPDATE plants SET lockout_reason=?,lockout_since=?,lockout_until=?,lockout_cleared_by=?,lockout_cleared_at=? \
         WHERE plant_id=? AND deleted_at IS NULL",
    )
    .bind(reason)
    .bind(since)
    .bind(hold_until)
    .bind(cleared_by)
    .bind(if reason.is_none() { Some(now) } else { None })
    .bind(plant_id)
    .execute(&mut *tx)
    .await
    .map_err(StorageError::from_sqlx)?;
    let kind = if reason.is_some() {
        "lockout.set"
    } else {
        "lockout.cleared"
    };
    crate::repo::outbox::emit(&mut tx, kind, &serde_json::json!({"plant_id":plant_id,"reason":reason,"since":since,"hold_until":hold_until,"cleared_by":cleared_by}), now).await?;
    tx.commit().await.map_err(StorageError::from_sqlx)?;
    Ok(())
}

/// A plant's lockout as stored, including the held-until deadline.
pub async fn lockout(
    db: &EdgeDb,
    plant_id: &str,
) -> Result<Option<(String, Option<i64>, Option<i64>)>, StorageError> {
    Ok(sqlx::query(
        "SELECT lockout_reason,lockout_since,lockout_until FROM plants WHERE plant_id=? AND deleted_at IS NULL",
    )
    .bind(plant_id)
    .fetch_optional(db.pool())
    .await
    .map_err(StorageError::from_sqlx)?
    .and_then(|row| {
        row.get::<Option<String>, _>("lockout_reason").map(|reason| {
            (
                reason,
                row.get::<Option<i64>, _>("lockout_since"),
                row.get::<Option<i64>, _>("lockout_until"),
            )
        })
    }))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    async fn db() -> EdgeDb {
        let db = EdgeDb::in_memory().await.unwrap();
        db.migrate().await.unwrap();
        crate::repo::outbox::configure(&db, true, 500_000)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO plants(plant_id,name,created_at) VALUES('monstera-01','Monstera',0)",
        )
        .execute(db.pool())
        .await
        .unwrap();
        db
    }

    fn command() -> NewCommand {
        NewCommand {
            command_id: "cmd-1".into(),
            device_id: "plant-node-01".into(),
            plant_id: Some("monstera-01".into()),
            kind: "water".into(),
            requested_ml: Some(40.0),
            mode: "automatic".into(),
            issued_at: 1_000,
            expires_at: 121_000,
        }
    }

    fn dose_issued() -> IrrigationStateRow {
        IrrigationStateRow {
            state: "dose_issued".into(),
            state_since: 1_000,
            doses_this_cycle: 1,
            cycle_started_at: Some(1_000),
            active_command_id: Some("cmd-1".into()),
            pre_dose_vwc: Some(20.0),
            ..IrrigationStateRow::default()
        }
    }

    #[tokio::test]
    async fn the_row_and_the_transition_share_one_transaction() {
        let db = db().await;
        issue(&db, &command(), &dose_issued(), 1_000).await.unwrap();
        let row = get(&db, "cmd-1").await.unwrap().unwrap();
        assert_eq!(row.status, "issued");
        assert_eq!(row.published_at, None, "committed before any publish");
        let state = irrigation_state(&db, "monstera-01").await.unwrap().unwrap();
        assert_eq!(state.state, "dose_issued");
        assert_eq!(state.active_command_id.as_deref(), Some("cmd-1"));
        let outbox: Vec<String> =
            sqlx::query_scalar("SELECT kind FROM pending_cloud_events ORDER BY kind")
                .fetch_all(db.pool())
                .await
                .unwrap();
        assert_eq!(outbox, vec!["command.issued", "watering.started"]);
    }

    #[tokio::test]
    async fn disabled_cloud_emits_no_outbox_row() {
        let db = db().await;
        crate::repo::outbox::configure(&db, false, 500_000)
            .await
            .unwrap();
        issue(&db, &command(), &dose_issued(), 1_000).await.unwrap();
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM pending_cloud_events")
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn a_duplicate_command_id_is_refused_by_the_primary_key() {
        let db = db().await;
        issue(&db, &command(), &dose_issued(), 1_000).await.unwrap();
        let error = issue(&db, &command(), &dose_issued(), 2_000)
            .await
            .unwrap_err();
        assert!(matches!(error, StorageError::Constraint(_)), "{error:?}");
    }

    /// The state a crash between commit and publish leaves behind, and the one
    /// reconciliation reads.
    #[tokio::test]
    async fn a_crash_between_commit_and_publish_leaves_an_issued_row() {
        let db = db().await;
        issue(&db, &command(), &dose_issued(), 1_000).await.unwrap();
        let open = open_commands(&db).await.unwrap();
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].status, "issued");
        assert!(open[0].published_at.is_none());
    }

    #[tokio::test]
    async fn a_result_for_a_terminal_command_changes_nothing() {
        let db = db().await;
        issue(&db, &command(), &dose_issued(), 1_000).await.unwrap();
        let event = NewWateringEvent {
            watering_event_id: "we-1".into(),
            plant_id: "monstera-01".into(),
            device_id: "plant-node-01".into(),
            mode: "automatic".into(),
            started_at: 1_000,
            completed_at: 5_000,
            requested_ml: Some(40.0),
            delivered_ml: Some(40.0),
            reason_json: None,
        };
        assert!(
            settle(&db, "cmd-1", "completed", None, Some(&event), None, 5_000)
                .await
                .unwrap()
        );
        assert!(
            !settle(&db, "cmd-1", "failed", Some("late"), None, None, 9_000)
                .await
                .unwrap(),
            "a second result settles nothing"
        );
        let row = get(&db, "cmd-1").await.unwrap().unwrap();
        assert_eq!(row.status, "completed");
        let events: i64 = sqlx::query_scalar("SELECT count(*) FROM watering_events")
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert_eq!(events, 1);
    }

    #[tokio::test]
    async fn an_unknown_command_id_settles_nothing() {
        let db = db().await;
        assert!(
            !settle(&db, "never-issued", "completed", None, None, None, 1)
                .await
                .unwrap()
        );
        let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM commands")
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert_eq!(rows, 0, "the edge does not invent a command row");
    }

    /// SAFETY-006: the window is a sum over rows, and an interrupted dose is
    /// charged its full request even though it created no watering event.
    #[tokio::test]
    async fn the_rolling_window_sums_rows_and_credits_conservatively() {
        let db = db().await;
        issue(&db, &command(), &dose_issued(), 1_000).await.unwrap();
        let event = NewWateringEvent {
            watering_event_id: "we-1".into(),
            plant_id: "monstera-01".into(),
            device_id: "plant-node-01".into(),
            mode: "automatic".into(),
            started_at: 1_000,
            completed_at: 5_000,
            requested_ml: Some(40.0),
            delivered_ml: Some(38.0),
            reason_json: None,
        };
        settle(&db, "cmd-1", "completed", None, Some(&event), None, 5_000)
            .await
            .unwrap();
        assert!((delivered_in_window(&db, "monstera-01", 0).await.unwrap() - 38.0).abs() < 1e-9);

        let mut second = command();
        second.command_id = "cmd-2".into();
        issue(&db, &second, &dose_issued(), 6_000).await.unwrap();
        settle(&db, "cmd-2", "interrupted", None, None, None, 7_000)
            .await
            .unwrap();
        assert!(
            (delivered_in_window(&db, "monstera-01", 0).await.unwrap() - 78.0).abs() < 1e-9,
            "an interrupted dose charges its full request"
        );

        // A manual dose is a person's responsibility and is outside the
        // automatic budget, though it still resets the cooldown.
        let mut manual = command();
        manual.command_id = "cmd-3".into();
        manual.mode = "manual".into();
        issue(&db, &manual, &dose_issued(), 8_000).await.unwrap();
        let manual_event = NewWateringEvent {
            watering_event_id: "we-3".into(),
            mode: "manual".into(),
            ..event.clone()
        };
        settle(
            &db,
            "cmd-3",
            "completed",
            None,
            Some(&manual_event),
            None,
            9_000,
        )
        .await
        .unwrap();
        assert!(
            (delivered_in_window(&db, "monstera-01", 0).await.unwrap() - 78.0).abs() < 1e-9,
            "manual water is outside the automatic cap"
        );

        // ...and the window really is a window.
        assert_eq!(
            delivered_in_window(&db, "monstera-01", 100_000)
                .await
                .unwrap(),
            0.0
        );
    }

    #[tokio::test]
    async fn publication_is_recorded_once() {
        let db = db().await;
        issue(&db, &command(), &dose_issued(), 1_000).await.unwrap();
        assert!(mark_published(&db, "cmd-1", 1_100).await.unwrap());
        assert!(
            !mark_published(&db, "cmd-1", 1_200).await.unwrap(),
            "a second publish of the same command does not re-stamp it"
        );
        let row = get(&db, "cmd-1").await.unwrap().unwrap();
        assert_eq!(row.published_at, Some(1_100));
        assert_eq!(row.status, "in_flight");
    }

    #[tokio::test]
    async fn a_lockout_round_trips_with_its_audit_fields() {
        let db = db().await;
        set_lockout(&db, "monstera-01", Some("leak"), Some(10), None, None, 10)
            .await
            .unwrap();
        assert_eq!(
            lockout(&db, "monstera-01").await.unwrap(),
            Some(("leak".to_owned(), Some(10), None))
        );
        set_lockout(&db, "monstera-01", None, None, None, Some("operator"), 20)
            .await
            .unwrap();
        assert_eq!(lockout(&db, "monstera-01").await.unwrap(), None);
        let cleared: Option<String> = sqlx::query_scalar(
            "SELECT lockout_cleared_by FROM plants WHERE plant_id='monstera-01'",
        )
        .fetch_one(db.pool())
        .await
        .unwrap();
        assert_eq!(cleared.as_deref(), Some("operator"));
    }

    #[tokio::test]
    async fn irrigation_state_is_never_defaulted_for_an_existing_plant() {
        let db = db().await;
        assert_eq!(irrigation_state(&db, "monstera-01").await.unwrap(), None);
        let stored = IrrigationStateRow {
            state: "wait_for_absorption".into(),
            state_since: 500,
            doses_this_cycle: 2,
            wait_until: Some(900_000),
            ..IrrigationStateRow::default()
        };
        put_irrigation_state(&db, "monstera-01", &stored, 500)
            .await
            .unwrap();
        let read = irrigation_state(&db, "monstera-01").await.unwrap().unwrap();
        assert_eq!(read, stored, "restored exactly, including wait_until");
    }
}
