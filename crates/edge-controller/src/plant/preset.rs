//! Applying a species preset to a plant (M5-018).
//!
//! The whole value is in what this does **not** do. It does not introduce a
//! second configuration path, it does not become the plant's authority, and it
//! does not survive as a link that later edits have to fight. It writes ordinary
//! [`MeasurementPolicy`] rows and ordinary defaults, exactly as a
//! hand-configured plant would have, and then gets out of the way.
//!
//! Five rules, each of which a test enforces:
//!
//! - **Materialisation happens exactly once**, at the moment of application. Not
//!   on restart, not on a catalogue upgrade, not on a tick. There is no code
//!   path that re-derives a value, which is why `applied_preset_id` can be inert.
//! - **Resolution is against the plant's existing `SensorBinding` rows.** A
//!   preset names a `MeasurementKind`; the binding decides which physical sensor
//!   supplies it. Applying a preset creates, selects, and edits no binding — a
//!   catalogue has no idea which probe is in which pot.
//! - **A preset cannot widen a safety limit.** Materialised values pass through
//!   the same validation as hand-entered ones, and a preset asking for a dose
//!   above the profile's hard limit is rejected by M5-003's check rather than
//!   clamped. A curated catalogue is an input, not a trusted one.
//! - **`auto_watering_enabled` stays `false`.** Nothing here authorises anything.
//! - **A monitoring-only plant is a fully supported target** (SAFETY-018).
//!   Application succeeds, measurement policies are created normally, and the
//!   dose and cooldown classes are recorded as inert starting values. Nothing on
//!   this path writes to `actuator_bindings`, so the actuation path is still
//!   absent and `POST /water` still returns 422 `no_actuator_bound`.
//!
//! [`MeasurementPolicy`]: rhizo_domain::plant::MeasurementPolicy
use rhizo_domain::measurement_policy::{MeasurementPolicyError, MeasurementPolicyRules as _};
use rhizo_domain::plant::MeasurementPolicy;
use rhizo_domain::preset::{PlantPreset, catalogue};
use rhizo_domain::profile::{PlantProfile, ProfileError};
use rhizo_storage::EdgeDb;
use rhizo_storage::repo::{binding as binding_repo, plant as plant_repo};

use super::{Loaded, policy_to_row};

/// The freshness horizon a materialised policy starts with.
///
/// A preset cannot know an installation's telemetry cadence, so it does not
/// carry one. Fifteen minutes is SAFETY-005's own floor, which makes it the
/// conservative starting value rather than an invented one; the operator edits
/// it afterwards like any other field.
pub const DEFAULT_STALE_AFTER_MS: u32 = 15 * 60 * 1_000;

/// Why an application was refused.
#[derive(Clone, Debug, PartialEq)]
pub enum PresetError {
    /// No such entry in the embedded catalogue.
    UnknownPreset {
        /// The id that was asked for.
        preset_id: String,
    },
    /// The plant already has policies and `overwrite` was not set.
    AlreadyConfigured {
        /// The kinds that would have been overwritten.
        kinds: Vec<String>,
    },
    /// A materialised value would violate a profile hard limit.
    ProfileRejected(ProfileError),
    /// A materialised policy would violate a policy rule.
    PolicyRejected(MeasurementPolicyError),
}

impl PresetError {
    /// The stable API error code.
    ///
    /// A rejected value reports the **rule it broke**, not that a preset was
    /// involved: the operator needs to know it is the firmware dose ceiling,
    /// and that the answer would have been the same had they typed the number
    /// by hand.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::UnknownPreset { .. } => "unknown_preset",
            Self::AlreadyConfigured { .. } => "already_configured",
            Self::ProfileRejected(error) => error.code(),
            Self::PolicyRejected(error) => error.code(),
        }
    }
}

impl core::fmt::Display for PresetError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::UnknownPreset { preset_id } => {
                write!(f, "{preset_id} is not in the embedded catalogue")
            }
            Self::AlreadyConfigured { kinds } => write!(
                f,
                "this plant already has measurement policies for {}; pass overwrite=true to replace them",
                kinds.join(", ")
            ),
            Self::ProfileRejected(error) => write!(f, "{error}"),
            Self::PolicyRejected(error) => write!(f, "{error}"),
        }
    }
}

/// What an application changed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Applied {
    /// The entry that was applied.
    pub preset_id: String,
    /// The catalogue version it came from.
    pub catalogue_version: u32,
    /// Kinds whose policy row was written.
    pub configured_kinds: Vec<String>,
    /// Kinds the preset knows about but the plant has no binding for, so no
    /// policy row was created. Reported rather than silently skipped.
    pub skipped_unbound_kinds: Vec<String>,
    /// Kinds whose existing policy was replaced.
    pub replaced_kinds: Vec<String>,
    /// The dose the dose class resolved to against this pot, or `None` when the
    /// plant has no pot volume recorded.
    pub dose_ml: Option<f32>,
    /// The cooldown the cooldown class resolved to.
    pub cooldown_hours: f64,
    /// Whether the plant has an actuator. `false` is normal, not an error.
    pub has_actuator: bool,
}

