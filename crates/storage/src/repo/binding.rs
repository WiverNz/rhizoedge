//! Bindings, per-measurement policies, and offline policies (M5-013, M5-014,
//! M5-016).
//!
//! The authoritative per-plant configuration under
//! [ADR-016](../../../../docs/adr/016-plant-binding-and-policy-model.md). A
//! profile seeds these rows once and then stops mattering; a preset writes
//! exactly these rows and then gets out of the way (M5-018). Nothing downstream
//! can tell which path produced a row, which is the point.
//!
//! The schema carries two of the rules itself: `uq_binding_control` makes "at
//! most one `control` binding per plant" a database constraint rather than a
//! hope, and `actuator_bindings` is keyed by `plant_id`, which is where the
//! `0..1` cardinality of SAFETY-018 actually lives.
#![allow(missing_docs)]
use sqlx::Row as _;

use crate::repo::outbox::EventKind;
use crate::{EdgeDb, StorageError};

/// A sensor binding as stored.
#[derive(Clone, Debug, PartialEq)]
pub struct SensorBindingRow {
    pub binding_id: String,
    pub plant_id: String,
    pub device_id: String,
    pub sensor_id: String,
    pub point: String,
    pub kind: String,
    pub role: String,
    pub created_at: i64,
}

/// An actuator binding as stored. At most one per plant.
#[derive(Clone, Debug, PartialEq)]
pub struct ActuatorBindingRow {
    pub plant_id: String,
    pub device_id: String,
    pub actuator_id: String,
    pub kind: String,
    pub created_at: i64,
}

/// A per-measurement policy as stored.
#[derive(Clone, Debug, PartialEq)]
pub struct MeasurementPolicyRow {
    pub plant_id: String,
    pub kind: String,
    pub target_min: Option<f64>,
    pub target_max: Option<f64>,
    pub warning_low: Option<f64>,
    pub warning_high: Option<f64>,
    pub critical_low: Option<f64>,
    pub critical_high: Option<f64>,
    pub stale_after_ms: i64,
    pub hysteresis: Option<f64>,
    pub confirm_duration_ms: Option<i64>,
}

/// One declared device capability, as the registry recorded it (M4-011).
#[derive(Clone, Debug, PartialEq)]
pub struct DeclaredCapabilityRow {
    pub device_id: String,
    pub capability_id: String,
    pub class: String,
    pub kinds_json: String,
    pub point: Option<String>,
}

/// The stored offline policy for a plant.
#[derive(Clone, Debug, PartialEq)]
pub struct OfflinePolicyRow {
    pub plant_id: String,
    pub policy_version: i64,
    pub enabled: bool,
    pub policy_json: String,
    pub published_at: Option<i64>,
    pub applied_version: Option<i64>,
    pub updated_at: i64,
}

// --------------------------------------------------------------- capabilities

/// Everything the fleet has declared, for binding validation.
pub async fn declared_capabilities(
    db: &EdgeDb,
) -> Result<Vec<DeclaredCapabilityRow>, StorageError> {
    let rows = sqlx::query(
        "SELECT device_id,capability_id,class,kinds_json,point FROM device_capabilities \
         ORDER BY device_id,class,capability_id",
    )
    .fetch_all(db.pool())
    .await
    .map_err(StorageError::from_sqlx)?;
    Ok(rows
        .into_iter()
        .map(|r| DeclaredCapabilityRow {
            device_id: r.get("device_id"),
            capability_id: r.get("capability_id"),
            class: r.get("class"),
            kinds_json: r.get("kinds_json"),
            point: r.get("point"),
        })
        .collect())
}

// ------------------------------------------------------------ sensor bindings

pub async fn sensor_bindings(
    db: &EdgeDb,
    plant_id: &str,
) -> Result<Vec<SensorBindingRow>, StorageError> {
    let rows =
        sqlx::query("SELECT * FROM sensor_bindings WHERE plant_id=? ORDER BY role,kind,binding_id")
            .bind(plant_id)
            .fetch_all(db.pool())
            .await
            .map_err(StorageError::from_sqlx)?;
    Ok(rows.iter().map(to_sensor_binding).collect())
}

pub async fn sensor_binding(
    db: &EdgeDb,
    binding_id: &str,
) -> Result<Option<SensorBindingRow>, StorageError> {
    Ok(
        sqlx::query("SELECT * FROM sensor_bindings WHERE binding_id=?")
            .bind(binding_id)
            .fetch_optional(db.pool())
            .await
            .map_err(StorageError::from_sqlx)?
            .as_ref()
            .map(to_sensor_binding),
    )
}

fn to_sensor_binding(row: &sqlx::sqlite::SqliteRow) -> SensorBindingRow {
    SensorBindingRow {
        binding_id: row.get("binding_id"),
        plant_id: row.get("plant_id"),
        device_id: row.get("device_id"),
        sensor_id: row.get("sensor_id"),
        point: row.get("point"),
        kind: row.get("kind"),
        role: row.get("role"),
        created_at: row.get("created_at"),
    }
}

