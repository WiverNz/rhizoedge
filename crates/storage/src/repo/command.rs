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

/// Writes the irrigation state only if it still reads as `expected`.
///
/// # There are two writers, and one of them evaluates before it writes
///
/// The control pass reads the plant's state, evaluates the pure machine, and
/// writes the result — and a `command.result` arriving from the pipeline in
/// between settles the same plant from a different task. Both writers are
/// correct in isolation; together they lose an update, because the pass writes
/// a row derived from a read that is no longer true.
///
/// The observed failure is specific and worth recording. A dose settles, the
/// pipeline moves the plant `dose_issued -> wait_for_absorption`, and the pass —
/// which read `dose_issued` a moment earlier — writes `dose_issued` back. It
/// records no transition, because from its own point of view nothing changed.
/// The plant is then held in `dose_issued` against a command that has already
/// completed, and the machine answers `Wait` for ever: fail-safe, since no
/// second dose can be issued, but the plant stops being watered until the
/// process restarts and `reconcile_ledger` clears it.
///
/// Compare-and-set is the smallest fix that keeps both writers independent.
/// Returns `false` when the row moved underneath the caller, which is not an
/// error: the other writer's view was the newer one, and the next tick re-reads
/// everything from storage anyway (F-060-13).
///
/// # Errors
///
/// Returns [`StorageError`] if the write fails.
pub async fn put_irrigation_state_if_unchanged(
    db: &EdgeDb,
    plant_id: &str,
    expected: &str,
    state: &IrrigationStateRow,
    now: i64,
) -> Result<bool, StorageError> {
    // One statement, not a read-modify-write transaction. SQLite serialises
    // writers even under WAL, and the control loop runs this once per plant per
    // tick — beside a telemetry pipeline and an API on the same file. A
    // `BEGIN; SELECT; UPSERT; COMMIT` holds the write lock across two round
    // trips and pushed the other two into `SQLITE_BUSY` past their five-second
    // timeout under the M8 overlay's accelerated clock, which surfaced as 500s
    // from the REST API. `ON CONFLICT ... DO UPDATE ... WHERE` is the same
    // compare-and-set in a single implicit transaction.
    //
    // `rows_affected() == 0` means the guard failed: the row exists and its
    // state is no longer `expected`.
    let affected = sqlx::query(
        "INSERT INTO irrigation_state(plant_id,state,state_since,doses_this_cycle,cycle_started_at,last_cycle_completed_at,wait_until,active_command_id,pre_dose_vwc,pre_dose_grams,updated_at) \
         VALUES(?,?,?,?,?,?,?,?,?,?,?) \
         ON CONFLICT(plant_id) DO UPDATE SET state=excluded.state,state_since=excluded.state_since,doses_this_cycle=excluded.doses_this_cycle,cycle_started_at=excluded.cycle_started_at,last_cycle_completed_at=excluded.last_cycle_completed_at,wait_until=excluded.wait_until,active_command_id=excluded.active_command_id,pre_dose_vwc=excluded.pre_dose_vwc,pre_dose_grams=excluded.pre_dose_grams,updated_at=excluded.updated_at \
         WHERE irrigation_state.state=?",
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
    .bind(expected)
    .execute(db.pool())
    .await
    .map_err(StorageError::from_sqlx)?;
    Ok(affected.rows_affected() > 0)
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
    crate::repo::outbox::emit(&mut tx,crate::repo::outbox::EventKind::COMMAND_ISSUED,&serde_json::json!({"command_id":command.command_id,"device_id":command.device_id,"plant_id":command.plant_id,"kind":command.kind,"requested_ml":command.requested_ml,"mode":command.mode,"issued_at":command.issued_at}),now).await?;
    if command.kind == "water" {
        crate::repo::outbox::emit(&mut tx,crate::repo::outbox::EventKind::WATERING_STARTED,&serde_json::json!({"watering_event_id":command.command_id,"command_id":command.command_id,"device_id":command.device_id,"plant_id":command.plant_id,"requested_ml":command.requested_ml,"mode":command.mode,"started_at":command.issued_at}),now).await?;
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
    extras: &SettleExtras<'_>,
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
        crate::repo::outbox::emit(&mut tx,crate::repo::outbox::EventKind::WATERING_COMPLETED,&serde_json::json!({"watering_event_id":event.watering_event_id,"command_id":command_id,"device_id":event.device_id,"plant_id":event.plant_id,"mode":event.mode,"started_at":event.started_at,"completed_at":event.completed_at,"requested_ml":event.requested_ml,"delivered_ml":event.delivered_ml}),now).await?;
    }

    if let Some((plant_id, state)) = next_state {
        put_state_in_tx(&mut tx, plant_id, state, now).await?;
    }

    // Both extras belong to *this* transaction, not to a follow-up statement.
    // A settle that wrote `state='locked'` and then failed before its lockout
    // reason landed would leave a plant held for a reason nothing records, and
    // an unrecorded transition would leave "what did the system think, and
    // when" unanswerable for the one transition most worth reconstructing.
    if let Some(hold) = extras.hold.as_ref() {
        crate::repo::lockout::hold_in_tx(&mut tx, hold.plant_id, hold.reason, hold.until, now)
            .await?;
    }
    if let Some(transition) = extras.transition.as_ref() {
        record_transition_in_tx(
            &mut tx,
            transition.plant_id,
            transition.from,
            transition.to,
            now,
        )
        .await?;
    }

    crate::repo::outbox::emit(&mut tx,crate::repo::outbox::EventKind::COMMAND_SETTLED,&serde_json::json!({"command_id":command_id,"status":status,"reason":reason,"occurred_at":now}),now).await?;

    tx.commit().await.map_err(StorageError::from_sqlx)?;
    Ok(true)
}

/// A hold to record in the settling transaction (`clock_unsynced`). A record
/// of its own in `plant_lockouts`, so it can never replace a latch.
#[derive(Clone, Copy, Debug)]
pub struct SettleHold<'a> {
    /// The plant to hold.
    pub plant_id: &'a str,
    /// The hold reason; one of `repo::lockout::HOLD_REASONS`.
    pub reason: &'a str,
    /// The deadline it is held to regardless of its condition.
    pub until: i64,
}

