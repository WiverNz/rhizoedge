//! Plant rows and the durable state the plant tick keeps (M5-001, M5-006,
//! M5-007, M5-008, M5-010, M5-012, M5-015).
//!
//! Transactions, not decisions (ADR-001 boundary rule 3). Nothing here decides
//! whether a plant should be watered; it stores what something else concluded.
//!
//! # Two invariants live at this layer on purpose
//!
//! **`auto_watering_enabled` is forced to `false` on insert**, in the storage
//! layer rather than only in the API. A plant created by any path — HTTP, a
//! preset, a future import — must be inert until a human opts in (SAFETY-012,
//! F-050-01). A default in one caller is a default one other caller can forget.
//!
//! **Deleting a plant preserves its `watering_events`.** The ledger is the
//! record of what the machine did to a living thing, and it outlives the row
//! that pointed at it. The delete is therefore a soft delete: the row stays,
//! `deleted_at` is set, and every read filters on it. Nullifying the reference
//! would keep the rows but lose which plant they belonged to.
#![allow(missing_docs)]
use sqlx::Row as _;

use crate::{EdgeDb, StorageError};

/// A plant as stored.
#[derive(Clone, Debug, PartialEq)]
pub struct PlantRow {
    pub plant_id: String,
    pub name: String,
    pub species: Option<String>,
    pub profile_id: Option<String>,
    pub pot_volume_ml: Option<f64>,
    pub soil_type: Option<String>,
    pub auto_watering_enabled: bool,
    pub lockout_reason: Option<String>,
    pub lockout_since: Option<i64>,
    pub applied_preset_id: Option<String>,
    pub applied_catalogue_version: Option<i64>,
    pub created_at: i64,
}

/// A plant as supplied by a caller. Note the absence of an
/// `auto_watering_enabled` field: it is not a caller's to set at creation.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NewPlant {
    pub plant_id: String,
    pub name: String,
    pub species: Option<String>,
    pub profile_id: Option<String>,
    pub pot_volume_ml: Option<f64>,
    pub soil_type: Option<String>,
}

/// A partial update. `None` leaves a field alone; `Some(None)` clears it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlantPatch {
    pub name: Option<String>,
    pub species: Option<Option<String>>,
    pub profile_id: Option<Option<String>>,
    pub pot_volume_ml: Option<Option<f64>>,
    pub soil_type: Option<Option<String>>,
    pub auto_watering_enabled: Option<bool>,
}

fn row_to_plant(row: &sqlx::sqlite::SqliteRow) -> PlantRow {
    PlantRow {
        plant_id: row.get("plant_id"),
        name: row.get("name"),
        species: row.get("species"),
        profile_id: row.get("profile_id"),
        pot_volume_ml: row.get("pot_volume_ml"),
        soil_type: row.get("soil_type"),
        auto_watering_enabled: row.get::<i64, _>("auto_watering_enabled") != 0,
        lockout_reason: row.get("lockout_reason"),
        lockout_since: row.get("lockout_since"),
        applied_preset_id: row.get("applied_preset_id"),
        applied_catalogue_version: row.get("applied_catalogue_version"),
        created_at: row.get("created_at"),
    }
}

/// Inserts a plant. Automation is off, whatever the caller wanted.
pub async fn create(db: &EdgeDb, plant: &NewPlant, now: i64) -> Result<PlantRow, StorageError> {
    let mut tx = db.begin().await?;
    sqlx::query(
        "INSERT INTO plants(plant_id,profile_id,name,species,pot_volume_ml,soil_type,auto_watering_enabled,created_at) \
         VALUES(?,?,?,?,?,?,0,?)",
    )
    .bind(&plant.plant_id)
    .bind(&plant.profile_id)
    .bind(&plant.name)
    .bind(&plant.species)
    .bind(plant.pot_volume_ml)
    .bind(&plant.soil_type)
    .bind(now)
    .execute(&mut *tx)
    .await
    .map_err(StorageError::from_sqlx)?;
    crate::repo::outbox::emit(&mut tx, crate::repo::outbox::EventKind::PLANT_CREATED, &serde_json::json!({"plant_id":plant.plant_id,"name":plant.name,"species":plant.species,"profile_id":plant.profile_id,"pot_volume_ml":plant.pot_volume_ml,"soil_type":plant.soil_type}), now).await?;
    tx.commit().await.map_err(StorageError::from_sqlx)?;
    get(db, &plant.plant_id)
        .await?
        .ok_or_else(|| StorageError::Database("the plant vanished between insert and read".into()))
}

/// Reads one live plant. A soft-deleted plant is gone as far as callers go.
pub async fn get(db: &EdgeDb, plant_id: &str) -> Result<Option<PlantRow>, StorageError> {
    Ok(
        sqlx::query("SELECT * FROM plants WHERE plant_id=? AND deleted_at IS NULL")
            .bind(plant_id)
            .fetch_optional(db.pool())
            .await
            .map_err(StorageError::from_sqlx)?
            .as_ref()
            .map(row_to_plant),
    )
}

/// Lists live plants after `cursor`, in id order, at most `limit` of them.
///
/// Keyset pagination on the primary key: a stable cursor that does not shift
/// when a plant is added or removed, which an `OFFSET` would.
pub async fn list(
    db: &EdgeDb,
    cursor: Option<&str>,
    limit: i64,
) -> Result<Vec<PlantRow>, StorageError> {
    let rows = sqlx::query(
        "SELECT * FROM plants WHERE deleted_at IS NULL AND plant_id > ? ORDER BY plant_id LIMIT ?",
    )
    .bind(cursor.unwrap_or(""))
    .bind(limit.clamp(1, 500))
    .fetch_all(db.pool())
    .await
    .map_err(StorageError::from_sqlx)?;
    Ok(rows.iter().map(row_to_plant).collect())
}

