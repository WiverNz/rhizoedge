//! Plant configuration, analysis, and the inputs the recommendation engine
//! consumes (M5-006 … M5-015).
//!
//! Everything here reads storage and calls `rhizo-domain`. The decisions live in
//! the domain; this module is the adapter that assembles their inputs and
//! persists their conclusions.
//!
//! # M5 issues no commands
//!
//! Nothing in this module or its children publishes to any MQTT topic. That is
//! not an accident of the current implementation — it is the property that lets
//! the recommendation logic be validated against a real plant for a week before
//! M6 gives it a pump, and `tests/no_commands_in_m5.rs` fails if it stops being
//! true.
pub mod detect;
pub mod preset;
pub mod state;

use chrono::{DateTime, Duration, Utc};
use rhizo_domain::binding::{
    DeclaredActuator, DeclaredCapabilities, DeclaredSensor, required_kinds,
};
use rhizo_domain::dry_duration::{DryConfig, DryDuration};
use rhizo_domain::plant::{ActuatorBinding, BindingRole, MeasurementPolicy, SensorBinding};
use rhizo_domain::profile::PlantProfile;
use rhizo_domain::recommend::RecommendationInputs;
use rhizo_domain::trend::{self, TrendSample, TrendVwcPerHour};
use rhizo_mqtt_contract::DeviceId;
use rhizo_mqtt_contract::payload::{MeasurementKind, MeasurementPoint, SensorId};
use rhizo_storage::EdgeDb;
use rhizo_storage::repo::{binding as binding_repo, plant as plant_repo, query};

/// Decodes a stored kind name. An unrecognised name is preserved, never dropped
/// and never guessed at ([ADR-017](../../../../docs/adr/017-extensible-measurement-model.md)).
#[must_use]
pub fn kind_from_str(name: &str) -> MeasurementKind {
    serde_json::from_value(serde_json::Value::String(name.to_owned()))
        .unwrap_or_else(|_| MeasurementKind::Unknown(name.to_owned()))
}

/// Decodes a stored role. An unrecognised role is `advisory`, which is the role
/// that gates nothing (SAFETY-012).
#[must_use]
pub fn role_from_str(name: &str) -> BindingRole {
    match name {
        "control" => BindingRole::Control,
        "required" => BindingRole::Required,
        _ => BindingRole::Advisory,
    }
}

/// The wire name of a role.
#[must_use]
pub const fn role_name(role: BindingRole) -> &'static str {
    match role {
        BindingRole::Control => "control",
        BindingRole::Required => "required",
        BindingRole::Advisory => "advisory",
    }
}

/// A stored binding, decoded into its domain form alongside its row identity.
#[derive(Clone, Debug)]
pub struct BoundSensor {
    /// Row identity, so an edit or a delete can name it.
    pub binding_id: String,
    /// The domain binding.
    pub binding: SensorBinding,
}

/// Everything one plant is configured with.
#[derive(Clone, Debug)]
pub struct Loaded {
    /// The plant row.
    pub plant: plant_repo::PlantRow,
    /// The profile template, or the built-in default when the plant names none.
    pub profile: PlantProfile,
    /// Sensor bindings, decoded.
    pub sensors: Vec<BoundSensor>,
    /// The optional actuator binding. `None` is a normal monitoring plant.
    pub actuator: Option<ActuatorBinding>,
    /// Per-measurement policies, decoded.
    pub policies: Vec<MeasurementPolicy>,
}

impl Loaded {
    /// The `control`-role binding, if the plant has one.
    #[must_use]
    pub fn control(&self) -> Option<&BoundSensor> {
        self.sensors
            .iter()
            .find(|b| b.binding.role == BindingRole::Control)
    }
    /// The domain bindings, for the rules that take a slice.
    #[must_use]
    pub fn bindings(&self) -> Vec<SensorBinding> {
        self.sensors.iter().map(|b| b.binding.clone()).collect()
    }
    /// The policy for one kind, if configured.
    #[must_use]
    pub fn policy(&self, kind: &MeasurementKind) -> Option<&MeasurementPolicy> {
        self.policies.iter().find(|p| &p.kind == kind)
    }
    /// A binding for one kind, whatever its role.
    #[must_use]
    pub fn binding_for(&self, kind: &MeasurementKind) -> Option<&BoundSensor> {
        self.sensors.iter().find(|b| &b.binding.kind == kind)
    }
}