/// An irrigation transition to record in the settling transaction.
#[derive(Clone, Copy, Debug)]
pub struct SettleTransition<'a> {
    /// The plant that moved.
    pub plant_id: &'a str,
    /// The state it left.
    pub from: &'a str,
    /// The state it entered.
    pub to: &'a str,
}

/// Consequences a settlement must commit atomically with itself.
///
/// Empty for every settlement that has none, which is most of them.
#[derive(Clone, Copy, Debug, Default)]
pub struct SettleExtras<'a> {
    /// A hold the result implies, such as a device's `clock_unsynced`.
    pub hold: Option<SettleHold<'a>>,
    /// The irrigation transition the new state represents.
    pub transition: Option<SettleTransition<'a>>,
}

/// Records an irrigation transition inside an open transaction.
async fn record_transition_in_tx(
    tx: &mut Transaction<'_, Sqlite>,
    plant_id: &str,
    from: &str,
    to: &str,
    at: i64,
) -> Result<(), StorageError> {
    sqlx::query(
        "INSERT INTO plant_events(event_id,plant_id,kind,severity,detail_json,occurred_at) \
         VALUES(?,?,'irrigation_state_changed','info',?,?) ON CONFLICT(event_id) DO NOTHING",
    )
    .bind(format!("irrigation:{plant_id}:{at}:{to}"))
    .bind(plant_id)
    .bind(serde_json::json!({ "from": from, "to": to }).to_string())
    .bind(at)
    .execute(&mut **tx)
    .await
    .map_err(StorageError::from_sqlx)?;
    Ok(())
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

/// Writes a plant's **display summary** — the one reason a UI shows — with its
/// audit fields. Nothing here decides anything: the gate reads
/// [`crate::repo::lockout::active`], never this column.
///
/// # Events, without duplicating the records
///
/// A recorded lockout's own transitions are already in `plant_events` and the
/// outbox, written by [`crate::repo::lockout`] when the latch or hold changed.
/// The summary therefore records only what no record covers — a **derived**
/// block (stale data, tank low, a silent leak sensor, ...) appearing or ending,
/// and a hold that ended by expiring — so `lockout_set` / `lockout_cleared`
/// still count every block once, and a deployment with the cloud disabled
/// still keeps that history locally. One row per summary change, as before:
/// `lockout_set` (naming any reason it replaced) when the new reason is derived,
/// otherwise `lockout_cleared` when the old one was derived or an expired hold.
///
/// Read before write inside the transaction, and written only when the summary
/// changed, so a steady state produces no rows.
///
/// # Errors
///
/// A database failure.
pub async fn set_summary(
    db: &EdgeDb,
    plant_id: &str,
    reason: Option<&str>,
    cleared_by: Option<&str>,
    now: i64,
) -> Result<bool, StorageError> {
    let mut tx = db.begin().await?;
    // `None` here means no live plant row, which is not a transition.
    let previous: Option<Option<String>> = sqlx::query_scalar(
        "SELECT lockout_reason FROM plants WHERE plant_id=? AND deleted_at IS NULL",
    )
    .bind(plant_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(StorageError::from_sqlx)?;
    let Some(prior) = previous.filter(|p| p.as_deref() != reason) else {
        return Ok(false);
    };
    sqlx::query(
        "UPDATE plants SET lockout_reason=?,lockout_since=?,lockout_until=NULL,lockout_cleared_by=?,lockout_cleared_at=? \
         WHERE plant_id=? AND deleted_at IS NULL",
    )
    .bind(reason)
    .bind(reason.map(|_| now))
    .bind(if reason.is_none() { cleared_by } else { None })
    .bind(if reason.is_none() { Some(now) } else { None })
    .bind(plant_id)
    .execute(&mut *tx)
    .await
    .map_err(StorageError::from_sqlx)?;

    let recorded_latch = |name: &str| {
        crate::repo::lockout::classify_legacy(name, None)
            == Some(crate::repo::lockout::LockoutKind::Latch)
    };
    let new_is_derived = match reason {
        Some(name) => {
            !recorded_latch(name)
                && !crate::repo::lockout::hold_is_active_in_tx(&mut tx, plant_id, name, now).await?
        }
        None => false,
    };
    let prior_ended_unrecorded = match prior.as_deref() {
        Some(name) => {
            !recorded_latch(name)
                && !crate::repo::lockout::hold_is_active_in_tx(&mut tx, plant_id, name, now).await?
        }
        None => false,
    };
    if new_is_derived || prior_ended_unrecorded {
        let (kind, severity, shown) = if new_is_derived {
            ("lockout_set", "warning", reason)
        } else {
            ("lockout_cleared", "info", None)
        };
        let detail = serde_json::json!({
            "reason": shown,
            "cleared_reason": prior,
            "since": shown.map(|_| now),
            "cleared_by": if shown.is_none() { cleared_by } else { None },
        })
        .to_string();
        let event_id = format!("lockout:{plant_id}:{now}:{}", shown.unwrap_or("cleared"));
        sqlx::query(
            "INSERT INTO plant_events(event_id,plant_id,kind,severity,detail_json,occurred_at) \
             VALUES(?,?,?,?,?,?) ON CONFLICT(event_id) DO NOTHING",
        )
        .bind(event_id)
        .bind(plant_id)
        .bind(kind)
        .bind(severity)
        .bind(detail)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(StorageError::from_sqlx)?;
        let event = if new_is_derived {
            crate::repo::outbox::EventKind::LOCKOUT_SET
        } else {
            crate::repo::outbox::EventKind::LOCKOUT_CLEARED
        };
        crate::repo::outbox::emit(&mut tx, event, &serde_json::json!({"plant_id":plant_id,"reason":shown,"cleared_reason":prior,"since":shown.map(|_| now),"hold_until":null,"cleared_by":if shown.is_none() { cleared_by } else { None }}), now).await?;
    }
    tx.commit().await.map_err(StorageError::from_sqlx)?;
    Ok(true)
}

/// A plant's display summary as stored: reason, since, and the legacy
/// held-until column (always `NULL` since the per-reason cutover). For
/// display and change detection only; never a watering input.
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
mod cas {
    use super::*;
    use crate::EdgeDb;

    async fn db() -> EdgeDb {
        let db = EdgeDb::in_memory().await.unwrap();
        db.migrate().await.unwrap();
        sqlx::query("INSERT INTO plants(plant_id,name,created_at) VALUES('p','P',0)")
            .execute(db.pool())
            .await
            .unwrap();
        db
    }

    fn row(state: &str) -> IrrigationStateRow {
        IrrigationStateRow {
            state: state.to_owned(),
            ..IrrigationStateRow::default()
        }
    }

    /// The first writer inserts; the second, holding a stale view, is refused.
    ///
    /// This is the lost update the control pass and the result pipeline can
    /// produce between them: a settled dose moves the plant on, and the pass —
    /// which read the older state — would otherwise write it back and hold the
    /// plant against a command that has already completed.
    #[tokio::test]
    async fn a_stale_writer_is_refused_and_the_row_is_left_alone() {
        let db = db().await;

        assert!(
            put_irrigation_state_if_unchanged(&db, "p", "normal", &row("dose_issued"), 1)
                .await
                .unwrap(),
            "a plant with no row yet reads as the caller's default"
        );

        // The other writer settles the command and moves the plant on.
        put_irrigation_state(&db, "p", &row("wait_for_absorption"), 2)
            .await
            .unwrap();

        assert!(
            !put_irrigation_state_if_unchanged(&db, "p", "dose_issued", &row("dose_issued"), 3)
                .await
                .unwrap(),
            "a writer whose read is stale must not win"
        );
        assert_eq!(
            irrigation_state(&db, "p").await.unwrap().unwrap().state,
            "wait_for_absorption",
            "the newer writer's state survives"
        );

        assert!(
            put_irrigation_state_if_unchanged(&db, "p", "wait_for_absorption", &row("recheck"), 4)
                .await
                .unwrap(),
            "a writer with a current read still wins"
        );
        assert_eq!(
            irrigation_state(&db, "p").await.unwrap().unwrap().state,
            "recheck"
        );
    }
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
            settle(
                &db,
                "cmd-1",
                "completed",
                None,
                Some(&event),
                None,
                &SettleExtras::default(),
                5_000
            )
            .await
            .unwrap()
        );
        assert!(
            !settle(
                &db,
                "cmd-1",
                "failed",
                Some("late"),
                None,
                None,
                &SettleExtras::default(),
                9_000
            )
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
            !settle(
                &db,
                "never-issued",
                "completed",
                None,
                None,
                None,
                &SettleExtras::default(),
                1
            )
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
        settle(
            &db,
            "cmd-1",
            "completed",
            None,
            Some(&event),
            None,
            &SettleExtras::default(),
            5_000,
        )
        .await
        .unwrap();
        assert!((delivered_in_window(&db, "monstera-01", 0).await.unwrap() - 38.0).abs() < 1e-9);

        let mut second = command();
        second.command_id = "cmd-2".into();
        issue(&db, &second, &dose_issued(), 6_000).await.unwrap();
        settle(
            &db,
            "cmd-2",
            "interrupted",
            None,
            None,
            None,
            &SettleExtras::default(),
            7_000,
        )
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
            &SettleExtras::default(),
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
    async fn the_summary_round_trips_with_its_audit_fields() {
        let db = db().await;
        assert!(
            set_summary(&db, "monstera-01", Some("leak"), None, 10)
                .await
                .unwrap()
        );
        assert_eq!(
            lockout(&db, "monstera-01").await.unwrap(),
            Some(("leak".to_owned(), Some(10), None))
        );
        assert!(
            !set_summary(&db, "monstera-01", Some("leak"), None, 15)
                .await
                .unwrap()
        );
        assert!(
            set_summary(&db, "monstera-01", None, Some("operator"), 20)
                .await
                .unwrap()
        );
        assert_eq!(lockout(&db, "monstera-01").await.unwrap(), None);
        let cleared: Option<String> = sqlx::query_scalar(
            "SELECT lockout_cleared_by FROM plants WHERE plant_id='monstera-01'",
        )
        .fetch_one(db.pool())
        .await
        .unwrap();
        assert_eq!(cleared.as_deref(), Some("operator"));
        // A summary is a display: it records and removes no lockout.
        let records: i64 = sqlx::query_scalar("SELECT count(*) FROM plant_lockouts")
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert_eq!(records, 0);
    }

    /// Plant-scoped lockout events, oldest first.
    async fn lockout_events(db: &EdgeDb) -> Vec<(String, String, String)> {
        sqlx::query(
            "SELECT kind,severity,detail_json FROM plant_events \
             WHERE plant_id='monstera-01' AND kind IN('lockout_set','lockout_cleared') \
             ORDER BY occurred_at, event_id",
        )
        .fetch_all(db.pool())
        .await
        .unwrap()
        .into_iter()
        .map(|r| {
            (
                r.get::<String, _>("kind"),
                r.get::<String, _>("severity"),
                r.get::<Option<String>, _>("detail_json")
                    .unwrap_or_default(),
            )
        })
        .collect()
    }

    /// The gap this record closes: with cloud sync off — the default — `emit`
    /// writes nothing, so without it a derived block left no trace once the
    /// summary moved on.
    #[tokio::test]
    async fn a_derived_block_is_durable_locally_with_the_cloud_disabled() {
        // Deliberately not the shared `db()` helper: that one enables cloud
        // sync, and the whole point here is the default local-first
        // configuration, where `emit` writes nothing at all.
        let db = EdgeDb::in_memory().await.unwrap();
        db.migrate().await.unwrap();
        sqlx::query(
            "INSERT INTO plants(plant_id,name,created_at) VALUES('monstera-01','Monstera',0)",
        )
        .execute(db.pool())
        .await
        .unwrap();

        set_summary(&db, "monstera-01", Some("tank_low"), None, 10)
            .await
            .unwrap();
        set_summary(&db, "monstera-01", None, Some("auto"), 20)
            .await
            .unwrap();

        let outbox: i64 = sqlx::query_scalar("SELECT count(*) FROM pending_cloud_events")
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert_eq!(outbox, 0, "cloud sync is disabled, so nothing was emitted");

        let events = lockout_events(&db).await;
        assert_eq!(events.len(), 2, "both transitions are recorded locally");
        assert_eq!(events[0].0, "lockout_set");
        assert_eq!(events[0].1, "warning");
        assert_eq!(events[1].0, "lockout_cleared");
        assert_eq!(events[1].1, "info");
        let cleared: serde_json::Value = serde_json::from_str(&events[1].2).unwrap();
        assert_eq!(cleared["cleared_reason"], "tank_low");
        assert_eq!(cleared["cleared_by"], "auto");
        assert!(cleared["reason"].is_null());
    }

    /// A recorded lockout's transitions are written by its own record, so the
    /// summary showing or dropping it adds nothing: one block, one row.
    #[tokio::test]
    async fn a_recorded_lockout_is_not_counted_twice() {
        let db = db().await;
        crate::repo::lockout::latch(&db, "monstera-01", "leak", 10)
            .await
            .unwrap();
        crate::repo::lockout::hold(&db, "monstera-01", "clock_unsynced", 500, 10)
            .await
            .unwrap();
        set_summary(&db, "monstera-01", Some("leak"), None, 10)
            .await
            .unwrap();
        set_summary(&db, "monstera-01", Some("clock_unsynced"), None, 20)
            .await
            .unwrap();
        assert_eq!(lockout_events(&db).await.len(), 2, "the two records only");

        // The hold expires and the summary moves off it: that end is recorded
        // by nobody else, so the summary records it.
        set_summary(&db, "monstera-01", None, Some("auto"), 600)
            .await
            .unwrap();
        let events = lockout_events(&db).await;
        assert_eq!(events.len(), 3);
        let ended: serde_json::Value = serde_json::from_str(&events[2].2).unwrap();
        assert_eq!(events[2].0, "lockout_cleared");
        assert_eq!(ended["cleared_reason"], "clock_unsynced");
    }

    /// A derived block re-asserted on every pass records one event, and its
    /// replacement by a recorded latch records the derived block's end.
    #[tokio::test]
    async fn re_asserting_the_same_summary_records_one_event() {
        let db = db().await;
        for at in [10, 20, 30] {
            set_summary(&db, "monstera-01", Some("uncertain"), None, at)
                .await
                .unwrap();
        }
        assert_eq!(lockout_events(&db).await.len(), 1);

        set_summary(&db, "monstera-01", Some("leak"), None, 40)
            .await
            .unwrap();
        let events = lockout_events(&db).await;
        assert_eq!(events.len(), 2);
        let second: serde_json::Value = serde_json::from_str(&events[1].2).unwrap();
        assert_eq!(events[1].0, "lockout_cleared");
        assert_eq!(second["cleared_reason"], "uncertain");
    }

    /// Clearing a plant that shows nothing changes nothing, so it is not a
    /// transition and must not manufacture an event.
    #[tokio::test]
    async fn clearing_an_unlocked_plant_records_nothing() {
        let db = db().await;
        assert!(
            !set_summary(&db, "monstera-01", None, Some("operator"), 10)
                .await
                .unwrap()
        );
        assert!(lockout_events(&db).await.is_empty());
    }

    /// An unknown or soft-deleted plant updates no row, so there is no
    /// transition to record.
    #[tokio::test]
    async fn an_unknown_plant_records_nothing() {
        let db = db().await;
        assert!(
            !set_summary(&db, "no-such-plant", Some("tank_low"), None, 10)
                .await
                .unwrap()
        );
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM plant_events WHERE kind IN('lockout_set','lockout_cleared')",
        )
        .fetch_one(db.pool())
        .await
        .unwrap();
        assert_eq!(count, 0);
    }

    /// The history is what a later operational view derives from: how many
    /// blocks, of which kind, and how long each lasted.
    #[tokio::test]
    async fn a_block_and_its_time_to_clear_are_derivable_from_the_history() {
        let db = db().await;
        set_summary(&db, "monstera-01", Some("stale_data"), None, 1_000)
            .await
            .unwrap();
        set_summary(&db, "monstera-01", None, Some("auto"), 61_000)
            .await
            .unwrap();

        let rows = sqlx::query(
            "SELECT kind,occurred_at FROM plant_events \
             WHERE plant_id='monstera-01' AND kind IN('lockout_set','lockout_cleared') \
             ORDER BY occurred_at",
        )
        .fetch_all(db.pool())
        .await
        .unwrap();
        let set_at: i64 = rows[0].get("occurred_at");
        let cleared_at: i64 = rows[1].get("occurred_at");
        assert_eq!(cleared_at - set_at, 60_000, "one minute blocked");
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