/// Applies a partial update. Returns `None` for an unknown or deleted plant.
pub async fn update(
    db: &EdgeDb,
    plant_id: &str,
    patch: &PlantPatch,
    now: i64,
) -> Result<Option<PlantRow>, StorageError> {
    if get(db, plant_id).await?.is_none() {
        return Ok(None);
    }
    let mut tx = db.begin().await?;
    macro_rules! set {
        ($column:literal, $value:expr) => {
            if let Some(value) = $value {
                sqlx::query(concat!(
                    "UPDATE plants SET ",
                    $column,
                    "=? WHERE plant_id=?"
                ))
                .bind(value)
                .bind(plant_id)
                .execute(&mut *tx)
                .await
                .map_err(StorageError::from_sqlx)?;
            }
        };
    }
    set!("name", patch.name.clone());
    set!("species", patch.species.clone());
    set!("profile_id", patch.profile_id.clone());
    set!("pot_volume_ml", patch.pot_volume_ml);
    set!("soil_type", patch.soil_type.clone());
    set!("auto_watering_enabled", patch.auto_watering_enabled);
    crate::repo::outbox::emit(&mut tx, crate::repo::outbox::EventKind::PLANT_UPDATED, &serde_json::json!({"operation":"update","plant_id":plant_id,"patch":{"name":patch.name,"species":patch.species,"profile_id":patch.profile_id,"pot_volume_ml":patch.pot_volume_ml,"soil_type":patch.soil_type,"auto_watering_enabled":patch.auto_watering_enabled}}), now).await?;
    tx.commit().await.map_err(StorageError::from_sqlx)?;
    get(db, plant_id).await
}

/// Records the plant's provenance after a preset was applied (M5-018).
///
/// Written once, at application. Nothing reads these columns to decide anything.
pub async fn record_applied_preset(
    db: &EdgeDb,
    plant_id: &str,
    preset_id: &str,
    catalogue_version: u32,
) -> Result<(), StorageError> {
    sqlx::query(
        "UPDATE plants SET applied_preset_id=?,applied_catalogue_version=? WHERE plant_id=?",
    )
    .bind(preset_id)
    .bind(i64::from(catalogue_version))
    .bind(plant_id)
    .execute(db.pool())
    .await
    .map_err(StorageError::from_sqlx)?;
    Ok(())
}

/// Soft-deletes a plant, leaving its watering history intact and attributed.
///
/// The deletion is announced with the canonical `plant.updated`, carrying
/// `operation: "delete"` and the identity the plant had. ADR-005's catalogue has
/// no `plant.deleted`, and a plant that simply stopped producing events would be
/// indistinguishable in cloud history from one whose edge went quiet — the
/// deletion is a fact, and a fact is what an event is for.
///
/// A second delete of the same plant changes no row and emits nothing.
pub async fn delete(db: &EdgeDb, plant_id: &str, now: i64) -> Result<bool, StorageError> {
    let mut tx = db.begin().await?;
    let previous: Option<(Option<String>, Option<String>)> =
        sqlx::query_as("SELECT name,species FROM plants WHERE plant_id=? AND deleted_at IS NULL")
            .bind(plant_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(StorageError::from_sqlx)?;
    let Some((name, species)) = previous else {
        return Ok(false);
    };
    let done =
        sqlx::query("UPDATE plants SET deleted_at=? WHERE plant_id=? AND deleted_at IS NULL")
            .bind(now)
            .bind(plant_id)
            .execute(&mut *tx)
            .await
            .map_err(StorageError::from_sqlx)?;
    if done.rows_affected() != 1 {
        return Ok(false);
    }
    crate::repo::outbox::emit(&mut tx, crate::repo::outbox::EventKind::PLANT_UPDATED, &serde_json::json!({"operation":"delete","plant_id":plant_id,"name":name,"species":species,"deleted_at":now}), now).await?;
    tx.commit().await.map_err(StorageError::from_sqlx)?;
    Ok(true)
}

// ---------------------------------------------------------------- dry duration

/// The persisted dry-duration accumulator (M5-006).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DryStateRow {
    pub dry_ms: i64,
    pub last_sample_at: Option<i64>,
}

pub async fn dry_state(db: &EdgeDb, plant_id: &str) -> Result<DryStateRow, StorageError> {
    let row = sqlx::query("SELECT dry_ms,last_sample_at FROM plant_dry_state WHERE plant_id=?")
        .bind(plant_id)
        .fetch_optional(db.pool())
        .await
        .map_err(StorageError::from_sqlx)?;
    Ok(row.map_or_else(DryStateRow::default, |r| DryStateRow {
        dry_ms: r.get("dry_ms"),
        last_sample_at: r.get("last_sample_at"),
    }))
}

pub async fn put_dry_state(
    db: &EdgeDb,
    plant_id: &str,
    state: DryStateRow,
    now: i64,
) -> Result<(), StorageError> {
    sqlx::query(
        "INSERT INTO plant_dry_state(plant_id,dry_ms,last_sample_at,updated_at) VALUES(?,?,?,?) \
         ON CONFLICT(plant_id) DO UPDATE SET dry_ms=excluded.dry_ms,last_sample_at=excluded.last_sample_at,updated_at=excluded.updated_at",
    )
    .bind(plant_id)
    .bind(state.dry_ms)
    .bind(state.last_sample_at)
    .bind(now)
    .execute(db.pool())
    .await
    .map_err(StorageError::from_sqlx)?;
    Ok(())
}

