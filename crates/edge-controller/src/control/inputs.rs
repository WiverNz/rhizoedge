//! Assembling [`IrrigationInputs`] from storage.
//!
//! The adapter between "what SQLite holds" and "what the pure gate reads". Every
//! age below is measured against the **edge** `received_at` and never against a
//! device timestamp: a device with a backwards clock must not be able to make
//! stale data look fresh (SAFETY-005).
//!
//! # Bindings own the whole identity
//!
//! Every read names the complete `(device_id, sensor_id, point, kind)` a binding
//! declares. A second, unbound probe on the same device reporting the same kind
//! at the same point is not evidence about this plant, and the M5 review found
//! exactly that hole in the recommendation path. It is not reopened here.
//!
//! # What is absent stays absent
//!
//! Nothing here substitutes a default for a missing reading. A silent leak
//! sensor produces [`LeakState::Unknown`], a silent tank produces
//! [`TankState::Unknown`], and both refuse (SAFETY-012). The one place a
//! fallback appears is the freshness *threshold*, which falls back to the
//! stricter of the plant's policy and the device cadence — never to a longer
//! window.

use chrono::{DateTime, Duration, Utc};
use rhizo_domain::irrigation::types::{
    EvaluationMode, IrrigationInputs, LeakState, RequiredInput, RequiredInputState, TankState,
    WeightSample, state_from_str,
};
use rhizo_domain::plant::{
    ActuatorBinding, AutomationPolicy, BindingRole, MeasurementPolicy, SensorBinding,
};
use rhizo_domain::profile::SoilSample;
use rhizo_domain::state::IrrigationState;
use rhizo_mqtt_contract::payload::MeasurementKind;
use rhizo_storage::EdgeDb;
use rhizo_storage::repo::{command as command_repo, query};

use crate::device::connectivity;
use crate::plant::{self, Loaded, control_freshness_ms};

/// Everything the gate reads, owned so the borrowed view can be built from it.
///
/// `IrrigationInputs` is a borrow of this. Keeping the two apart means the gate
/// stays a pure function over data, and the database work happens exactly once
/// per tick rather than once per check.
#[derive(Clone, Debug)]
pub struct Gathered {
    /// The persisted irrigation state, or the starting one for a new plant.
    pub state: IrrigationState,
    /// The plant's automation configuration.
    pub automation: AutomationPolicy,
    /// The latest control reading.
    pub soil: Option<SoilSample>,
    /// The reading taken before the current cycle's first dose.
    pub pre_dose_soil: Option<SoilSample>,
    /// The latest pot-scale reading.
    pub weight: Option<WeightSample>,
    /// The pot-scale reading taken before the current cycle's first dose.
    pub pre_dose_weight: Option<WeightSample>,
    /// The reservoir.
    pub tank: Option<TankState>,
    /// The leak signal.
    pub leak: LeakState,
    /// The plant's sensor bindings.
    pub bindings: Vec<SensorBinding>,
    /// The optional actuation route.
    pub actuator: Option<ActuatorBinding>,
    /// Per-measurement policies.
    pub policies: Vec<MeasurementPolicy>,
    /// Required measurements other than leak and tank.
    pub required: Vec<RequiredInput>,
    /// The rolling 24-hour total, derived from rows.
    pub delivered_last_24h_ml: f32,
    /// Doses delivered in the current cycle.
    pub doses_this_cycle: u8,
    /// When the last cycle completed.
    pub last_cycle_completed_at: Option<DateTime<Utc>>,
    /// The absorption deadline in force.
    pub wait_until: Option<DateTime<Utc>>,
    /// The operator's opt-in.
    pub auto_watering_enabled: bool,
    /// Whether the actuator's device is reachable.
    pub device_online: bool,
    /// Whether the actuator's device is asleep inside its own wake window.
    pub device_sleeping: bool,
    /// Continuous observed dryness.
    pub dry_duration: Duration,
    /// Whether any bound device is still replaying buffered history.
    pub reconciling: bool,
    /// The lockout persisted against this plant.
    pub active_lockout: Option<rhizo_domain::state::LockoutReason>,
    /// How long that lockout is held regardless of its condition.
    pub lockout_held_until: Option<DateTime<Utc>>,
    /// The command currently in flight, if any.
    pub active_command_id: Option<String>,
    /// The device the actuator lives on, for convenience.
    pub actuator_device: Option<String>,
}

