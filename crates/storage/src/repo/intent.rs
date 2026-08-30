//! Durable command intents for sleeping devices (M6-022, ADR-018 §3).
//!
//! # An intent is not a command, and the distinction is the safety argument
//!
//! A battery device is listening for a few seconds out of every fifteen minutes.
//! Publishing to it immediately would deliver a command whose 120-second TTL
//! expired while it slept — fail-closed, and therefore not dangerous, but it
//! would mean manual watering on a battery device simply never works.
//!
//! So the edge persists what the operator asked for and mints the command at the
//! next wake. **No `command_id` exists until then**, which is why nothing in
//! SAFETY-001 or SAFETY-010 changes: there is still exactly one
//! persist-before-publish moment per command, a delivery retry still reuses the
//! id allocated at that moment, and `commands` gains no column. The reviewer's
//! check is the one visible here: `command_intents.command_id` is **nullable**.
//!
//! # Two clocks, deliberately separated
//!
//! `intent_expires_at` is the **edge's** clock and is an operator-facing bound on
//! how long the edge will keep trying. The wire TTL is unchanged at 120 s and is
//! what the *device* validates against its own synchronised clock (SAFETY-002).
//! They are not the same mechanism and must not be merged into one field;
//! `intent_expires_at` never reaches a device.
#![allow(
    missing_docs,
    reason = "row structs mirror table columns one for one; the columns are \n              documented in the migration and the module header, and repeating \n              them per field would drown the rules that matter"
)]

use sqlx::Row as _;

use crate::{EdgeDb, StorageError};

/// The lifecycle of an intent. `pending_for_device_wake` is the only open state.
pub const PENDING: &str = "pending_for_device_wake";
/// Delivered: a command was minted and published.
pub const SENT: &str = "sent";
/// The gate refused it at delivery.
pub const REFUSED: &str = "refused";
/// It reached `intent_expires_at` without a wake.
pub const EXPIRED: &str = "expired_before_wake";

/// The floor on how long an intent is held, whatever the wake interval.
pub const MIN_INTENT_TTL_MS: i64 = 30 * 60 * 1_000;

/// An intent as stored.
#[derive(Clone, Debug, PartialEq)]
pub struct IntentRow {
    pub intent_id: String,
    pub plant_id: String,
    pub device_id: String,
    pub kind: String,
    pub requested_ml: f64,
    pub mode: String,
    pub created_at: i64,
    pub intent_expires_at: i64,
    pub expected_delivery_after: Option<i64>,
    pub state: String,
    /// **Null until delivery.** The whole safety argument in one column.
    pub command_id: Option<String>,
    pub refusal_reason: Option<String>,
    pub settled_at: Option<i64>,
}

/// An intent as a caller supplies it.
#[derive(Clone, Debug, PartialEq)]
pub struct NewIntent {
    pub intent_id: String,
    pub plant_id: String,
    pub device_id: String,
    pub kind: String,
    pub requested_ml: f64,
    pub mode: String,
    pub created_at: i64,
    pub intent_expires_at: i64,
    pub expected_delivery_after: Option<i64>,
}

/// `2 x wake_interval_seconds`, with a thirty-minute floor.
///
/// Two wakes rather than one, because a single missed wake is ordinary and an
/// intent that expired on it would be indistinguishable from a broken feature.
#[must_use]
pub fn intent_ttl_ms(wake_interval_seconds: Option<i64>) -> i64 {
    wake_interval_seconds
        .map(|seconds| seconds.saturating_mul(2_000))
        .unwrap_or(MIN_INTENT_TTL_MS)
        .max(MIN_INTENT_TTL_MS)
}

fn row_to_intent(row: &sqlx::sqlite::SqliteRow) -> IntentRow {
    IntentRow {
        intent_id: row.get("intent_id"),
        plant_id: row.get("plant_id"),
        device_id: row.get("device_id"),
        kind: row.get("kind"),
        requested_ml: row.get("requested_ml"),
        mode: row.get("mode"),
        created_at: row.get("created_at"),
        intent_expires_at: row.get("intent_expires_at"),
        expected_delivery_after: row.get("expected_delivery_after"),
        state: row.get("state"),
        command_id: row.get("command_id"),
        refusal_reason: row.get("refusal_reason"),
        settled_at: row.get("settled_at"),
    }
}