// ----------------------------------------------------------------- plant state

/// The persisted operator-facing state (M5-010).
pub async fn plant_state(db: &EdgeDb, plant_id: &str) -> Result<Option<String>, StorageError> {
    sqlx::query_scalar::<_, String>("SELECT state FROM plant_state_current WHERE plant_id=?")
        .bind(plant_id)
        .fetch_optional(db.pool())
        .await
        .map_err(StorageError::from_sqlx)
}

/// Records a transition and the current state in one transaction.
///
/// Called only when the state actually changed: steady state writes nothing, so
/// a 30-second tick does not fill the event log with "still healthy".
pub async fn record_state_transition(
    db: &EdgeDb,
    plant_id: &str,
    from: Option<&str>,
    to: &str,
    now: i64,
) -> Result<(), StorageError> {
    let mut tx = db.begin().await?;
    sqlx::query(
        "INSERT INTO plant_state_current(plant_id,state,since,updated_at) VALUES(?,?,?,?) \
         ON CONFLICT(plant_id) DO UPDATE SET state=excluded.state,since=excluded.since,updated_at=excluded.updated_at",
    )
    .bind(plant_id)
    .bind(to)
    .bind(now)
    .bind(now)
    .execute(&mut *tx)
    .await
    .map_err(StorageError::from_sqlx)?;
    crate::repo::outbox::emit(
        &mut tx,
        crate::repo::outbox::EventKind::PLANT_STATE_CHANGED,
        &serde_json::json!({"plant_id":plant_id,"from":from,"to":to}),
        now,
    )
    .await?;
    let event_id = format!("plant:{plant_id}:state:{now}:{to}");
    let detail = serde_json::json!({ "from": from, "to": to }).to_string();
    sqlx::query(
        "INSERT INTO plant_events(event_id,plant_id,kind,severity,detail_json,occurred_at) \
         VALUES(?,?,'plant_state_changed','info',?,?) ON CONFLICT(event_id) DO NOTHING",
    )
    .bind(event_id)
    .bind(plant_id)
    .bind(&detail)
    .bind(now)
    .execute(&mut *tx)
    .await
    .map_err(StorageError::from_sqlx)?;
    tx.commit().await.map_err(StorageError::from_sqlx)?;
    Ok(())
}

/// Records a plant-scoped event, at most once per `event_id`.
pub async fn record_plant_event(
    db: &EdgeDb,
    plant_id: Option<&str>,
    event_id: &str,
    kind: &str,
    severity: &str,
    detail: Option<&serde_json::Value>,
    now: i64,
) -> Result<bool, StorageError> {
    let detail = detail.map(ToString::to_string);
    let mut tx = db.begin().await?;
    let done = sqlx::query(
        "INSERT INTO plant_events(event_id,plant_id,kind,severity,detail_json,occurred_at) \
         VALUES(?,?,?,?,?,?) ON CONFLICT(event_id) DO NOTHING",
    )
    .bind(event_id)
    .bind(plant_id)
    .bind(kind)
    .bind(severity)
    .bind(&detail)
    .bind(now)
    .execute(&mut *tx)
    .await
    .map_err(StorageError::from_sqlx)?;
    if done.rows_affected() == 1 {
        let cloud_kind = match severity {
            "critical" => crate::repo::outbox::EventKind::THRESHOLD_CRITICAL,
            "warning" => crate::repo::outbox::EventKind::THRESHOLD_WARNING,
            _ => crate::repo::outbox::EventKind::DEVICE_EVENT,
        };
        crate::repo::outbox::emit(&mut tx, cloud_kind, &serde_json::json!({"plant_id":plant_id,"source_event_id":event_id,"kind":kind,"severity":severity,"detail":detail}), now).await?;
    }
    tx.commit().await.map_err(StorageError::from_sqlx)?;
    Ok(done.rows_affected() == 1)
}

/// Plant-scoped events, newest first.
pub async fn plant_events(
    db: &EdgeDb,
    plant_id: &str,
    limit: i64,
) -> Result<Vec<(String, String, Option<String>, i64)>, StorageError> {
    let rows = sqlx::query(
        "SELECT kind,severity,detail_json,occurred_at FROM plant_events WHERE plant_id=? \
         ORDER BY occurred_at DESC, event_id DESC LIMIT ?",
    )
    .bind(plant_id)
    .bind(limit.clamp(1, 500))
    .fetch_all(db.pool())
    .await
    .map_err(StorageError::from_sqlx)?;
    Ok(rows
        .into_iter()
        .map(|r| {
            (
                r.get("kind"),
                r.get("severity"),
                r.get("detail_json"),
                r.get("occurred_at"),
            )
        })
        .collect())
}

// ------------------------------------------------------------- recommendations

/// A persisted recommendation.
#[derive(Clone, Debug, PartialEq)]
pub struct RecommendationRow {
    pub decision: String,
    pub recommended_ml: Option<f64>,
    pub confidence: f64,
    pub reasons_json: String,
    pub blocked_by: Option<String>,
    pub evaluated_at: i64,
}