/// Reads a stored policy row into its domain form.
#[must_use]
pub fn policy_from_row(row: &binding_repo::MeasurementPolicyRow) -> MeasurementPolicy {
    MeasurementPolicy {
        kind: kind_from_str(&row.kind),
        target_min: row.target_min,
        target_max: row.target_max,
        warning_low: row.warning_low,
        warning_high: row.warning_high,
        critical_low: row.critical_low,
        critical_high: row.critical_high,
        stale_after_ms: u32::try_from(row.stale_after_ms).unwrap_or(u32::MAX),
        hysteresis: row.hysteresis,
        confirm_duration_ms: row
            .confirm_duration_ms
            .map(|v| u32::try_from(v).unwrap_or(u32::MAX)),
    }
}

/// Writes a domain policy back into its stored form.
#[must_use]
pub fn policy_to_row(
    plant_id: &str,
    policy: &MeasurementPolicy,
) -> binding_repo::MeasurementPolicyRow {
    binding_repo::MeasurementPolicyRow {
        plant_id: plant_id.to_owned(),
        kind: policy.kind.as_str().to_owned(),
        target_min: policy.target_min,
        target_max: policy.target_max,
        warning_low: policy.warning_low,
        warning_high: policy.warning_high,
        critical_low: policy.critical_low,
        critical_high: policy.critical_high,
        stale_after_ms: i64::from(policy.stale_after_ms),
        hysteresis: policy.hysteresis,
        confirm_duration_ms: policy.confirm_duration_ms.map(i64::from),
    }
}

/// Loads a plant and everything configured on it.
///
/// A plant that names no profile gets the built-in default template. A profile
/// is a *starting point*, and a plant without one is configured entirely by its
/// own `MeasurementPolicy` rows — which is the ADR-016 model working as
/// intended, not a degraded state.
pub async fn load(
    db: &EdgeDb,
    plant_id: &str,
) -> Result<Option<Loaded>, rhizo_storage::StorageError> {
    let Some(plant) = plant_repo::get(db, plant_id).await? else {
        return Ok(None);
    };
    let profile = match plant.profile_id.as_deref() {
        Some(id) => rhizo_storage::repo::profile::get(db, id)
            .await?
            .and_then(|row| serde_json::from_str::<PlantProfile>(&row.profile_json).ok())
            .unwrap_or_else(default_profile),
        None => default_profile(),
    };
    let sensors = binding_repo::sensor_bindings(db, plant_id)
        .await?
        .into_iter()
        .filter_map(|row| {
            Some(BoundSensor {
                binding_id: row.binding_id.clone(),
                binding: SensorBinding {
                    device_id: DeviceId::parse(&row.device_id).ok()?,
                    sensor_id: SensorId::parse(&row.sensor_id).ok()?,
                    point: MeasurementPoint::parse(&row.point).ok()?,
                    kind: kind_from_str(&row.kind),
                    role: role_from_str(&row.role),
                },
            })
        })
        .collect();
    let actuator = binding_repo::actuator_binding(db, plant_id)
        .await?
        .and_then(|row| {
            Some(ActuatorBinding {
                device_id: DeviceId::parse(&row.device_id).ok()?,
                actuator_id: SensorId::parse(&row.actuator_id).ok()?,
                kind: serde_json::from_value(serde_json::Value::String(row.kind)).ok()?,
            })
        });
    let policies = binding_repo::measurement_policies(db, plant_id)
        .await?
        .iter()
        .map(policy_from_row)
        .collect();
    Ok(Some(Loaded {
        plant,
        profile,
        sensors,
        actuator,
        policies,
    }))
}

/// The template a plant falls back to when it names no profile.
#[must_use]
pub fn default_profile() -> PlantProfile {
    PlantProfile::default_seed(rhizo_domain::ProfileId::from_uuid(uuid::Uuid::nil()))
}