/// Persists a pending intent.
///
/// # Errors
///
/// A second open water intent for the same plant violates the partial unique
/// index and is reported as [`StorageError::Constraint`]. That is the storage
/// half of "at most one open water intent per plant": the API's 409 is the
/// courteous answer, and this is the one that holds under a race.
pub async fn create(db: &EdgeDb, intent: &NewIntent, now: i64) -> Result<(), StorageError> {
    sqlx::query(
        "INSERT INTO command_intents(intent_id,plant_id,device_id,kind,requested_ml,mode,created_at,intent_expires_at,expected_delivery_after,state,updated_at) \
         VALUES(?,?,?,?,?,?,?,?,?,?,?)",
    )
    .bind(&intent.intent_id)
    .bind(&intent.plant_id)
    .bind(&intent.device_id)
    .bind(&intent.kind)
    .bind(intent.requested_ml)
    .bind(&intent.mode)
    .bind(intent.created_at)
    .bind(intent.intent_expires_at)
    .bind(intent.expected_delivery_after)
    .bind(PENDING)
    .bind(now)
    .execute(db.pool())
    .await
    .map_err(StorageError::from_sqlx)?;
    Ok(())
}

/// Reads one intent.
pub async fn get(db: &EdgeDb, intent_id: &str) -> Result<Option<IntentRow>, StorageError> {
    Ok(
        sqlx::query("SELECT * FROM command_intents WHERE intent_id=?")
            .bind(intent_id)
            .fetch_optional(db.pool())
            .await
            .map_err(StorageError::from_sqlx)?
            .as_ref()
            .map(row_to_intent),
    )
}

/// The open water intent for a plant, if there is one.
pub async fn open_for_plant(
    db: &EdgeDb,
    plant_id: &str,
) -> Result<Option<IntentRow>, StorageError> {
    Ok(sqlx::query(
        "SELECT * FROM command_intents WHERE plant_id=? AND kind='water' AND state=? LIMIT 1",
    )
    .bind(plant_id)
    .bind(PENDING)
    .fetch_optional(db.pool())
    .await
    .map_err(StorageError::from_sqlx)?
    .as_ref()
    .map(row_to_intent))
}

/// Every pending intent for a device, oldest first.
pub async fn pending_for_device(
    db: &EdgeDb,
    device_id: &str,
) -> Result<Vec<IntentRow>, StorageError> {
    let rows = sqlx::query(
        "SELECT * FROM command_intents WHERE device_id=? AND state=? ORDER BY created_at",
    )
    .bind(device_id)
    .bind(PENDING)
    .fetch_all(db.pool())
    .await
    .map_err(StorageError::from_sqlx)?;
    Ok(rows.iter().map(row_to_intent).collect())
}

/// Every pending intent, oldest first. Used by restart reconciliation.
pub async fn all_pending(db: &EdgeDb) -> Result<Vec<IntentRow>, StorageError> {
    let rows = sqlx::query("SELECT * FROM command_intents WHERE state=? ORDER BY created_at")
        .bind(PENDING)
        .fetch_all(db.pool())
        .await
        .map_err(StorageError::from_sqlx)?;
    Ok(rows.iter().map(row_to_intent).collect())
}

/// Marks an intent delivered, recording the `command_id` allocated at the wake.
///
/// Conditional on the intent still being pending, so a delivery that races an
/// expiry sweep settles once. Returns `false` if it had already left `pending`.
pub async fn mark_sent(
    db: &EdgeDb,
    intent_id: &str,
    command_id: &str,
    now: i64,
) -> Result<bool, StorageError> {
    let done = sqlx::query(
        "UPDATE command_intents SET state=?,command_id=?,settled_at=?,updated_at=? WHERE intent_id=? AND state=?",
    )
    .bind(SENT)
    .bind(command_id)
    .bind(now)
    .bind(now)
    .bind(intent_id)
    .bind(PENDING)
    .execute(db.pool())
    .await
    .map_err(StorageError::from_sqlx)?;
    Ok(done.rows_affected() == 1)
}