pub async fn latest_recommendation(
    db: &EdgeDb,
    plant_id: &str,
) -> Result<Option<RecommendationRow>, StorageError> {
    let row = sqlx::query(
        "SELECT decision,recommended_ml,confidence,reasons_json,blocked_by,evaluated_at \
         FROM plant_recommendations WHERE plant_id=? ORDER BY evaluated_at DESC, id DESC LIMIT 1",
    )
    .bind(plant_id)
    .fetch_optional(db.pool())
    .await
    .map_err(StorageError::from_sqlx)?;
    Ok(row.map(|r| RecommendationRow {
        decision: r.get("decision"),
        recommended_ml: r.get("recommended_ml"),
        confidence: r.get("confidence"),
        reasons_json: r.get("reasons_json"),
        blocked_by: r.get("blocked_by"),
        evaluated_at: r.get("evaluated_at"),
    }))
}

pub async fn insert_recommendation(
    db: &EdgeDb,
    plant_id: &str,
    row: &RecommendationRow,
) -> Result<(), StorageError> {
    sqlx::query(
        "INSERT INTO plant_recommendations(plant_id,decision,recommended_ml,confidence,reasons_json,blocked_by,evaluated_at) \
         VALUES(?,?,?,?,?,?,?)",
    )
    .bind(plant_id)
    .bind(&row.decision)
    .bind(row.recommended_ml)
    .bind(row.confidence)
    .bind(&row.reasons_json)
    .bind(&row.blocked_by)
    .bind(row.evaluated_at)
    .execute(db.pool())
    .await
    .map_err(StorageError::from_sqlx)?;
    Ok(())
}

pub async fn recommendation_count(db: &EdgeDb, plant_id: &str) -> Result<i64, StorageError> {
    sqlx::query_scalar::<_, i64>("SELECT count(*) FROM plant_recommendations WHERE plant_id=?")
        .bind(plant_id)
        .fetch_one(db.pool())
        .await
        .map_err(StorageError::from_sqlx)
}

// ------------------------------------------------------------ threshold state

/// The persisted threshold state for one (plant, kind).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ThresholdStateRow {
    pub severity: String,
    pub candidate: Option<String>,
    pub candidate_since: Option<i64>,
}

pub async fn threshold_state(
    db: &EdgeDb,
    plant_id: &str,
    kind: &str,
) -> Result<Option<ThresholdStateRow>, StorageError> {
    let row = sqlx::query(
        "SELECT severity,candidate,candidate_since FROM plant_threshold_state WHERE plant_id=? AND kind=?",
    )
    .bind(plant_id)
    .bind(kind)
    .fetch_optional(db.pool())
    .await
    .map_err(StorageError::from_sqlx)?;
    Ok(row.map(|r| ThresholdStateRow {
        severity: r.get("severity"),
        candidate: r.get("candidate"),
        candidate_since: r.get("candidate_since"),
    }))
}

pub async fn put_threshold_state(
    db: &EdgeDb,
    plant_id: &str,
    kind: &str,
    state: &ThresholdStateRow,
    now: i64,
) -> Result<(), StorageError> {
    sqlx::query(
        "INSERT INTO plant_threshold_state(plant_id,kind,severity,candidate,candidate_since,updated_at) \
         VALUES(?,?,?,?,?,?) ON CONFLICT(plant_id,kind) DO UPDATE SET \
         severity=excluded.severity,candidate=excluded.candidate,candidate_since=excluded.candidate_since,updated_at=excluded.updated_at",
    )
    .bind(plant_id)
    .bind(kind)
    .bind(&state.severity)
    .bind(&state.candidate)
    .bind(state.candidate_since)
    .bind(now)
    .execute(db.pool())
    .await
    .map_err(StorageError::from_sqlx)?;
    Ok(())
}

pub async fn threshold_states(
    db: &EdgeDb,
    plant_id: &str,
) -> Result<Vec<(String, String)>, StorageError> {
    let rows = sqlx::query(
        "SELECT kind,severity FROM plant_threshold_state WHERE plant_id=? ORDER BY kind",
    )
    .bind(plant_id)
    .fetch_all(db.pool())
    .await
    .map_err(StorageError::from_sqlx)?;
    Ok(rows
        .into_iter()
        .map(|r| (r.get("kind"), r.get("severity")))
        .collect())
}

// ------------------------------------------------------------- stuck detection

/// The persisted run-length state for one sensor stream (M5-008).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StuckStateRow {
    pub last_bits: Option<i64>,
    pub last_bool: Option<bool>,
    /// Edge receipt time of the reading the run last consumed, so the same row
    /// is never folded in twice.
    pub last_received_at: Option<i64>,
    pub repeats: i64,
    pub reported: bool,
}

pub async fn stuck_state(
    db: &EdgeDb,
    device_id: &str,
    sensor_id: &str,
    point: &str,
    kind: &str,
) -> Result<StuckStateRow, StorageError> {
    let row = sqlx::query(
        "SELECT last_bits,last_bool,last_received_at,repeats,reported FROM sensor_stuck_state \
         WHERE device_id=? AND sensor_id=? AND point=? AND kind=?",
    )
    .bind(device_id)
    .bind(sensor_id)
    .bind(point)
    .bind(kind)
    .fetch_optional(db.pool())
    .await
    .map_err(StorageError::from_sqlx)?;
    Ok(row.map_or_else(StuckStateRow::default, |r| StuckStateRow {
        last_bits: r.get("last_bits"),
        last_bool: r.get::<Option<i64>, _>("last_bool").map(|v| v != 0),
        last_received_at: r.get("last_received_at"),
        repeats: r.get("repeats"),
        reported: r.get::<i64, _>("reported") != 0,
    }))
}