/// The declared capabilities a binding may name, read from the M4 registry.
pub async fn declared_capabilities(
    db: &EdgeDb,
) -> Result<DeclaredCapabilities, rhizo_storage::StorageError> {
    let rows = binding_repo::declared_capabilities(db).await?;
    let mut declared = DeclaredCapabilities::default();
    for row in rows {
        let kinds: Vec<MeasurementKind> = serde_json::from_str(&row.kinds_json).unwrap_or_default();
        if row.class == "sensor" {
            declared.sensors.push(DeclaredSensor {
                device_id: row.device_id,
                sensor_id: row.capability_id,
                point: row.point.unwrap_or_else(|| "default".to_owned()),
                kinds,
            });
        } else if row.class == "actuator" {
            let kind = kinds
                .first()
                .map(rhizo_mqtt_contract::payload::MeasurementKind::as_str)
                .and_then(|k| serde_json::from_value(serde_json::Value::String(k.to_owned())).ok())
                .unwrap_or(rhizo_mqtt_contract::payload::ActuatorKind::Unknown);
            declared.actuators.push(DeclaredActuator {
                device_id: row.device_id,
                actuator_id: row.capability_id,
                kind,
            });
        }
    }
    Ok(declared)
}

/// The trend window every fit in M5 uses.
#[must_use]
pub fn trend_window() -> Duration {
    trend::default_window()
}

/// The control-freshness threshold for this plant, in milliseconds.
///
/// **SAFETY-005.** Two numbers answer the same question and the stricter one
/// wins: the plant's own `stale_after_ms`, and the device cadence bound
/// `max(15 min, 3 x telemetry interval)`. Note which function is called —
/// [`crate::device::health::max_sample_age_seconds`] takes a *telemetry* cadence
/// and nothing else. `liveness_interval_seconds` is the other formula, it is
/// widened by a battery device's declared wake interval, and it answers a
/// different question. Feeding it in here would let a device advertise itself a
/// three-day freshness window (PRD 040 F-040-26).
#[must_use]
pub fn control_freshness_ms(policy_stale_after_ms: u32, telemetry_interval_seconds: i64) -> i64 {
    let from_cadence =
        crate::device::health::max_sample_age_seconds(telemetry_interval_seconds) * 1_000;
    i64::from(policy_stale_after_ms).min(from_cadence).max(1)
}

/// A stored row read as a domain trend sample.
#[must_use]
pub fn to_trend_sample(row: &query::MeasurementRow) -> TrendSample {
    TrendSample {
        // A row with no numeric value is a failed or rejected read. It is
        // excluded from the fit rather than fitted as zero.
        value: row.value_num.unwrap_or(f64::NAN),
        at: chrono::DateTime::from_timestamp_millis(row.received_at).unwrap_or_default(),
        valid: row.value_num.is_some_and(f64::is_finite) && row.quality == "ok",
    }
}

/// Everything the tick needed to compute, kept together so the API can report
/// the same numbers the engine used.
#[derive(Clone, Debug)]
pub struct Analysis {
    /// The engine inputs.
    pub inputs: RecommendationInputs,
    /// The dry-duration accumulator after this evaluation.
    pub dry: DryDuration,
    /// The control kind's latest reading, for display.
    pub latest_moisture: Option<f64>,
    /// The EC trend, when one could be fitted.
    pub ec_trend: Option<rhizo_domain::ec::TrendUsCmPerHour>,
    /// The latest EC reading, for display and for the warning.
    pub latest_ec: Option<f64>,
}