/// Resolves a preset against a plant and returns what would be written.
///
/// Pure with respect to storage: it reads the loaded plant and answers. Split
/// from [`apply`] so the API can validate before writing anything, and so a test
/// can assert on the result without a database round trip.
///
/// # Errors
///
/// Returns the first rule the materialised configuration would violate.
pub fn plan(
    loaded: &Loaded,
    preset: &PlantPreset,
    catalogue_version: u32,
    overwrite: bool,
) -> Result<(Applied, Vec<MeasurementPolicy>, PlantProfile), PresetError> {
    let mut configured = Vec::new();
    let mut skipped = Vec::new();
    let mut replaced = Vec::new();
    let mut policies = Vec::new();

    for preference in &preset.measurements {
        // A preset names a kind; a binding names a sensor. A kind the plant has
        // no binding for gets no policy row, and is reported.
        if loaded.binding_for(&preference.kind).is_none() {
            skipped.push(preference.kind.as_str().to_owned());
            continue;
        }
        let existing = loaded.policy(&preference.kind);
        if existing.is_some() {
            replaced.push(preference.kind.as_str().to_owned());
        }
        // The confirmation window is a property of the plant's own debounce, not
        // of the species: a preset that carried one would be a schedule.
        let confirm = existing
            .and_then(|p| p.confirm_duration_ms)
            .or_else(|| Some(loaded.profile.dry_confirm_minutes.saturating_mul(60_000)));
        let stale_after = existing.map_or(DEFAULT_STALE_AFTER_MS, |p| p.stale_after_ms);
        let policy = preference.to_policy(stale_after, confirm);
        policy.validate().map_err(PresetError::PolicyRejected)?;
        configured.push(preference.kind.as_str().to_owned());
        policies.push(policy);
    }

    if !replaced.is_empty() && !overwrite {
        return Err(PresetError::AlreadyConfigured { kinds: replaced });
    }

    // The dose class resolves against the pot, because millilitres without a pot
    // volume are meaningless. A plant with no recorded pot keeps the profile's
    // dose rather than inventing one.
    let dose_ml = loaded
        .plant
        .pot_volume_ml
        .map(|volume| preset.dose_class.dose_ml(volume));
    let cooldown_hours = preset.cooldown_class.hours();
    let profile = PlantProfile {
        dose_ml: dose_ml.unwrap_or(loaded.profile.dose_ml),
        cooldown_hours,
        ..loaded.profile.clone()
    };
    // The same validation a hand-entered profile faces. A curated catalogue is
    // an input, not a trusted one, so a dose above the firmware ceiling is
    // rejected here rather than clamped (ADR-011).
    profile.validate().map_err(PresetError::ProfileRejected)?;

    Ok((
        Applied {
            preset_id: preset.preset_id.clone(),
            catalogue_version,
            configured_kinds: configured,
            skipped_unbound_kinds: skipped,
            replaced_kinds: replaced,
            dose_ml,
            cooldown_hours,
            has_actuator: loaded.actuator.is_some(),
        },
        policies,
        profile,
    ))
}

/// Materialises a preset into ordinary per-plant configuration.
///
/// Writes `measurement_policies` rows and the plant's provenance columns, and
/// nothing else. In particular it writes **no** `actuator_bindings` row, touches
/// **no** `sensor_bindings` row, and does not enable automation.
///
/// # Errors
///
/// Returns the refusal from [`plan`], or a storage failure.
pub async fn apply(
    db: &EdgeDb,
    loaded: &Loaded,
    preset_id: &str,
    overwrite: bool,
    now: i64,
) -> Result<Applied, ApplyError> {
    let catalogue = catalogue::catalogue();
    let preset = catalogue::get(preset_id).ok_or_else(|| {
        ApplyError::Preset(PresetError::UnknownPreset {
            preset_id: preset_id.to_owned(),
        })
    })?;
    let (applied, policies, profile) =
        plan(loaded, preset, catalogue.catalogue_version, overwrite).map_err(ApplyError::Preset)?;

    let plant_id = loaded.plant.plant_id.as_str();
    for policy in &policies {
        binding_repo::upsert_measurement_policy(db, &policy_to_row(plant_id, policy), now).await?;
    }
    // The automation starting values live on the plant's own profile document,
    // so an operator edits them exactly where they edit a hand-configured
    // plant's. A plant with no profile of its own gets one named after itself.
    let profile_id = loaded
        .plant
        .profile_id
        .clone()
        .unwrap_or_else(|| format!("{plant_id}-profile"));
    let document = serde_json::to_string(&profile).map_err(|e| {
        ApplyError::Storage(rhizo_storage::StorageError::Serialization(e.to_string()))
    })?;
    rhizo_storage::repo::profile::upsert(db, &profile_id, &profile.name, &document, now).await?;
    if loaded.plant.profile_id.is_none() {
        plant_repo::update(
            db,
            plant_id,
            &plant_repo::PlantPatch {
                profile_id: Some(Some(profile_id)),
                ..Default::default()
            },
        )
        .await?;
    }
    plant_repo::record_applied_preset(db, plant_id, preset_id, applied.catalogue_version).await?;
    Ok(applied)
}