pub async fn put_stuck_state(
    db: &EdgeDb,
    device_id: &str,
    sensor_id: &str,
    point: &str,
    kind: &str,
    state: StuckStateRow,
    now: i64,
) -> Result<(), StorageError> {
    sqlx::query(
        "INSERT INTO sensor_stuck_state(device_id,sensor_id,point,kind,last_bits,last_bool,last_received_at,repeats,reported,updated_at) \
         VALUES(?,?,?,?,?,?,?,?,?,?) ON CONFLICT(device_id,sensor_id,point,kind) DO UPDATE SET \
         last_bits=excluded.last_bits,last_bool=excluded.last_bool,last_received_at=excluded.last_received_at, \
         repeats=excluded.repeats,reported=excluded.reported,updated_at=excluded.updated_at",
    )
    .bind(device_id)
    .bind(sensor_id)
    .bind(point)
    .bind(kind)
    .bind(state.last_bits)
    .bind(state.last_bool.map(i64::from))
    .bind(state.last_received_at)
    .bind(state.repeats)
    .bind(i64::from(state.reported))
    .bind(now)
    .execute(db.pool())
    .await
    .map_err(StorageError::from_sqlx)?;
    Ok(())
}

// ------------------------------------------------------------ watering ledger

/// A watering event as stored.
#[derive(Clone, Debug, PartialEq)]
pub struct WateringEventRow {
    pub watering_event_id: String,
    pub plant_id: Option<String>,
    pub device_id: Option<String>,
    pub command_id: Option<String>,
    pub mode: String,
    pub origin: String,
    pub started_at: i64,
    pub completed_at: Option<i64>,
    pub requested_ml: Option<f64>,
    pub delivered_ml: Option<f64>,
    pub status: String,
    pub reason_json: Option<String>,
}

/// Records a watering the system did **not** perform (M5-007).
///
/// `command_id` is `NULL` and `mode` is `detected`, which is how the daily-cap
/// query excludes it from the automatic budget while the cooldown query still
/// counts it: a human watered the plant, so the machine should wait.
///
/// The identifier is deterministic, so replaying the same pair of samples after
/// a restart records the same event once rather than twice.
pub async fn insert_detected_watering(
    db: &EdgeDb,
    plant_id: &str,
    device_id: Option<&str>,
    at: i64,
    estimated_ml: Option<f64>,
    detail: &serde_json::Value,
) -> Result<bool, StorageError> {
    let id = format!("detected:{plant_id}:{at}");
    let mut tx = db.begin().await?;
    let done = sqlx::query(
        "INSERT INTO watering_events(watering_event_id,plant_id,device_id,command_id,mode,origin,started_at,completed_at,requested_ml,delivered_ml,status,reason_json) \
         VALUES(?,?,?,NULL,'detected','detected',?,?,NULL,?,'completed',?) ON CONFLICT(watering_event_id) DO NOTHING",
    )
    .bind(&id)
    .bind(plant_id)
    .bind(device_id)
    .bind(at)
    .bind(at)
    .bind(estimated_ml)
    .bind(detail.to_string())
    .execute(&mut *tx)
    .await
    .map_err(StorageError::from_sqlx)?;
    if done.rows_affected() == 1 {
        crate::repo::outbox::emit(&mut tx, crate::repo::outbox::EventKind::WATERING_DETECTED, &serde_json::json!({"watering_event_id":id,"plant_id":plant_id,"device_id":device_id,"completed_at":at,"delivered_ml":estimated_ml,"detail":detail}), at).await?;
    }
    tx.commit().await.map_err(StorageError::from_sqlx)?;
    Ok(done.rows_affected() == 1)
}

/// The completion instant of the most recent watering of **any** mode
/// (F-050-13), which is what the cooldown is measured from.
pub async fn last_watering_at(db: &EdgeDb, plant_id: &str) -> Result<Option<i64>, StorageError> {
    sqlx::query_scalar::<_, Option<i64>>(
        "SELECT max(completed_at) FROM watering_events WHERE plant_id=? AND completed_at IS NOT NULL",
    )
    .bind(plant_id)
    .fetch_one(db.pool())
    .await
    .map_err(StorageError::from_sqlx)
}

/// The completion instant of the most recent **commanded** watering, used to
/// attribute a moisture rise to the command that caused it (F-050-16).
pub async fn last_command_completed_at(
    db: &EdgeDb,
    plant_id: &str,
) -> Result<Option<i64>, StorageError> {
    sqlx::query_scalar::<_, Option<i64>>(
        "SELECT max(completed_at) FROM watering_events \
         WHERE plant_id=? AND command_id IS NOT NULL AND completed_at IS NOT NULL",
    )
    .bind(plant_id)
    .fetch_one(db.pool())
    .await
    .map_err(StorageError::from_sqlx)
}

/// Volume delivered by **automatic** waterings in the rolling window.
///
/// Derived from rows and bounded by a timestamp, not a counter: a counter would
/// reset on restart, and a calendar day would permit two allowances around
/// midnight (SAFETY-006). `detected` rows are excluded because they were not
/// automatic; they still reset the cooldown through [`last_watering_at`].
pub async fn delivered_since(db: &EdgeDb, plant_id: &str, since: i64) -> Result<f64, StorageError> {
    let total: Option<f64> = sqlx::query_scalar(
        "SELECT sum(delivered_ml) FROM watering_events \
         WHERE plant_id=? AND mode<>'detected' AND completed_at IS NOT NULL AND completed_at>=?",
    )
    .bind(plant_id)
    .bind(since)
    .fetch_one(db.pool())
    .await
    .map_err(StorageError::from_sqlx)?;
    Ok(total.unwrap_or(0.0))
}