/// Assembles the recommendation inputs for one plant.
///
/// Pure decisions stay in the domain; this function only reads. `now` is the
/// edge clock, supplied by the caller — every age below is measured against
/// **edge receipt time**, never a device timestamp (SAFETY-005).
pub async fn analyse(
    db: &EdgeDb,
    loaded: &Loaded,
    now: DateTime<Utc>,
) -> Result<Analysis, rhizo_storage::StorageError> {
    let now_ms = now.timestamp_millis();
    let plant_id = loaded.plant.plant_id.as_str();
    let control = loaded.control();

    let mut latest_moisture = None;
    let mut latest_invalid = false;
    let mut sample_age = None;
    let mut trend_value: Option<TrendVwcPerHour> = None;
    let mut samples_in_window = 0;
    let mut freshness_ms = i64::from(u32::MAX);
    let mut control_rows: Vec<query::MeasurementRow> = Vec::new();

    if let Some(bound) = control {
        let device = bound.binding.device_id.to_string();
        let sensor = bound.binding.sensor_id.as_str();
        let point = bound.binding.point.as_str();
        let kind = bound.binding.kind.as_str();
        let cadence = query::telemetry_interval_seconds(db, &device)
            .await?
            .unwrap_or(300);
        let stale_after = loaded
            .policy(&bound.binding.kind)
            .map_or(u32::MAX, |p| p.stale_after_ms);
        freshness_ms = control_freshness_ms(stale_after, cadence);
        if let Some(latest) = query::latest_measurement(db, &device, sensor, point, kind).await? {
            sample_age = Some(Duration::milliseconds(
                now_ms.saturating_sub(latest.received_at).max(0),
            ));
            match latest.value_num.filter(|v| v.is_finite()) {
                Some(value) if latest.quality == "ok" => latest_moisture = Some(value),
                _ => latest_invalid = true,
            }
        }
        let window = trend_window();
        let rows = query::measurements_for(
            db,
            &device,
            sensor,
            point,
            kind,
            now_ms - window.num_milliseconds(),
            now_ms,
            5_000,
        )
        .await?;
        let samples: Vec<TrendSample> = rows.iter().map(to_trend_sample).collect();
        samples_in_window = samples.iter().filter(|s| s.valid).count();
        trend_value = trend::fit(&samples, window).map(|t| TrendVwcPerHour(t.per_hour));
        control_rows = rows;
    }

    // The dry-duration accumulator, advanced by the observed interval only.
    let stored = plant_repo::dry_state(db, plant_id).await?;
    let mut dry = DryDuration {
        dry_ms: stored.dry_ms,
        last_sample_at: stored
            .last_sample_at
            .and_then(DateTime::from_timestamp_millis),
    };
    let target_min = control
        .and_then(|b| loaded.policy(&b.binding.kind))
        .and_then(|p| p.target_min)
        .unwrap_or(loaded.profile.target_min_vwc);
    // Every sample not yet folded in, oldest first. Advancing one reading per
    // *tick* would make the debounce a property of how often the loop runs
    // rather than of what the plant did: a 30-second tick and a 5-minute cadence
    // would over-count, and a slow tick would under-count. Folding the observed
    // series instead means the accumulator answers the same thing whatever the
    // loop's timing, and a restart replays cleanly from the last sample it saw.
    let config = DryConfig {
        target_min,
        stale_after: Duration::milliseconds(freshness_ms),
    };
    let already_seen = dry
        .last_sample_at
        .map_or(i64::MIN, |v| v.timestamp_millis());
    for row in control_rows
        .iter()
        .filter(|row| row.received_at > already_seen)
    {
        let value = row
            .value_num
            .filter(|v| v.is_finite() && row.quality == "ok");
        let at = DateTime::from_timestamp_millis(row.received_at).unwrap_or(now);
        dry.observe(value, at, &config);
    }
    plant_repo::put_dry_state(
        db,
        plant_id,
        plant_repo::DryStateRow {
            dry_ms: dry.dry_ms,
            last_sample_at: dry.last_sample_at.map(|v| v.timestamp_millis()),
        },
        now_ms,
    )
    .await?;

    // Required-role bindings must be healthy and have a usable, fresh sample.
    // Device health alone is not measurement evidence: a healthy leak or tank
    // sensor with no current value is still uncertainty (SAFETY-012).
    let mut required_healthy = true;
    for bound in loaded
        .sensors
        .iter()
        .filter(|b| b.binding.role == BindingRole::Required)
    {
        let device = bound.binding.device_id.to_string();
        let healthy = query::sensor_healthy(db, &device, bound.binding.sensor_id.as_str()).await?;
        if healthy != Some(true) {
            required_healthy = false;
            continue;
        }
        let Some(policy) = loaded.policy(&bound.binding.kind) else {
            required_healthy = false;
            continue;
        };
        let cadence = query::telemetry_interval_seconds(db, &device)
            .await?
            .unwrap_or(300);
        let max_age = control_freshness_ms(policy.stale_after_ms, cadence);
        let latest = query::latest_measurement(
            db,
            &device,
            bound.binding.sensor_id.as_str(),
            bound.binding.point.as_str(),
            bound.binding.kind.as_str(),
        )
        .await?;
        let usable = latest.is_some_and(|row| {
            let has_typed_value = row.value_num.is_some_and(f64::is_finite)
                || row.value_bool.is_some_and(|value| matches!(value, 0 | 1));
            row.quality == "ok"
                && has_typed_value
                && now_ms.saturating_sub(row.received_at).max(0) < max_age
        });
        if !usable {
            required_healthy = false;
        }
    }

    let last_watering = plant_repo::last_watering_at(db, plant_id).await?;
    let time_since_last_watering =
        last_watering.map(|at| Duration::milliseconds(now_ms.saturating_sub(at).max(0)));

    // EC is recorded, trended, and warned about. It reaches no decision below.
    let mut latest_ec = None;
    let mut ec_trend = None;
    if let Some(bound) = loaded.binding_for(&MeasurementKind::SoilEc) {
        let device = bound.binding.device_id.to_string();
        let sensor = bound.binding.sensor_id.as_str();
        let point = bound.binding.point.as_str();
        latest_ec = query::latest_measurement(db, &device, sensor, point, "soil_ec")
            .await?
            .and_then(|r| r.value_num);
        let window = trend_window();
        let rows = query::measurements_for(
            db,
            &device,
            sensor,
            point,
            "soil_ec",
            now_ms - window.num_milliseconds(),
            now_ms,
            5_000,
        )
        .await?;
        let samples: Vec<TrendSample> = rows.iter().map(to_trend_sample).collect();
        ec_trend = rhizo_domain::ec::ec_trend(&samples, window);
    }

    let inputs = RecommendationInputs {
        moisture_vwc: latest_moisture,
        latest_sample_invalid: latest_invalid,
        sample_age,
        max_sample_age: Duration::milliseconds(freshness_ms),
        target_min,
        dry_duration: dry.duration(),
        dry_confirm: Duration::minutes(i64::from(loaded.profile.dry_confirm_minutes)),
        time_since_last_watering,
        cooldown: Duration::milliseconds(
            (loaded.profile.cooldown_hours * 3_600_000.0).max(0.0) as i64
        ),
        dose_ml: loaded.profile.dose_ml,
        has_actuator: loaded.actuator.is_some(),
        required_sensors_healthy: required_healthy,
        lockout: loaded
            .plant
            .lockout_reason
            .as_deref()
            .and_then(lockout_from_str),
        trend: trend_value,
        samples_in_window,
        has_weight_sensor: loaded.binding_for(&MeasurementKind::PotWeight).is_some(),
    };
    Ok(Analysis {
        inputs,
        dry,
        latest_moisture,
        ec_trend,
        latest_ec,
    })
}

/// Decodes a stored lockout name. An unrecognised lockout is `Unknown`, which
/// still blocks: an unreadable lockout is not an absent one (SAFETY-012).
#[must_use]
pub fn lockout_from_str(name: &str) -> Option<rhizo_domain::LockoutReason> {
    serde_json::from_value(serde_json::Value::String(name.to_owned())).ok()
}

/// The wire name of a lockout.
#[must_use]
pub fn lockout_name(reason: rhizo_domain::LockoutReason) -> String {
    serde_json::to_value(reason)
        .ok()
        .and_then(|v| v.as_str().map(ToOwned::to_owned))
        .unwrap_or_else(|| "unknown".to_owned())
}

/// The kinds a plant's `required` bindings cover, for the offline policy.
#[must_use]
pub fn plant_required_kinds(loaded: &Loaded) -> Vec<MeasurementKind> {
    required_kinds(&loaded.bindings())
}