pub async fn upsert_sensor_binding(
    db: &EdgeDb,
    binding: &SensorBindingRow,
) -> Result<(), StorageError> {
    let mut tx = db.begin().await?;
    sqlx::query(
        "INSERT INTO sensor_bindings(binding_id,plant_id,device_id,sensor_id,point,kind,role,created_at) \
         VALUES(?,?,?,?,?,?,?,?) ON CONFLICT(binding_id) DO UPDATE SET \
         device_id=excluded.device_id,sensor_id=excluded.sensor_id,point=excluded.point,kind=excluded.kind,role=excluded.role",
    )
    .bind(&binding.binding_id)
    .bind(&binding.plant_id)
    .bind(&binding.device_id)
    .bind(&binding.sensor_id)
    .bind(&binding.point)
    .bind(&binding.kind)
    .bind(&binding.role)
    .bind(binding.created_at)
    .execute(&mut *tx)
    .await
    .map_err(StorageError::from_sqlx)?;
    crate::repo::outbox::emit(&mut tx, EventKind::PLANT_BINDING_CHANGED, &serde_json::json!({"operation":"upsert","binding_class":"sensor","plant_id":binding.plant_id,"binding_id":binding.binding_id,"device_id":binding.device_id,"sensor_id":binding.sensor_id,"point":binding.point,"kind":binding.kind,"role":binding.role}), binding.created_at).await?;
    tx.commit().await.map_err(StorageError::from_sqlx)
}