impl Gathered {
    /// The borrowed view the pure functions take.
    #[must_use]
    pub fn inputs(&self, now: DateTime<Utc>, mode: EvaluationMode) -> IrrigationInputs<'_> {
        IrrigationInputs {
            now,
            state: &self.state,
            mode,
            latest_soil: self.soil.as_ref(),
            pre_dose_soil: self.pre_dose_soil.as_ref(),
            latest_weight: self.weight.as_ref(),
            pre_dose_weight: self.pre_dose_weight.as_ref(),
            tank: self.tank,
            leak: self.leak,
            sensor_bindings: &self.bindings,
            actuator_binding: self.actuator.as_ref(),
            measurement_policies: &self.policies,
            automation: &self.automation,
            delivered_last_24h_ml: self.delivered_last_24h_ml,
            doses_this_cycle: self.doses_this_cycle,
            last_cycle_completed_at: self.last_cycle_completed_at,
            wait_until: self.wait_until,
            auto_watering_enabled: self.auto_watering_enabled,
            device_online: self.device_online,
            dry_duration: self.dry_duration,
            reconciling: self.reconciling,
            required_inputs: &self.required,
            active_lockout: self.active_lockout,
            lockout_held_until: self.lockout_held_until,
        }
    }

    /// The stored irrigation state as a row, ready to be transitioned.
    #[must_use]
    pub fn state_row(&self, now_ms: i64) -> command_repo::IrrigationStateRow {
        command_repo::IrrigationStateRow {
            state: rhizo_domain::irrigation::types::state_name(self.state).to_owned(),
            state_since: now_ms,
            doses_this_cycle: i64::from(self.doses_this_cycle),
            cycle_started_at: None,
            last_cycle_completed_at: self.last_cycle_completed_at.map(|v| v.timestamp_millis()),
            wait_until: self.wait_until.map(|v| v.timestamp_millis()),
            active_command_id: self.active_command_id.clone(),
            pre_dose_vwc: self.pre_dose_soil.and_then(|s| s.moisture_vwc),
            pre_dose_grams: self.pre_dose_weight.and_then(|s| s.grams),
        }
    }
}

/// Reads everything one plant's watering decision may consider.
pub async fn gather(
    db: &EdgeDb,
    loaded: &Loaded,
    dry_duration: Duration,
    now: DateTime<Utc>,
) -> Result<Gathered, rhizo_storage::StorageError> {
    let now_ms = now.timestamp_millis();
    let plant_id = loaded.plant.plant_id.as_str();

    let stored = command_repo::irrigation_state(db, plant_id).await?;
    let state = stored
        .as_ref()
        .map_or(IrrigationState::Normal, |row| state_from_str(&row.state));

    // The control reading, from the exact bound stream.
    let mut soil = None;
    if let Some(bound) = loaded.control() {
        soil = read_scalar(db, &bound.binding, now_ms)
            .await?
            .map(|(value, received_at)| SoilSample {
                moisture_vwc: value,
                received_at,
            });
    }

    // The pot scale, where one is bound.
    let mut weight = None;
    if let Some(bound) = loaded.binding_for(&MeasurementKind::PotWeight) {
        weight = read_scalar(db, &bound.binding, now_ms)
            .await?
            .map(|(value, received_at)| WeightSample {
                grams: value,
                received_at,
            });
    }

    // Leak and tank, the two hard vetoes, each with its own tri-state.
    let leak = match loaded.binding_for(&MeasurementKind::LeakState) {
        None => LeakState::Unknown,
        Some(bound) => read_leak(db, loaded, &bound.binding, now_ms).await?,
    };
    let tank = match loaded.binding_for(&MeasurementKind::TankLevel) {
        None => None,
        Some(bound) => Some(read_tank(db, &bound.binding, now_ms).await?),
    };

    // Every other `required` binding, so a plant that declared one is not
    // watered while it is silent (SAFETY-017).
    let mut required = Vec::new();
    for bound in loaded
        .sensors
        .iter()
        .filter(|b| b.binding.role == BindingRole::Required)
        .filter(|b| {
            !matches!(
                b.binding.kind,
                MeasurementKind::LeakState | MeasurementKind::TankLevel
            )
        })
    {
        required.push(RequiredInput {
            kind: bound.binding.kind.clone(),
            state: read_required(db, loaded, &bound.binding, now_ms).await?,
        });
    }

    let actuator_device = loaded.actuator.as_ref().map(|a| a.device_id.to_string());
    let (device_online, device_sleeping, reconciling) = match actuator_device.as_deref() {
        None => (false, false, false),
        Some(device) => device_reachability(db, device, now_ms).await?,
    };

    // Through `window_start`, not recomputed here. It is the one place the
    // window's *shape* is stated — rolling, never a calendar day (SAFETY-006) —
    // and a second expression saying the same thing is a second thing to change.
    let delivered = command_repo::delivered_in_window(
        db,
        plant_id,
        rhizo_domain::irrigation::budget::window_start(now).timestamp_millis(),
    )
    .await?;

    let lockout = command_repo::lockout(db, plant_id).await?;
    let automation =
        AutomationPolicy::from_profile(&loaded.profile, loaded.plant.auto_watering_enabled, None);

    Ok(Gathered {
        state,
        automation,
        soil,
        pre_dose_soil: stored.as_ref().and_then(|row| {
            row.pre_dose_vwc.map(|vwc| SoilSample {
                moisture_vwc: Some(vwc),
                // The baseline's own age never gates anything; the gate reads
                // the *latest* sample's freshness. Dating it at the cycle start
                // keeps the type honest without inventing a receipt time.
                received_at: DateTime::from_timestamp_millis(row.state_since).unwrap_or(now),
            })
        }),
        weight,
        pre_dose_weight: stored.as_ref().and_then(|row| {
            row.pre_dose_grams.map(|grams| WeightSample {
                grams: Some(grams),
                received_at: DateTime::from_timestamp_millis(row.state_since).unwrap_or(now),
            })
        }),
        tank,
        leak,
        bindings: loaded.bindings(),
        actuator: loaded.actuator.clone(),
        policies: loaded.policies.clone(),
        required,
        delivered_last_24h_ml: delivered as f32,
        doses_this_cycle: stored.as_ref().map_or(0, |row| {
            u8::try_from(row.doses_this_cycle).unwrap_or(u8::MAX)
        }),
        last_cycle_completed_at: stored
            .as_ref()
            .and_then(|row| row.last_cycle_completed_at)
            .and_then(DateTime::from_timestamp_millis),
        wait_until: stored
            .as_ref()
            .and_then(|row| row.wait_until)
            .and_then(DateTime::from_timestamp_millis),
        auto_watering_enabled: loaded.plant.auto_watering_enabled,
        device_online,
        device_sleeping,
        dry_duration,
        reconciling,
        active_lockout: lockout
            .as_ref()
            .and_then(|(reason, ..)| plant::lockout_from_str(reason)),
        lockout_held_until: lockout
            .as_ref()
            .and_then(|(_, _, until)| *until)
            .and_then(DateTime::from_timestamp_millis),
        active_command_id: stored.and_then(|row| row.active_command_id),
        actuator_device,
    })
}