/// The watering ledger for a plant, newest first.
pub async fn watering_events(
    db: &EdgeDb,
    plant_id: &str,
    from: Option<i64>,
    to: Option<i64>,
    limit: i64,
) -> Result<Vec<WateringEventRow>, StorageError> {
    let rows = sqlx::query(
        "SELECT * FROM watering_events WHERE plant_id=? AND started_at>=? AND started_at<=? \
         ORDER BY started_at DESC, watering_event_id DESC LIMIT ?",
    )
    .bind(plant_id)
    .bind(from.unwrap_or(i64::MIN))
    .bind(to.unwrap_or(i64::MAX))
    .bind(limit.clamp(1, 500))
    .fetch_all(db.pool())
    .await
    .map_err(StorageError::from_sqlx)?;
    Ok(rows
        .into_iter()
        .map(|r| WateringEventRow {
            watering_event_id: r.get("watering_event_id"),
            plant_id: r.get("plant_id"),
            device_id: r.get("device_id"),
            command_id: r.get("command_id"),
            mode: r.get("mode"),
            origin: r.get("origin"),
            started_at: r.get("started_at"),
            completed_at: r.get("completed_at"),
            requested_ml: r.get("requested_ml"),
            delivered_ml: r.get("delivered_ml"),
            status: r.get("status"),
            reason_json: r.get("reason_json"),
        })
        .collect())
}

/// How many plant rows reference a profile.
///
/// Deliberately **all** rows, including soft-deleted ones. The foreign key is
/// what actually prevents the profile from being removed, and it does not know
/// about `deleted_at`; counting only live plants would report a profile as free
/// and then fail the delete with a constraint error nobody asked about. A
/// removed plant still holds the record of which template it was seeded from,
/// and that is worth more than the ability to tidy an unused profile away.
pub async fn count_using_profile(db: &EdgeDb, profile_id: &str) -> Result<i64, StorageError> {
    sqlx::query_scalar::<_, i64>("SELECT count(*) FROM plants WHERE profile_id=?")
        .bind(profile_id)
        .fetch_one(db.pool())
        .await
        .map_err(StorageError::from_sqlx)
}