/// Removes a sensor binding, and says so to the cloud in the same transaction.
///
/// A delete is a real configuration change, so it carries the same canonical
/// `plant.binding_changed` kind an upsert does — ADR-005's catalogue is closed,
/// and a `plant.binding_removed` invented here would reach the cloud as an
/// unknown kind and be quarantined. The `operation` field is what distinguishes
/// them, and the identity of the row that went away is copied into the payload
/// *before* the delete: a historical event saying only that "something was
/// unbound from this plant" cannot be read back later, which is the whole
/// reason the event exists.
///
/// Deleting a binding that is not there changes nothing and emits nothing.
pub async fn delete_sensor_binding(
    db: &EdgeDb,
    binding_id: &str,
    now: i64,
) -> Result<bool, StorageError> {
    let mut tx = db.begin().await?;
    let previous = sqlx::query("SELECT * FROM sensor_bindings WHERE binding_id=?")
        .bind(binding_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(StorageError::from_sqlx)?
        .as_ref()
        .map(to_sensor_binding);
    let Some(previous) = previous else {
        return Ok(false);
    };
    let done = sqlx::query("DELETE FROM sensor_bindings WHERE binding_id=?")
        .bind(binding_id)
        .execute(&mut *tx)
        .await
        .map_err(StorageError::from_sqlx)?;
    if done.rows_affected() != 1 {
        return Ok(false);
    }
    crate::repo::outbox::emit(&mut tx, EventKind::PLANT_BINDING_CHANGED, &serde_json::json!({"operation":"delete","binding_class":"sensor","plant_id":previous.plant_id,"binding_id":previous.binding_id,"device_id":previous.device_id,"sensor_id":previous.sensor_id,"point":previous.point,"kind":previous.kind,"role":previous.role,"bound_at":previous.created_at}), now).await?;
    tx.commit().await.map_err(StorageError::from_sqlx)?;
    Ok(true)
}

// ---------------------------------------------------------- actuator binding

pub async fn actuator_binding(
    db: &EdgeDb,
    plant_id: &str,
) -> Result<Option<ActuatorBindingRow>, StorageError> {
    Ok(
        sqlx::query("SELECT * FROM actuator_bindings WHERE plant_id=?")
            .bind(plant_id)
            .fetch_optional(db.pool())
            .await
            .map_err(StorageError::from_sqlx)?
            .map(|r| ActuatorBindingRow {
                plant_id: r.get("plant_id"),
                device_id: r.get("device_id"),
                actuator_id: r.get("actuator_id"),
                kind: r.get("kind"),
                created_at: r.get("created_at"),
            }),
    )
}

pub async fn upsert_actuator_binding(
    db: &EdgeDb,
    binding: &ActuatorBindingRow,
) -> Result<(), StorageError> {
    let mut tx = db.begin().await?;
    sqlx::query(
        "INSERT INTO actuator_bindings(plant_id,device_id,actuator_id,kind,created_at) VALUES(?,?,?,?,?) \
         ON CONFLICT(plant_id) DO UPDATE SET device_id=excluded.device_id,actuator_id=excluded.actuator_id,kind=excluded.kind",
    )
    .bind(&binding.plant_id)
    .bind(&binding.device_id)
    .bind(&binding.actuator_id)
    .bind(&binding.kind)
    .bind(binding.created_at)
    .execute(&mut *tx)
    .await
    .map_err(StorageError::from_sqlx)?;
    crate::repo::outbox::emit(&mut tx, EventKind::PLANT_BINDING_CHANGED, &serde_json::json!({"operation":"upsert","binding_class":"actuator","plant_id":binding.plant_id,"device_id":binding.device_id,"actuator_id":binding.actuator_id,"kind":binding.kind}), binding.created_at).await?;
    tx.commit().await.map_err(StorageError::from_sqlx)
}

/// Unbinds a plant's actuator, and says so in the same transaction.
///
/// Removing the actuator turns a watering plant into a monitoring plant
/// (SAFETY-018), which is exactly the kind of change history has to record: the
/// payload names the device and actuator that stopped being this plant's pump,
/// so a later reader can tell which hardware the plant's earlier doses came
/// from. Nothing is emitted for a plant that had no actuator to begin with.
pub async fn delete_actuator_binding(
    db: &EdgeDb,
    plant_id: &str,
    now: i64,
) -> Result<bool, StorageError> {
    let mut tx = db.begin().await?;
    let previous = sqlx::query("SELECT * FROM actuator_bindings WHERE plant_id=?")
        .bind(plant_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(StorageError::from_sqlx)?
        .map(|r| ActuatorBindingRow {
            plant_id: r.get("plant_id"),
            device_id: r.get("device_id"),
            actuator_id: r.get("actuator_id"),
            kind: r.get("kind"),
            created_at: r.get("created_at"),
        });
    let Some(previous) = previous else {
        return Ok(false);
    };
    let done = sqlx::query("DELETE FROM actuator_bindings WHERE plant_id=?")
        .bind(plant_id)
        .execute(&mut *tx)
        .await
        .map_err(StorageError::from_sqlx)?;
    if done.rows_affected() != 1 {
        return Ok(false);
    }
    crate::repo::outbox::emit(&mut tx, EventKind::PLANT_BINDING_CHANGED, &serde_json::json!({"operation":"delete","binding_class":"actuator","plant_id":previous.plant_id,"device_id":previous.device_id,"actuator_id":previous.actuator_id,"kind":previous.kind,"bound_at":previous.created_at}), now).await?;
    tx.commit().await.map_err(StorageError::from_sqlx)?;
    Ok(true)
}

// ------------------------------------------------------- measurement policies

fn to_measurement_policy(row: &sqlx::sqlite::SqliteRow) -> MeasurementPolicyRow {
    MeasurementPolicyRow {
        plant_id: row.get("plant_id"),
        kind: row.get("kind"),
        target_min: row.get("target_min"),
        target_max: row.get("target_max"),
        warning_low: row.get("warning_low"),
        warning_high: row.get("warning_high"),
        critical_low: row.get("critical_low"),
        critical_high: row.get("critical_high"),
        stale_after_ms: row.get("stale_after_ms"),
        hysteresis: row.get("hysteresis"),
        confirm_duration_ms: row.get("confirm_duration_ms"),
    }
}

pub async fn measurement_policies(
    db: &EdgeDb,
    plant_id: &str,
) -> Result<Vec<MeasurementPolicyRow>, StorageError> {
    let rows = sqlx::query("SELECT * FROM measurement_policies WHERE plant_id=? ORDER BY kind")
        .bind(plant_id)
        .fetch_all(db.pool())
        .await
        .map_err(StorageError::from_sqlx)?;
    Ok(rows.iter().map(to_measurement_policy).collect())
}

pub async fn measurement_policy(
    db: &EdgeDb,
    plant_id: &str,
    kind: &str,
) -> Result<Option<MeasurementPolicyRow>, StorageError> {
    Ok(
        sqlx::query("SELECT * FROM measurement_policies WHERE plant_id=? AND kind=?")
            .bind(plant_id)
            .bind(kind)
            .fetch_optional(db.pool())
            .await
            .map_err(StorageError::from_sqlx)?
            .as_ref()
            .map(to_measurement_policy),
    )
}

pub async fn upsert_measurement_policy(
    db: &EdgeDb,
    policy: &MeasurementPolicyRow,
    now: i64,
) -> Result<(), StorageError> {
    let mut tx = db.begin().await?;
    sqlx::query(
        "INSERT INTO measurement_policies(plant_id,kind,target_min,target_max,warning_low,warning_high,critical_low,critical_high,stale_after_ms,hysteresis,confirm_duration_ms,updated_at) \
         VALUES(?,?,?,?,?,?,?,?,?,?,?,?) ON CONFLICT(plant_id,kind) DO UPDATE SET \
         target_min=excluded.target_min,target_max=excluded.target_max,warning_low=excluded.warning_low,warning_high=excluded.warning_high, \
         critical_low=excluded.critical_low,critical_high=excluded.critical_high,stale_after_ms=excluded.stale_after_ms, \
         hysteresis=excluded.hysteresis,confirm_duration_ms=excluded.confirm_duration_ms,updated_at=excluded.updated_at",
    )
    .bind(&policy.plant_id)
    .bind(&policy.kind)
    .bind(policy.target_min)
    .bind(policy.target_max)
    .bind(policy.warning_low)
    .bind(policy.warning_high)
    .bind(policy.critical_low)
    .bind(policy.critical_high)
    .bind(policy.stale_after_ms)
    .bind(policy.hysteresis)
    .bind(policy.confirm_duration_ms)
    .bind(now)
    .execute(&mut *tx)
    .await
    .map_err(StorageError::from_sqlx)?;
    crate::repo::outbox::emit(&mut tx, EventKind::PLANT_POLICY_CHANGED, &serde_json::json!({"operation":"upsert","policy_class":"measurement","plant_id":policy.plant_id,"kind":policy.kind,"target_min":policy.target_min,"target_max":policy.target_max,"warning_low":policy.warning_low,"warning_high":policy.warning_high,"critical_low":policy.critical_low,"critical_high":policy.critical_high,"stale_after_ms":policy.stale_after_ms,"hysteresis":policy.hysteresis,"confirm_duration_ms":policy.confirm_duration_ms}), now).await?;
    tx.commit().await.map_err(StorageError::from_sqlx)
}

/// Removes one measurement policy, and says so in the same transaction.
///
/// The thresholds that went away are copied into the payload. A policy delete
/// can retire the control measurement a plant waters on, so an event recording
/// only that "a policy for soil_moisture was removed" would leave the cloud
/// unable to say what the plant's rules were before it happened.
///
/// Deleting a policy that is not there changes nothing and emits nothing.
pub async fn delete_measurement_policy(
    db: &EdgeDb,
    plant_id: &str,
    kind: &str,
    now: i64,
) -> Result<bool, StorageError> {
    let mut tx = db.begin().await?;
    let previous = sqlx::query("SELECT * FROM measurement_policies WHERE plant_id=? AND kind=?")
        .bind(plant_id)
        .bind(kind)
        .fetch_optional(&mut *tx)
        .await
        .map_err(StorageError::from_sqlx)?
        .as_ref()
        .map(to_measurement_policy);
    let Some(previous) = previous else {
        return Ok(false);
    };
    let done = sqlx::query("DELETE FROM measurement_policies WHERE plant_id=? AND kind=?")
        .bind(plant_id)
        .bind(kind)
        .execute(&mut *tx)
        .await
        .map_err(StorageError::from_sqlx)?;
    if done.rows_affected() != 1 {
        return Ok(false);
    }
    crate::repo::outbox::emit(&mut tx, EventKind::PLANT_POLICY_CHANGED, &serde_json::json!({"operation":"delete","policy_class":"measurement","plant_id":previous.plant_id,"kind":previous.kind,"target_min":previous.target_min,"target_max":previous.target_max,"warning_low":previous.warning_low,"warning_high":previous.warning_high,"critical_low":previous.critical_low,"critical_high":previous.critical_high,"stale_after_ms":previous.stale_after_ms,"hysteresis":previous.hysteresis,"confirm_duration_ms":previous.confirm_duration_ms}), now).await?;
    tx.commit().await.map_err(StorageError::from_sqlx)?;
    Ok(true)
}

/// Atomically materialises every durable effect of applying a preset.
///
/// Planning and validation happen before this call. Keeping the policy rows,
/// profile document, optional profile assignment, and provenance in one SQLite
/// transaction prevents a late constraint or I/O failure from leaving a plant
/// half configured.
#[allow(clippy::too_many_arguments)]
pub async fn materialize_preset(
    db: &EdgeDb,
    plant_id: &str,
    policies: &[MeasurementPolicyRow],
    current_profile_id: Option<&str>,
    private_profile_id: &str,
    profile_name: &str,
    profile_json: &str,
    preset_id: &str,
    catalogue_version: u32,
    now: i64,
) -> Result<(), StorageError> {
    let mut tx = db.begin().await?;
    // A profile is a seed, not shared mutable runtime configuration. Reusing it
    // is safe only when this plant is its sole owner; otherwise materialisation
    // clones into the caller-supplied private id and re-points only this plant.
    let profile_id = if let Some(current) = current_profile_id {
        let references: i64 = sqlx::query_scalar("SELECT count(*) FROM plants WHERE profile_id=?")
            .bind(current)
            .fetch_one(&mut *tx)
            .await
            .map_err(StorageError::from_sqlx)?;
        if references == 1 {
            current
        } else {
            private_profile_id
        }
    } else {
        private_profile_id
    };
    for policy in policies {
        sqlx::query(
            "INSERT INTO measurement_policies(plant_id,kind,target_min,target_max,warning_low,warning_high,critical_low,critical_high,stale_after_ms,hysteresis,confirm_duration_ms,updated_at) \
             VALUES(?,?,?,?,?,?,?,?,?,?,?,?) ON CONFLICT(plant_id,kind) DO UPDATE SET \
             target_min=excluded.target_min,target_max=excluded.target_max,warning_low=excluded.warning_low,warning_high=excluded.warning_high, \
             critical_low=excluded.critical_low,critical_high=excluded.critical_high,stale_after_ms=excluded.stale_after_ms, \
             hysteresis=excluded.hysteresis,confirm_duration_ms=excluded.confirm_duration_ms,updated_at=excluded.updated_at",
        )
        .bind(&policy.plant_id)
        .bind(&policy.kind)
        .bind(policy.target_min)
        .bind(policy.target_max)
        .bind(policy.warning_low)
        .bind(policy.warning_high)
        .bind(policy.critical_low)
        .bind(policy.critical_high)
        .bind(policy.stale_after_ms)
        .bind(policy.hysteresis)
        .bind(policy.confirm_duration_ms)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(StorageError::from_sqlx)?;
    }
    sqlx::query(
        "INSERT INTO plant_profiles(profile_id,name,profile_json,updated_at) VALUES(?,?,?,?) \
         ON CONFLICT(profile_id) DO UPDATE SET name=excluded.name,profile_json=excluded.profile_json,updated_at=excluded.updated_at",
    )
    .bind(profile_id)
    .bind(profile_name)
    .bind(profile_json)
    .bind(now)
    .execute(&mut *tx)
    .await
    .map_err(StorageError::from_sqlx)?;
    let updated = sqlx::query(
        "UPDATE plants SET profile_id=?,applied_preset_id=?,applied_catalogue_version=? \
         WHERE plant_id=? AND deleted_at IS NULL",
    )
    .bind(profile_id)
    .bind(preset_id)
    .bind(i64::from(catalogue_version))
    .bind(plant_id)
    .execute(&mut *tx)
    .await
    .map_err(StorageError::from_sqlx)?;
    if updated.rows_affected() != 1 {
        return Err(StorageError::Constraint(format!(
            "preset target {plant_id} is no longer a live plant"
        )));
    }
    // A preset writes exactly the rows `upsert_measurement_policy` writes, so it
    // owes the cloud exactly the same events. Nothing downstream can tell which
    // path produced a policy row, and nothing downstream should be able to tell
    // which path produced its event either.
    for policy in policies {
        crate::repo::outbox::emit(&mut tx, EventKind::PLANT_POLICY_CHANGED, &serde_json::json!({"operation":"upsert","policy_class":"measurement","source":"preset","preset_id":preset_id,"plant_id":policy.plant_id,"kind":policy.kind,"target_min":policy.target_min,"target_max":policy.target_max,"warning_low":policy.warning_low,"warning_high":policy.warning_high,"critical_low":policy.critical_low,"critical_high":policy.critical_high,"stale_after_ms":policy.stale_after_ms,"hysteresis":policy.hysteresis,"confirm_duration_ms":policy.confirm_duration_ms}), now).await?;
    }
    crate::repo::outbox::emit(&mut tx, EventKind::PLANT_UPDATED, &serde_json::json!({"operation":"apply_preset","plant_id":plant_id,"patch":{"profile_id":profile_id,"applied_preset_id":preset_id,"applied_catalogue_version":catalogue_version}}), now).await?;
    tx.commit().await.map_err(StorageError::from_sqlx)
}

// ------------------------------------------------------------ offline policy

pub async fn offline_policy(
    db: &EdgeDb,
    plant_id: &str,
) -> Result<Option<OfflinePolicyRow>, StorageError> {
    Ok(
        sqlx::query("SELECT * FROM offline_policies WHERE plant_id=?")
            .bind(plant_id)
            .fetch_optional(db.pool())
            .await
            .map_err(StorageError::from_sqlx)?
            .map(|r| OfflinePolicyRow {
                plant_id: r.get("plant_id"),
                policy_version: r.get("policy_version"),
                enabled: r.get::<i64, _>("enabled") != 0,
                policy_json: r.get("policy_json"),
                published_at: r.get("published_at"),
                applied_version: r.get("applied_version"),
                updated_at: r.get("updated_at"),
            }),
    )
}

/// Writes an authored policy at a strictly higher version.
///
/// `policy_version` is monotonic per plant: it is what a device uses to refuse a
/// retained replay of a policy it has already superseded (§5.11), so it may
/// never move backwards. Publication clears on every rewrite, because a
/// republished policy has not been published yet.
pub async fn upsert_offline_policy(
    db: &EdgeDb,
    plant_id: &str,
    policy_version: i64,
    enabled: bool,
    policy_json: &str,
    now: i64,
) -> Result<(), StorageError> {
    let mut tx = db.begin().await?;
    let done = sqlx::query(
        "INSERT INTO offline_policies(plant_id,policy_version,enabled,policy_json,published_at,applied_version,applied_at,updated_at) \
         VALUES(?,?,?,?,NULL,NULL,NULL,?) ON CONFLICT(plant_id) DO UPDATE SET \
         policy_version=excluded.policy_version,enabled=excluded.enabled,policy_json=excluded.policy_json, \
         published_at=NULL,updated_at=excluded.updated_at \
         WHERE excluded.policy_version > offline_policies.policy_version",
    )
    .bind(plant_id)
    .bind(policy_version)
    .bind(i64::from(enabled))
    .bind(policy_json)
    .bind(now)
    .execute(&mut *tx)
    .await
    .map_err(StorageError::from_sqlx)?;
    // The `WHERE` clause is a guard, not a formality: a stale version writes no
    // row at all. Emitting unconditionally would announce a policy change that
    // did not happen, so the emission follows the write rather than the call.
    if done.rows_affected() == 1 {
        crate::repo::outbox::emit(&mut tx, EventKind::PLANT_POLICY_CHANGED, &serde_json::json!({"operation":"upsert","policy_class":"offline","plant_id":plant_id,"policy_version":policy_version,"enabled":enabled}), now).await?;
    }
    tx.commit().await.map_err(StorageError::from_sqlx)
}

/// The next version to allocate for a plant.
pub async fn next_policy_version(db: &EdgeDb, plant_id: &str) -> Result<i64, StorageError> {
    let current = sqlx::query_scalar::<_, Option<i64>>(
        "SELECT policy_version FROM offline_policies WHERE plant_id=?",
    )
    .bind(plant_id)
    .fetch_optional(db.pool())
    .await
    .map_err(StorageError::from_sqlx)?
    .flatten()
    .unwrap_or(0);
    Ok(current.saturating_add(1))
}

/// Withdraws a plant's offline policy, and says so in the same transaction.
///
/// Deleting the policy is what takes offline autonomy away from a device
/// (ADR-015), so the version and enabled flag that were in force are carried in
/// the payload: without them the cloud cannot tell a withdrawal of a live
/// policy from the tidy-up of a disabled one.
pub async fn delete_offline_policy(
    db: &EdgeDb,
    plant_id: &str,
    now: i64,
) -> Result<bool, StorageError> {
    let mut tx = db.begin().await?;
    let previous: Option<(i64, i64)> =
        sqlx::query_as("SELECT policy_version,enabled FROM offline_policies WHERE plant_id=?")
            .bind(plant_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(StorageError::from_sqlx)?;
    let Some((policy_version, enabled)) = previous else {
        return Ok(false);
    };
    let done = sqlx::query("DELETE FROM offline_policies WHERE plant_id=?")
        .bind(plant_id)
        .execute(&mut *tx)
        .await
        .map_err(StorageError::from_sqlx)?;
    if done.rows_affected() != 1 {
        return Ok(false);
    }
    crate::repo::outbox::emit(&mut tx, EventKind::PLANT_POLICY_CHANGED, &serde_json::json!({"operation":"delete","policy_class":"offline","plant_id":plant_id,"policy_version":policy_version,"enabled":enabled != 0}), now).await?;
    tx.commit().await.map_err(StorageError::from_sqlx)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn db() -> EdgeDb {
        let db = EdgeDb::in_memory().await.unwrap();
        db.migrate().await.unwrap();
        crate::repo::outbox::configure(&db, true, 500_000)
            .await
            .unwrap();
        crate::repo::plant::create(
            &db,
            &crate::repo::plant::NewPlant {
                plant_id: "monstera-01".to_owned(),
                name: "Monstera".to_owned(),
                species: None,
                profile_id: None,
                pot_volume_ml: Some(2_500.0),
                soil_type: None,
            },
            1_000,
        )
        .await
        .unwrap();
        clear_outbox(&db).await;
        db
    }

    async fn clear_outbox(db: &EdgeDb) {
        sqlx::query("DELETE FROM pending_cloud_events")
            .execute(db.pool())
            .await
            .unwrap();
    }

    async fn events(db: &EdgeDb) -> Vec<(String, serde_json::Value)> {
        let rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT kind,payload_json FROM pending_cloud_events ORDER BY created_at,event_id",
        )
        .fetch_all(db.pool())
        .await
        .unwrap();
        rows.into_iter()
            .map(|(kind, payload)| (kind, serde_json::from_str(&payload).unwrap()))
            .collect()
    }

    fn sensor(binding_id: &str) -> SensorBindingRow {
        SensorBindingRow {
            binding_id: binding_id.to_owned(),
            plant_id: "monstera-01".to_owned(),
            device_id: "node-01".to_owned(),
            sensor_id: "soil-a".to_owned(),
            point: "default".to_owned(),
            kind: "soil_moisture".to_owned(),
            role: "control".to_owned(),
            created_at: 2_000,
        }
    }

    fn policy(kind: &str) -> MeasurementPolicyRow {
        MeasurementPolicyRow {
            plant_id: "monstera-01".to_owned(),
            kind: kind.to_owned(),
            target_min: Some(28.0),
            target_max: Some(45.0),
            warning_low: Some(24.0),
            warning_high: None,
            critical_low: Some(18.0),
            critical_high: None,
            stale_after_ms: 900_000,
            hysteresis: Some(2.0),
            confirm_duration_ms: Some(600_000),
        }
    }

    /// A delete is a state change, so it owes the cloud an event — and the event
    /// has to carry enough of the row that went away to be worth keeping.
    #[tokio::test]
    async fn deleting_a_sensor_binding_emits_the_canonical_kind_with_its_prior_identity() {
        let db = db().await;
        upsert_sensor_binding(&db, &sensor("b-1")).await.unwrap();
        clear_outbox(&db).await;

        assert!(delete_sensor_binding(&db, "b-1", 3_000).await.unwrap());
        let events = events(&db).await;
        assert_eq!(events.len(), 1);
        let (kind, payload) = &events[0];
        assert_eq!(kind, "plant.binding_changed");
        assert_eq!(payload["operation"], "delete");
        assert_eq!(payload["binding_class"], "sensor");
        assert_eq!(payload["plant_id"], "monstera-01");
        assert_eq!(payload["binding_id"], "b-1");
        assert_eq!(payload["device_id"], "node-01");
        assert_eq!(payload["sensor_id"], "soil-a");
        assert_eq!(payload["point"], "default");
        assert_eq!(payload["kind"], "soil_moisture");
        assert_eq!(payload["role"], "control");
        assert_eq!(payload["bound_at"], 2_000);
        assert!(sensor_binding(&db, "b-1").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn deleting_an_actuator_binding_emits_the_canonical_kind_with_its_prior_identity() {
        let db = db().await;
        upsert_actuator_binding(
            &db,
            &ActuatorBindingRow {
                plant_id: "monstera-01".to_owned(),
                device_id: "node-01".to_owned(),
                actuator_id: "pump-a".to_owned(),
                kind: "pump".to_owned(),
                created_at: 2_000,
            },
        )
        .await
        .unwrap();
        clear_outbox(&db).await;

        assert!(
            delete_actuator_binding(&db, "monstera-01", 3_000)
                .await
                .unwrap()
        );
        let events = events(&db).await;
        assert_eq!(events.len(), 1);
        let (kind, payload) = &events[0];
        assert_eq!(kind, "plant.binding_changed");
        assert_eq!(payload["operation"], "delete");
        assert_eq!(payload["binding_class"], "actuator");
        assert_eq!(payload["plant_id"], "monstera-01");
        assert_eq!(payload["device_id"], "node-01");
        assert_eq!(payload["actuator_id"], "pump-a");
        assert_eq!(payload["kind"], "pump");
        assert!(
            actuator_binding(&db, "monstera-01")
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn deleting_a_measurement_policy_emits_the_thresholds_that_went_away() {
        let db = db().await;
        upsert_measurement_policy(&db, &policy("soil_moisture"), 2_000)
            .await
            .unwrap();
        clear_outbox(&db).await;

        assert!(
            delete_measurement_policy(&db, "monstera-01", "soil_moisture", 3_000)
                .await
                .unwrap()
        );
        let events = events(&db).await;
        assert_eq!(events.len(), 1);
        let (kind, payload) = &events[0];
        assert_eq!(kind, "plant.policy_changed");
        assert_eq!(payload["operation"], "delete");
        assert_eq!(payload["policy_class"], "measurement");
        assert_eq!(payload["plant_id"], "monstera-01");
        assert_eq!(payload["kind"], "soil_moisture");
        assert_eq!(payload["target_min"], 28.0);
        assert_eq!(payload["critical_low"], 18.0);
        assert_eq!(payload["stale_after_ms"], 900_000);
        assert_eq!(payload["confirm_duration_ms"], 600_000);
        assert!(
            measurement_policy(&db, "monstera-01", "soil_moisture")
                .await
                .unwrap()
                .is_none()
        );
    }

    /// Nothing changed, so nothing happened, so there is nothing to announce.
    /// An event emitted here would be a fact that is not true.
    #[tokio::test]
    async fn deleting_a_row_that_is_not_there_emits_nothing() {
        let db = db().await;
        assert!(!delete_sensor_binding(&db, "absent", 3_000).await.unwrap());
        assert!(
            !delete_actuator_binding(&db, "monstera-01", 3_000)
                .await
                .unwrap()
        );
        assert!(
            !delete_measurement_policy(&db, "monstera-01", "soil_moisture", 3_000)
                .await
                .unwrap()
        );
        assert!(
            !delete_offline_policy(&db, "monstera-01", 3_000)
                .await
                .unwrap()
        );
        assert!(events(&db).await.is_empty());
    }

    /// Repeating a delete is not a second change. The first call emits; the
    /// second finds nothing to remove and says so without inventing an event.
    #[tokio::test]
    async fn a_repeated_delete_emits_once() {
        let db = db().await;
        upsert_sensor_binding(&db, &sensor("b-1")).await.unwrap();
        clear_outbox(&db).await;
        assert!(delete_sensor_binding(&db, "b-1", 3_000).await.unwrap());
        assert!(!delete_sensor_binding(&db, "b-1", 4_000).await.unwrap());
        assert_eq!(events(&db).await.len(), 1);
    }

    /// With the cloud disabled there is no outbox row at all — the delete still
    /// happens, and nothing accumulates for a drain that will never run.
    #[tokio::test]
    async fn a_delete_with_the_cloud_disabled_writes_no_outbox_row() {
        let db = db().await;
        upsert_sensor_binding(&db, &sensor("b-1")).await.unwrap();
        crate::repo::outbox::configure(&db, false, 500_000)
            .await
            .unwrap();
        clear_outbox(&db).await;
        assert!(delete_sensor_binding(&db, "b-1", 3_000).await.unwrap());
        assert!(events(&db).await.is_empty());
        assert!(sensor_binding(&db, "b-1").await.unwrap().is_none());
    }

    /// The state change and its event share one transaction, so a failure
    /// leaves neither. Here the failure is a foreign key: the plant is gone, so
    /// the policy insert cannot stand, and the preset's events must go with it.
    #[tokio::test]
    async fn a_rolled_back_change_leaves_neither_the_state_nor_the_event() {
        let db = db().await;
        let failed = materialize_preset(
            &db,
            "no-such-plant",
            &[MeasurementPolicyRow {
                plant_id: "no-such-plant".to_owned(),
                ..policy("soil_moisture")
            }],
            None,
            "profile-private-01",
            "Monstera preset",
            "{}",
            "monstera-deliciosa",
            1,
            3_000,
        )
        .await;
        assert!(failed.is_err());
        assert!(events(&db).await.is_empty());
        let policies: i64 =
            sqlx::query_scalar("SELECT count(*) FROM measurement_policies WHERE plant_id=?")
                .bind("no-such-plant")
                .fetch_one(db.pool())
                .await
                .unwrap();
        assert_eq!(policies, 0);
        let profiles: i64 =
            sqlx::query_scalar("SELECT count(*) FROM plant_profiles WHERE profile_id=?")
                .bind("profile-private-01")
                .fetch_one(db.pool())
                .await
                .unwrap();
        assert_eq!(profiles, 0);
    }

    /// A preset writes policy rows through a different door; it owes the same
    /// events, and the plant re-point it performs is a `plant.updated`.
    #[tokio::test]
    async fn materialising_a_preset_emits_one_policy_event_per_row_and_one_plant_update() {
        let db = db().await;
        materialize_preset(
            &db,
            "monstera-01",
            &[policy("soil_moisture"), policy("air_temperature")],
            None,
            "profile-private-01",
            "Monstera preset",
            "{}",
            "monstera-deliciosa",
            1,
            3_000,
        )
        .await
        .unwrap();
        let events = events(&db).await;
        let policy_events: Vec<_> = events
            .iter()
            .filter(|(kind, _)| kind == "plant.policy_changed")
            .collect();
        assert_eq!(policy_events.len(), 2);
        for (_, payload) in &policy_events {
            assert_eq!(payload["operation"], "upsert");
            assert_eq!(payload["source"], "preset");
            assert_eq!(payload["preset_id"], "monstera-deliciosa");
        }
        let updates: Vec<_> = events
            .iter()
            .filter(|(kind, _)| kind == "plant.updated")
            .collect();
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].1["operation"], "apply_preset");
    }

    /// A stale offline policy writes no row, so it announces no change. The
    /// version guard and the emission have to agree, or the cloud learns about
    /// a policy the edge refused to store.
    #[tokio::test]
    async fn a_refused_offline_policy_version_emits_nothing() {
        let db = db().await;
        upsert_offline_policy(&db, "monstera-01", 2, true, "{}", 2_000)
            .await
            .unwrap();
        clear_outbox(&db).await;
        upsert_offline_policy(&db, "monstera-01", 1, true, "{}", 3_000)
            .await
            .unwrap();
        assert!(events(&db).await.is_empty());
        assert_eq!(
            offline_policy(&db, "monstera-01")
                .await
                .unwrap()
                .unwrap()
                .policy_version,
            2
        );

        upsert_offline_policy(&db, "monstera-01", 3, true, "{}", 4_000)
            .await
            .unwrap();
        let events = events(&db).await;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].0, "plant.policy_changed");
        assert_eq!(events[0].1["policy_class"], "offline");
        assert_eq!(events[0].1["policy_version"], 3);
    }
}