/// The plant's control-freshness threshold for one kind, in milliseconds.
async fn freshness_ms(
    db: &EdgeDb,
    loaded: &Loaded,
    binding: &SensorBinding,
) -> Result<i64, rhizo_storage::StorageError> {
    let device = binding.device_id.to_string();
    let cadence = query::telemetry_interval_seconds(db, &device)
        .await?
        .unwrap_or(300);
    let stale_after = loaded
        .policy(&binding.kind)
        .map_or(u32::MAX, |p| p.stale_after_ms);
    Ok(control_freshness_ms(stale_after, cadence))
}

/// The latest scalar reading of a bound stream, or `None` when there is none.
///
/// The inner `Option<f64>` is `None` for a reading that exists but is not usable
/// — the distinction the gate turns into `SensorFault` rather than "no sample".
async fn read_scalar(
    db: &EdgeDb,
    binding: &SensorBinding,
    now_ms: i64,
) -> Result<Option<(Option<f64>, DateTime<Utc>)>, rhizo_storage::StorageError> {
    let Some(row) = query::latest_measurement(
        db,
        binding.device_id.as_ref(),
        binding.sensor_id.as_str(),
        binding.point.as_str(),
        binding.kind.as_str(),
    )
    .await?
    else {
        return Ok(None);
    };
    let received_at = DateTime::from_timestamp_millis(row.received_at)
        .unwrap_or_else(|| DateTime::from_timestamp_millis(now_ms).unwrap_or_default());
    let value = row
        .value_num
        .filter(|v| v.is_finite())
        .filter(|_| row.quality == "ok");
    Ok(Some((value, received_at)))
}

/// The leak tri-state. Absent, unreadable, non-`ok`, or stale all read
/// `Unknown` — and `Unknown` refuses.
async fn read_leak(
    db: &EdgeDb,
    loaded: &Loaded,
    binding: &SensorBinding,
    now_ms: i64,
) -> Result<LeakState, rhizo_storage::StorageError> {
    let max_age = freshness_ms(db, loaded, binding).await?;
    let Some(row) = query::latest_measurement(
        db,
        binding.device_id.as_ref(),
        binding.sensor_id.as_str(),
        binding.point.as_str(),
        binding.kind.as_str(),
    )
    .await?
    else {
        return Ok(LeakState::Unknown);
    };
    if row.quality != "ok" || now_ms.saturating_sub(row.received_at).max(0) >= max_age {
        return Ok(LeakState::Unknown);
    }
    // `leak_state` is a boolean kind, so a numeric-only row is a wrongly typed
    // reading rather than a leak reading (ADR-017).
    Ok(match row.value_bool {
        Some(0) => LeakState::Clear,
        Some(1) => LeakState::Detected,
        Some(_) | None => LeakState::Unknown,
    })
}