/// The number of live plants, for the `plants_total` gauge.
pub async fn live_count(db: &EdgeDb) -> Result<i64, StorageError> {
    sqlx::query_scalar::<_, i64>("SELECT count(*) FROM plants WHERE deleted_at IS NULL")
        .fetch_one(db.pool())
        .await
        .map_err(StorageError::from_sqlx)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn db() -> EdgeDb {
        let db = EdgeDb::in_memory().await.unwrap();
        db.migrate().await.unwrap();
        db
    }

    fn new_plant(id: &str) -> NewPlant {
        NewPlant {
            plant_id: id.to_owned(),
            name: "Monstera".to_owned(),
            species: Some("Monstera deliciosa".to_owned()),
            profile_id: None,
            pot_volume_ml: Some(2_500.0),
            soil_type: Some("aroid mix".to_owned()),
        }
    }

    /// A soft delete is a real state change, and ADR-005's catalogue has no
    /// `plant.deleted` to spend on it. The canonical `plant.updated` carries it,
    /// with the operation named and the identity the plant had — a plant that
    /// merely stopped producing events would be indistinguishable in cloud
    /// history from one whose edge went quiet.
    #[tokio::test]
    async fn deleting_a_plant_emits_plant_updated_once_and_never_for_a_second_delete() {
        let db = db().await;
        crate::repo::outbox::configure(&db, true, 500_000)
            .await
            .unwrap();
        create(&db, &new_plant("monstera-01"), 1_000).await.unwrap();
        sqlx::query("DELETE FROM pending_cloud_events")
            .execute(db.pool())
            .await
            .unwrap();

        assert!(delete(&db, "monstera-01", 2_000).await.unwrap());
        let rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT kind,payload_json FROM pending_cloud_events ORDER BY created_at,event_id",
        )
        .fetch_all(db.pool())
        .await
        .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].0, "plant.updated");
        let payload: serde_json::Value = serde_json::from_str(&rows[0].1).unwrap();
        assert_eq!(payload["operation"], "delete");
        assert_eq!(payload["plant_id"], "monstera-01");
        assert_eq!(payload["name"], "Monstera");
        assert_eq!(payload["deleted_at"], 2_000);

        // Already gone: no row changes, so no event is invented.
        assert!(!delete(&db, "monstera-01", 3_000).await.unwrap());
        assert!(!delete(&db, "absent", 3_000).await.unwrap());
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM pending_cloud_events")
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn plants_can_be_created_read_updated_and_listed() {
        let db = db().await;
        let created = create(&db, &new_plant("monstera-01"), 1_000).await.unwrap();
        assert_eq!(created.name, "Monstera");
        assert_eq!(created.pot_volume_ml, Some(2_500.0));
        assert_eq!(created.created_at, 1_000);
        assert_eq!(get(&db, "monstera-01").await.unwrap(), Some(created));
        assert_eq!(get(&db, "absent").await.unwrap(), None);

        create(&db, &new_plant("fern-01"), 1_000).await.unwrap();
        let all = list(&db, None, 50).await.unwrap();
        assert_eq!(
            all.iter().map(|p| p.plant_id.as_str()).collect::<Vec<_>>(),
            vec!["fern-01", "monstera-01"]
        );
        let page = list(&db, Some("fern-01"), 50).await.unwrap();
        assert_eq!(page.len(), 1);
        assert_eq!(page[0].plant_id, "monstera-01");
        assert_eq!(list(&db, None, 1).await.unwrap().len(), 1);

        let updated = update(
            &db,
            "monstera-01",
            &PlantPatch {
                name: Some("Big Monstera".to_owned()),
                soil_type: Some(None),
                ..Default::default()
            },
            2_000,
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(updated.name, "Big Monstera");
        assert_eq!(updated.soil_type, None, "Some(None) clears a field");
        assert_eq!(
            updated.species,
            Some("Monstera deliciosa".to_owned()),
            "an omitted field is left alone"
        );
        assert_eq!(
            update(&db, "absent", &PlantPatch::default(), 1_000)
                .await
                .unwrap(),
            None
        );
        assert_eq!(live_count(&db).await.unwrap(), 2);
    }

    /// F-050-01 and SAFETY-012, enforced at the storage layer rather than only
    /// in the API: a plant created by any path is inert until a human opts in.
    #[tokio::test]
    async fn a_new_plant_has_auto_watering_disabled() {
        let db = db().await;
        let created = create(&db, &new_plant("monstera-01"), 1_000).await.unwrap();
        assert!(!created.auto_watering_enabled);
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT auto_watering_enabled FROM plants")
                .fetch_one(db.pool())
                .await
                .unwrap(),
            0,
            "the column itself must be off, not merely the projection"
        );
        // It takes a deliberate act to turn it on.
        let enabled = update(
            &db,
            "monstera-01",
            &PlantPatch {
                auto_watering_enabled: Some(true),
                ..Default::default()
            },
            2_000,
        )
        .await
        .unwrap()
        .unwrap();
        assert!(enabled.auto_watering_enabled);
    }

    /// F-050-05. The ledger is the record of what the machine did, and it
    /// outlives the row that pointed at it -- with its attribution intact.
    #[tokio::test]
    async fn deleting_a_plant_leaves_its_watering_events_intact() {
        let db = db().await;
        create(&db, &new_plant("monstera-01"), 1_000).await.unwrap();
        assert!(
            insert_detected_watering(
                &db,
                "monstera-01",
                Some("plant-node-01"),
                2_000,
                Some(350.0),
                &serde_json::json!({"source": "weight"}),
            )
            .await
            .unwrap()
        );

        assert!(delete(&db, "monstera-01", 3_000).await.unwrap());
        assert_eq!(get(&db, "monstera-01").await.unwrap(), None);
        assert!(list(&db, None, 50).await.unwrap().is_empty());
        assert_eq!(live_count(&db).await.unwrap(), 0);
        assert!(
            !delete(&db, "monstera-01", 4_000).await.unwrap(),
            "a second delete changes nothing"
        );

        let events = watering_events(&db, "monstera-01", None, None, 50)
            .await
            .unwrap();
        assert_eq!(events.len(), 1, "history survives the plant");
        assert_eq!(events[0].plant_id, Some("monstera-01".to_owned()));
        assert_eq!(events[0].mode, "detected");
        assert_eq!(events[0].delivered_ml, Some(350.0));
    }

    /// Foreign keys are on, so a plant cannot name a profile that is not there.
    #[tokio::test]
    async fn foreign_key_violations_are_rejected() {
        use rhizo_telemetry::{Classify as _, FailureKind};

        let db = db().await;
        let orphan = NewPlant {
            profile_id: Some("no-such-profile".to_owned()),
            ..new_plant("monstera-01")
        };
        let error = create(&db, &orphan, 1_000).await.unwrap_err();
        assert!(matches!(error, StorageError::Constraint(_)));
        assert_eq!(
            error.classify(),
            FailureKind::Permanent,
            "SQLITE_CONSTRAINT_FOREIGNKEY (787) must never become fatal or retryable"
        );
        assert_eq!(live_count(&db).await.unwrap(), 0);

        // The same id twice is a primary-key violation, not a silent overwrite.
        create(&db, &new_plant("monstera-01"), 1_000).await.unwrap();
        assert!(matches!(
            create(&db, &new_plant("monstera-01"), 1_000).await,
            Err(StorageError::Constraint(_))
        ));
    }

    #[tokio::test]
    async fn the_detected_ledger_answers_the_cooldown_and_budget_questions() {
        let db = db().await;
        create(&db, &new_plant("monstera-01"), 1_000).await.unwrap();
        assert_eq!(last_watering_at(&db, "monstera-01").await.unwrap(), None);
        assert_eq!(delivered_since(&db, "monstera-01", 0).await.unwrap(), 0.0);

        insert_detected_watering(
            &db,
            "monstera-01",
            None,
            5_000,
            Some(120.0),
            &serde_json::json!({}),
        )
        .await
        .unwrap();
        assert_eq!(
            last_watering_at(&db, "monstera-01").await.unwrap(),
            Some(5_000),
            "a detected watering resets the cooldown"
        );
        assert_eq!(
            delivered_since(&db, "monstera-01", 0).await.unwrap(),
            0.0,
            "a detected watering is excluded from the automatic daily total"
        );
        assert_eq!(
            last_command_completed_at(&db, "monstera-01").await.unwrap(),
            None,
            "no command was involved"
        );

        // The same pair of samples replayed after a restart records once.
        assert!(
            !insert_detected_watering(
                &db,
                "monstera-01",
                None,
                5_000,
                Some(120.0),
                &serde_json::json!({})
            )
            .await
            .unwrap()
        );
        assert_eq!(
            watering_events(&db, "monstera-01", None, None, 50)
                .await
                .unwrap()
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn the_dry_accumulator_survives_a_reopen() {
        let db = db().await;
        create(&db, &new_plant("monstera-01"), 1_000).await.unwrap();
        assert_eq!(
            dry_state(&db, "monstera-01").await.unwrap(),
            DryStateRow::default()
        );
        let state = DryStateRow {
            dry_ms: 1_200_000,
            last_sample_at: Some(9_000),
        };
        put_dry_state(&db, "monstera-01", state, 9_000)
            .await
            .unwrap();
        assert_eq!(dry_state(&db, "monstera-01").await.unwrap(), state);
        put_dry_state(&db, "monstera-01", DryStateRow::default(), 10_000)
            .await
            .unwrap();
        assert_eq!(
            dry_state(&db, "monstera-01").await.unwrap(),
            DryStateRow::default()
        );
    }

    #[tokio::test]
    async fn state_transitions_are_recorded_and_steady_state_writes_nothing() {
        let db = db().await;
        create(&db, &new_plant("monstera-01"), 1_000).await.unwrap();
        assert_eq!(plant_state(&db, "monstera-01").await.unwrap(), None);
        record_state_transition(&db, "monstera-01", None, "healthy", 1_000)
            .await
            .unwrap();
        assert_eq!(
            plant_state(&db, "monstera-01").await.unwrap(),
            Some("healthy".to_owned())
        );
        record_state_transition(&db, "monstera-01", Some("healthy"), "drying", 2_000)
            .await
            .unwrap();
        assert_eq!(
            plant_state(&db, "monstera-01").await.unwrap(),
            Some("drying".to_owned())
        );
        let events = plant_events(&db, "monstera-01", 50).await.unwrap();
        assert_eq!(events.len(), 2, "one row per transition, and no more");
        assert_eq!(events[0].0, "plant_state_changed");
    }

    #[tokio::test]
    async fn stuck_and_threshold_state_round_trip() {
        let db = db().await;
        create(&db, &new_plant("monstera-01"), 1_000).await.unwrap();
        let stuck = StuckStateRow {
            last_bits: Some(4_611_686_018_427_387_904),
            last_bool: None,
            last_received_at: Some(7_000),
            repeats: 19,
            reported: false,
        };
        put_stuck_state(
            &db,
            "plant-node-01",
            "soil-0",
            "default",
            "soil_moisture",
            stuck,
            1,
        )
        .await
        .unwrap();
        assert_eq!(
            stuck_state(&db, "plant-node-01", "soil-0", "default", "soil_moisture",)
                .await
                .unwrap(),
            stuck
        );
        let other = StuckStateRow {
            repeats: 1,
            ..StuckStateRow::default()
        };
        put_stuck_state(
            &db,
            "plant-node-01",
            "soil-1",
            "default",
            "soil_moisture",
            other,
            2,
        )
        .await
        .unwrap();
        assert_eq!(
            stuck_state(&db, "plant-node-01", "soil-0", "default", "soil_moisture",)
                .await
                .unwrap(),
            stuck,
            "two same-kind sensors must not share stuck-run state"
        );

        let threshold = ThresholdStateRow {
            severity: "warning".to_owned(),
            candidate: Some("critical".to_owned()),
            candidate_since: Some(42),
        };
        put_threshold_state(&db, "monstera-01", "ambient_temperature", &threshold, 1)
            .await
            .unwrap();
        assert_eq!(
            threshold_state(&db, "monstera-01", "ambient_temperature")
                .await
                .unwrap(),
            Some(threshold)
        );
        assert_eq!(
            threshold_states(&db, "monstera-01").await.unwrap(),
            vec![("ambient_temperature".to_owned(), "warning".to_owned())]
        );
    }

    #[tokio::test]
    async fn recommendations_are_read_back_newest_first() {
        let db = db().await;
        create(&db, &new_plant("monstera-01"), 1_000).await.unwrap();
        assert_eq!(
            latest_recommendation(&db, "monstera-01").await.unwrap(),
            None
        );
        for (at, decision) in [(1_000, "no_water"), (2_000, "water")] {
            insert_recommendation(
                &db,
                "monstera-01",
                &RecommendationRow {
                    decision: decision.to_owned(),
                    recommended_ml: Some(40.0),
                    confidence: 0.9,
                    reasons_json: "[]".to_owned(),
                    blocked_by: None,
                    evaluated_at: at,
                },
            )
            .await
            .unwrap();
        }
        let latest = latest_recommendation(&db, "monstera-01")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(latest.decision, "water");
        assert_eq!(recommendation_count(&db, "monstera-01").await.unwrap(), 2);
    }

    /// The provenance columns are written once and read by nothing that decides.
    #[tokio::test]
    async fn applied_preset_provenance_is_recorded_and_inert() {
        let db = db().await;
        create(&db, &new_plant("monstera-01"), 1_000).await.unwrap();
        record_applied_preset(&db, "monstera-01", "monstera-deliciosa", 1)
            .await
            .unwrap();
        let plant = get(&db, "monstera-01").await.unwrap().unwrap();
        assert_eq!(
            plant.applied_preset_id.as_deref(),
            Some("monstera-deliciosa")
        );
        assert_eq!(plant.applied_catalogue_version, Some(1));
        assert!(
            !plant.auto_watering_enabled,
            "provenance does not authorise anything"
        );
    }
}