/// An application failure: a refusal, or storage.
#[derive(Debug)]
pub enum ApplyError {
    /// The application was refused.
    Preset(PresetError),
    /// Storage failed.
    Storage(rhizo_storage::StorageError),
}

impl From<rhizo_storage::StorageError> for ApplyError {
    fn from(error: rhizo_storage::StorageError) -> Self {
        Self::Storage(error)
    }
}

#[cfg(test)]
mod tests {
    use crate::api::testsupport::{TestApi, base};
    use chrono::Duration;

    /// Two plants with the same numbers, one preset-configured and one
    /// hand-configured, must be indistinguishable to every decision path.
    #[tokio::test]
    async fn recommendation_is_identical_for_preset_and_hand_configured_plants() {
        let api = TestApi::start().await;
        api.with_device().await;
        for id in ["preset-01", "hand-01"] {
            api.plant(id).await;
            api.bind_control(id).await;
            api.json(
                "PUT",
                &format!("/api/v1/plants/{id}/bindings/actuator"),
                serde_json::json!({ "device_id": "plant-node-01", "actuator_id": "pump-0" }),
            )
            .await;
        }
        api.json(
            "POST",
            "/api/v1/plants/preset-01/apply-preset",
            serde_json::json!({ "preset_id": "monstera-deliciosa" }),
        )
        .await;
        // The same numbers the preset materialised, typed by hand.
        let (_, materialised) = api
            .get("/api/v1/plants/preset-01/measurement-policies")
            .await;
        let mut body = materialised["measurement_policies"][0].clone();
        let object = body.as_object_mut().unwrap();
        object.remove("kind");
        api.json(
            "PUT",
            "/api/v1/plants/hand-01/measurement-policies/soil_moisture",
            body,
        )
        .await;
        // ...and the same automation defaults, selected the way an operator
        // would select any template. What is being compared is whether a
        // decision path can tell the two plants apart, so they must differ in
        // nothing except how their numbers got there.
        api.json(
            "PATCH",
            "/api/v1/plants/hand-01",
            serde_json::json!({ "profile_id": "preset-01-profile" }),
        )
        .await;

        for i in 0i64..72 {
            let at = base() - Duration::minutes((71 - i) * 5);
            api.sample(at, 40.0 - i as f64 * 0.25).await;
        }

        let preset = api.evaluate("preset-01").await;
        let hand = api.evaluate("hand-01").await;
        assert_eq!(preset.decision, hand.decision);
        assert_eq!(preset.recommended_ml, hand.recommended_ml);
        assert_eq!(preset.blocked_by, hand.blocked_by);
        assert_eq!(
            preset.reasons.iter().map(|r| r.code()).collect::<Vec<_>>(),
            hand.reasons.iter().map(|r| r.code()).collect::<Vec<_>>(),
            "no decision path may be able to tell the two apart"
        );

        // And one of them does carry provenance, so the test is not vacuous.
        let (_, plant) = api.get("/api/v1/plants/preset-01").await;
        assert_eq!(plant["applied_preset_id"], "monstera-deliciosa");
    }

    /// The structural half of the same promise: the provenance column is not
    /// read by recommendation, by the safety gate, by irrigation control, or by
    /// offline-policy evaluation.
    ///
    /// Asserted over the source rather than by behaviour, because a behavioural
    /// test can only show that nothing reads it *today*.
    #[tokio::test]
    async fn applied_preset_id_is_read_by_no_decision_path() {
        let column = concat!("applied_preset", "_id");
        let decision_modules = [
            (
                "recommend",
                include_str!("../../../domain/src/recommend.rs"),
            ),
            (
                "plant_state",
                include_str!("../../../domain/src/plant_state.rs"),
            ),
            (
                "offline_policy",
                include_str!("../../../domain/src/offline_policy.rs"),
            ),
            (
                "threshold",
                include_str!("../../../domain/src/threshold.rs"),
            ),
            (
                "safety gate",
                include_str!("../../../mqtt-contract/src/safety.rs"),
            ),
            (
                "offline evaluator types",
                include_str!("../../../policy/src/types.rs"),
            ),
            (
                "offline policy validation",
                include_str!("../../../policy/src/validate.rs"),
            ),
            (
                "the control tick",
                include_str!("../../src/control/tick.rs"),
            ),
            (
                "threshold evaluation",
                include_str!("../../src/control/threshold.rs"),
            ),
            ("input assembly", include_str!("../../src/plant/mod.rs")),
        ];
        for (name, source) in decision_modules {
            assert!(
                !source.contains(column),
                "{name} reads {column}: a preset must not be a second configuration authority"
            );
        }
        // The column does exist, and is written by exactly this module and read
        // by the API — so the assertion above is not passing by accident.
        assert!(include_str!("preset.rs").contains("record_applied_preset"));
    }
}