/// The reservoir tri-state, carrying its age so the gate can order low before
/// stale.
async fn read_tank(
    db: &EdgeDb,
    binding: &SensorBinding,
    now_ms: i64,
) -> Result<TankState, rhizo_storage::StorageError> {
    let Some(row) = query::latest_measurement(
        db,
        binding.device_id.as_ref(),
        binding.sensor_id.as_str(),
        binding.point.as_str(),
        binding.kind.as_str(),
    )
    .await?
    else {
        return Ok(TankState::Unknown);
    };
    if row.quality != "ok" {
        return Ok(TankState::Invalid);
    }
    match row.value_num.filter(|v| v.is_finite()) {
        None => Ok(TankState::Invalid),
        Some(percent) => Ok(TankState::Level {
            percent,
            age: Duration::milliseconds(now_ms.saturating_sub(row.received_at).max(0)),
        }),
    }
}

/// The condition of one required measurement.
async fn read_required(
    db: &EdgeDb,
    loaded: &Loaded,
    binding: &SensorBinding,
    now_ms: i64,
) -> Result<RequiredInputState, rhizo_storage::StorageError> {
    // A required binding with no policy has no freshness limit to be judged
    // against, which is uncertainty rather than health (SAFETY-012).
    if loaded.policy(&binding.kind).is_none() {
        return Ok(RequiredInputState::Missing);
    }
    let max_age = freshness_ms(db, loaded, binding).await?;
    let Some(row) = query::latest_measurement(
        db,
        binding.device_id.as_ref(),
        binding.sensor_id.as_str(),
        binding.point.as_str(),
        binding.kind.as_str(),
    )
    .await?
    else {
        return Ok(RequiredInputState::Missing);
    };
    let typed = row.value_num.is_some_and(f64::is_finite)
        || row.value_bool.is_some_and(|v| matches!(v, 0 | 1));
    if row.quality != "ok" || !typed {
        return Ok(RequiredInputState::Invalid);
    }
    if now_ms.saturating_sub(row.received_at).max(0) >= max_age {
        return Ok(RequiredInputState::Stale);
    }
    // The M4 registry's own health declaration still counts: a probe the device
    // says is faulty is not made healthy by one recent number.
    if query::sensor_healthy(db, binding.device_id.as_ref(), binding.sensor_id.as_str()).await?
        == Some(false)
    {
        return Ok(RequiredInputState::Invalid);
    }
    Ok(RequiredInputState::Usable)
}

/// `(online, sleeping, reconciling)` for one device, from the **derived**
/// connectivity rather than the stored column.
///
/// `connectivity::from_projection` re-checks `overdue_at` against the edge clock
/// on every read, so an overdue sleeper is `isolated` even if the liveness timer
/// is stopped or wedged (SAFETY-021). Reading the raw column here would make the
/// invariant depend on a writer.
pub async fn device_reachability(
    db: &EdgeDb,
    device_id: &str,
    now_ms: i64,
) -> Result<(bool, bool, bool), rhizo_storage::StorageError> {
    use sqlx::Row as _;
    let Some(row) = sqlx::query(
        "SELECT connectivity_mode,expected_wake_at,overdue_at FROM devices WHERE device_id=?",
    )
    .bind(device_id)
    .fetch_optional(db.pool())
    .await
    .map_err(|e| rhizo_storage::StorageError::Database(e.to_string()))?
    else {
        return Ok((false, false, false));
    };
    // Reconciliation outranks everything else. A device replaying its buffer is
    // *reachable* and still must not be issued a dose, because the budget does
    // not yet include what it did while it was alone (SAFETY-016). It is derived
    // from `replay_progress` rather than read from the column, which
    // `device.status` overwrites on every heartbeat.
    if crate::control::reconcile::is_reconciling(db, device_id)
        .await
        .map_err(|_| {
            rhizo_storage::StorageError::Database(
                "could not determine reconciliation state".to_owned(),
            )
        })?
    {
        return Ok((false, false, true));
    }
    let state = connectivity::from_projection(
        &row.get::<String, _>("connectivity_mode"),
        row.get("expected_wake_at"),
        row.get("overdue_at"),
        now_ms,
    );
    Ok(match state {
        connectivity::State::Online => (true, false, false),
        connectivity::State::SleepingExpected { .. } => (false, true, false),
        connectivity::State::OfflineUnexpectedly => (false, false, false),
        connectivity::State::Reconciling => (false, false, true),
    })
}