/// Marks an intent refused by the gate at delivery.
pub async fn mark_refused(
    db: &EdgeDb,
    intent_id: &str,
    reason: &str,
    now: i64,
) -> Result<bool, StorageError> {
    let done = sqlx::query(
        "UPDATE command_intents SET state=?,refusal_reason=?,settled_at=?,updated_at=? WHERE intent_id=? AND state=?",
    )
    .bind(REFUSED)
    .bind(reason)
    .bind(now)
    .bind(now)
    .bind(intent_id)
    .bind(PENDING)
    .execute(db.pool())
    .await
    .map_err(StorageError::from_sqlx)?;
    Ok(done.rows_affected() == 1)
}

/// Expires every pending intent past its deadline, returning how many.
pub async fn sweep_expired(db: &EdgeDb, now: i64) -> Result<u64, StorageError> {
    let done = sqlx::query(
        "UPDATE command_intents SET state=?,settled_at=?,updated_at=? WHERE state=? AND intent_expires_at<=?",
    )
    .bind(EXPIRED)
    .bind(now)
    .bind(now)
    .bind(PENDING)
    .bind(now)
    .execute(db.pool())
    .await
    .map_err(StorageError::from_sqlx)?;
    Ok(done.rows_affected())
}

/// How many intents are open, for the gauge.
pub async fn pending_count(db: &EdgeDb) -> Result<i64, StorageError> {
    sqlx::query_scalar("SELECT count(*) FROM command_intents WHERE state=?")
        .bind(PENDING)
        .fetch_one(db.pool())
        .await
        .map_err(StorageError::from_sqlx)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod intents {
    use super::*;

    async fn db() -> EdgeDb {
        let db = EdgeDb::in_memory().await.unwrap();
        db.migrate().await.unwrap();
        for plant in ["monstera-01", "fern-01"] {
            sqlx::query("INSERT INTO plants(plant_id,name,created_at) VALUES(?,?,0)")
                .bind(plant)
                .bind(plant)
                .execute(db.pool())
                .await
                .unwrap();
        }
        // `command_intents.command_id` really references `commands`, so a
        // delivery test needs a command to point at. That the foreign key bites
        // is itself the point: an intent may only ever name a command the edge
        // actually minted.
        for id in ["cmd-1", "cmd-2"] {
            sqlx::query(
                "INSERT INTO commands(command_id,device_id,plant_id,kind,requested_ml,mode,issued_at,expires_at,status)                  VALUES(?,'plant-node-01','monstera-01','water',30.0,'manual',900000,900120,'issued')",
            )
            .bind(id)
            .execute(db.pool())
            .await
            .unwrap();
        }
        db
    }

    fn intent(id: &str, plant: &str) -> NewIntent {
        NewIntent {
            intent_id: id.into(),
            plant_id: plant.into(),
            device_id: "plant-node-01".into(),
            kind: "water".into(),
            requested_ml: 30.0,
            mode: "manual".into(),
            created_at: 1_000,
            intent_expires_at: 1_000 + MIN_INTENT_TTL_MS,
            expected_delivery_after: Some(900_000),
        }
    }

    #[tokio::test]
    async fn a_pending_intent_carries_no_command_id() {
        let db = db().await;
        create(&db, &intent("i-1", "monstera-01"), 1_000)
            .await
            .unwrap();
        let row = get(&db, "i-1").await.unwrap().unwrap();
        assert_eq!(row.state, PENDING);
        assert_eq!(
            row.command_id, None,
            "no command exists until the device is awake"
        );
    }

    #[tokio::test]
    async fn at_most_one_open_water_intent_per_plant() {
        let db = db().await;
        create(&db, &intent("i-1", "monstera-01"), 1_000)
            .await
            .unwrap();
        let error = create(&db, &intent("i-2", "monstera-01"), 1_100)
            .await
            .unwrap_err();
        assert!(matches!(error, StorageError::Constraint(_)), "{error:?}");

        // ...but a different plant is unaffected.
        create(&db, &intent("i-3", "fern-01"), 1_200).await.unwrap();
        assert!(open_for_plant(&db, "fern-01").await.unwrap().is_some());

        // ...and once the first is settled, a second is allowed.
        mark_sent(&db, "i-1", "cmd-1", 2_000).await.unwrap();
        create(&db, &intent("i-4", "monstera-01"), 2_100)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn delivery_records_the_command_id_allocated_at_the_wake() {
        let db = db().await;
        create(&db, &intent("i-1", "monstera-01"), 1_000)
            .await
            .unwrap();
        assert!(mark_sent(&db, "i-1", "cmd-1", 900_000).await.unwrap());
        let row = get(&db, "i-1").await.unwrap().unwrap();
        assert_eq!(row.state, SENT);
        assert_eq!(row.command_id.as_deref(), Some("cmd-1"));
        assert_eq!(row.settled_at, Some(900_000));
        assert!(
            !mark_sent(&db, "i-1", "cmd-2", 900_100).await.unwrap(),
            "a terminal intent never leaves its state"
        );
    }

    #[tokio::test]
    async fn a_refusal_is_terminal_and_names_its_reason() {
        let db = db().await;
        create(&db, &intent("i-1", "monstera-01"), 1_000)
            .await
            .unwrap();
        assert!(mark_refused(&db, "i-1", "leak", 900_000).await.unwrap());
        let row = get(&db, "i-1").await.unwrap().unwrap();
        assert_eq!(row.state, REFUSED);
        assert_eq!(row.refusal_reason.as_deref(), Some("leak"));
        assert_eq!(row.command_id, None, "a refused intent mints no command");
        assert!(!mark_sent(&db, "i-1", "cmd-1", 900_100).await.unwrap());
    }

    #[tokio::test]
    async fn the_sweep_expires_only_what_is_past_its_deadline() {
        let db = db().await;
        create(&db, &intent("i-1", "monstera-01"), 1_000)
            .await
            .unwrap();
        assert_eq!(sweep_expired(&db, 1_000).await.unwrap(), 0);
        assert_eq!(pending_count(&db).await.unwrap(), 1);
        assert_eq!(
            sweep_expired(&db, 1_000 + MIN_INTENT_TTL_MS).await.unwrap(),
            1
        );
        assert_eq!(get(&db, "i-1").await.unwrap().unwrap().state, EXPIRED);
        assert_eq!(pending_count(&db).await.unwrap(), 0);
        assert!(
            !mark_sent(&db, "i-1", "cmd-1", 9_000_000).await.unwrap(),
            "an expired intent is never delivered"
        );
    }

    #[tokio::test]
    async fn the_ttl_is_two_wakes_with_a_half_hour_floor() {
        assert_eq!(intent_ttl_ms(Some(900)), MIN_INTENT_TTL_MS);
        assert_eq!(intent_ttl_ms(Some(3_600)), 2 * 3_600 * 1_000);
        assert_eq!(intent_ttl_ms(Some(60)), MIN_INTENT_TTL_MS);
        assert_eq!(intent_ttl_ms(None), MIN_INTENT_TTL_MS);
    }

    #[tokio::test]
    async fn pending_intents_survive_to_be_read_back_after_a_restart() {
        let db = db().await;
        create(&db, &intent("i-1", "monstera-01"), 1_000)
            .await
            .unwrap();
        create(&db, &intent("i-2", "fern-01"), 1_100).await.unwrap();
        // A "restart" is just another read of the same durable rows.
        let pending = all_pending(&db).await.unwrap();
        assert_eq!(pending.len(), 2);
        assert_eq!(pending[0].intent_id, "i-1");
        assert_eq!(
            pending_for_device(&db, "plant-node-01")
                .await
                .unwrap()
                .len(),
            2
        );
    }
}
