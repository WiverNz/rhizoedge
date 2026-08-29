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
    .execute(db.pool())
    .await
    .map_err(StorageError::from_sqlx)?;
    Ok(())
}

pub async fn delete_sensor_binding(db: &EdgeDb, binding_id: &str) -> Result<bool, StorageError> {
    let done = sqlx::query("DELETE FROM sensor_bindings WHERE binding_id=?")
        .bind(binding_id)
        .execute(db.pool())
        .await
        .map_err(StorageError::from_sqlx)?;
    Ok(done.rows_affected() == 1)
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
    sqlx::query(
        "INSERT INTO actuator_bindings(plant_id,device_id,actuator_id,kind,created_at) VALUES(?,?,?,?,?) \
         ON CONFLICT(plant_id) DO UPDATE SET device_id=excluded.device_id,actuator_id=excluded.actuator_id,kind=excluded.kind",
    )
    .bind(&binding.plant_id)
    .bind(&binding.device_id)
    .bind(&binding.actuator_id)
    .bind(&binding.kind)
    .bind(binding.created_at)
    .execute(db.pool())
    .await
    .map_err(StorageError::from_sqlx)?;
    Ok(())
}

pub async fn delete_actuator_binding(db: &EdgeDb, plant_id: &str) -> Result<bool, StorageError> {
    let done = sqlx::query("DELETE FROM actuator_bindings WHERE plant_id=?")
        .bind(plant_id)
        .execute(db.pool())
        .await
        .map_err(StorageError::from_sqlx)?;
    Ok(done.rows_affected() == 1)
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
    .execute(db.pool())
    .await
    .map_err(StorageError::from_sqlx)?;
    Ok(())
}

pub async fn delete_measurement_policy(
    db: &EdgeDb,
    plant_id: &str,
    kind: &str,
) -> Result<bool, StorageError> {
    let done = sqlx::query("DELETE FROM measurement_policies WHERE plant_id=? AND kind=?")
        .bind(plant_id)
        .bind(kind)
        .execute(db.pool())
        .await
        .map_err(StorageError::from_sqlx)?;
    Ok(done.rows_affected() == 1)
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
    sqlx::query(
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
    .execute(db.pool())
    .await
    .map_err(StorageError::from_sqlx)?;
    Ok(())
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

pub async fn delete_offline_policy(db: &EdgeDb, plant_id: &str) -> Result<bool, StorageError> {
    let done = sqlx::query("DELETE FROM offline_policies WHERE plant_id=?")
        .bind(plant_id)
        .execute(db.pool())
        .await
        .map_err(StorageError::from_sqlx)?;
    Ok(done.rows_affected() == 1)
}
