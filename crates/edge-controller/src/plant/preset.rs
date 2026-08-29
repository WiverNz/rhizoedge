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
use rhizo_storage::repo::binding as binding_repo;

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
    // The storage transaction reuses an exclusively owned profile, but clones
    // a shared one so applying to this plant cannot rewrite another plant's
    // configuration. The fallback id is unique even if an unrelated profile
    // already uses a plant-derived name.
    let private_profile_id = format!("{plant_id}-profile-{}", uuid::Uuid::new_v4());
    let document = serde_json::to_string(&profile).map_err(|e| {
        ApplyError::Storage(rhizo_storage::StorageError::Serialization(e.to_string()))
    })?;
    let policy_rows = policies
        .iter()
        .map(|policy| policy_to_row(plant_id, policy))
        .collect::<Vec<_>>();
    binding_repo::materialize_preset(
        db,
        plant_id,
        &policy_rows,
        loaded.plant.profile_id.as_deref(),
        &private_profile_id,
        &profile.name,
        &document,
        preset_id,
        applied.catalogue_version,
        now,
    )
    .await?;
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
        let (_, preset_plant) = api.get("/api/v1/plants/preset-01").await;
        let preset_profile = preset_plant["profile_id"].as_str().unwrap();
        api.json(
            "PATCH",
            "/api/v1/plants/hand-01",
            serde_json::json!({ "profile_id": preset_profile }),
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
        assert!(include_str!("preset.rs").contains("materialize_preset"));
        assert!(
            include_str!("../../../storage/src/repo/binding.rs")
                .contains("applied_preset_id=?,applied_catalogue_version=?")
        );
    }

    /// A failure at the final provenance write must roll back the earlier
    /// policy and profile writes. The trigger is a negative control for the
    /// transaction boundary, not production behavior.
    #[tokio::test]
    async fn a_partial_preset_failure_rolls_back_every_materialized_value() {
        let api = TestApi::start().await;
        api.with_device().await;
        api.plant("monstera-01").await;
        api.bind_control("monstera-01").await;
        let loaded = super::super::load(&api.db, "monstera-01")
            .await
            .unwrap()
            .unwrap();
        sqlx::query(
            "CREATE TRIGGER reject_preset_provenance BEFORE UPDATE OF applied_preset_id ON plants \
             BEGIN SELECT RAISE(ABORT, 'injected late preset failure'); END",
        )
        .execute(api.db.pool())
        .await
        .unwrap();

        let result = super::apply(
            &api.db,
            &loaded,
            "monstera-deliciosa",
            false,
            base().timestamp_millis(),
        )
        .await;
        assert!(matches!(result, Err(super::ApplyError::Storage(_))));
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM measurement_policies WHERE plant_id='monstera-01'"
            )
            .fetch_one(api.db.pool())
            .await
            .unwrap(),
            0
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM plant_profiles")
                .fetch_one(api.db.pool())
                .await
                .unwrap(),
            0
        );
        let plant = rhizo_storage::repo::plant::get(&api.db, "monstera-01")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(plant.profile_id, None);
        assert_eq!(plant.applied_preset_id, None);
    }

    /// Race-shaped stale-read control: the API loaded a live plant, another
    /// request soft-deleted it, and only then did materialisation reach its
    /// transaction. The checked final update must fail and roll everything
    /// before it back.
    #[tokio::test]
    async fn a_soft_deleted_target_rolls_back_preset_materialization() {
        let api = TestApi::start().await;
        api.with_device().await;
        api.plant("monstera-01").await;
        api.bind_control("monstera-01").await;
        let loaded = super::super::load(&api.db, "monstera-01")
            .await
            .unwrap()
            .unwrap();
        assert!(
            rhizo_storage::repo::plant::delete(&api.db, "monstera-01", base().timestamp_millis())
                .await
                .unwrap()
        );

        let result = super::apply(
            &api.db,
            &loaded,
            "monstera-deliciosa",
            false,
            base().timestamp_millis(),
        )
        .await;
        assert!(matches!(
            result,
            Err(super::ApplyError::Storage(
                rhizo_storage::StorageError::Constraint(_)
            ))
        ));
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM measurement_policies")
                .fetch_one(api.db.pool())
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM plant_profiles")
                .fetch_one(api.db.pool())
                .await
                .unwrap(),
            0
        );
        let row = sqlx::query(
            "SELECT profile_id,applied_preset_id,applied_catalogue_version FROM plants WHERE plant_id='monstera-01'",
        )
        .fetch_one(api.db.pool())
        .await
        .unwrap();
        use sqlx::Row as _;
        assert_eq!(row.get::<Option<String>, _>("profile_id"), None);
        assert_eq!(row.get::<Option<String>, _>("applied_preset_id"), None);
        assert_eq!(row.get::<Option<i64>, _>("applied_catalogue_version"), None);
    }

    /// Profiles may seed several plants, but materialised automation values are
    /// per plant. Applying to one owner must clone before changing the profile.
    #[tokio::test]
    async fn applying_to_one_plant_does_not_mutate_a_shared_profile() {
        let api = TestApi::start().await;
        api.with_device().await;
        let shared = super::super::default_profile();
        let original = serde_json::to_string(&shared).unwrap();
        rhizo_storage::repo::profile::upsert(
            &api.db,
            "shared-profile",
            &shared.name,
            &original,
            base().timestamp_millis(),
        )
        .await
        .unwrap();
        for id in ["plant-a", "plant-b"] {
            rhizo_storage::repo::plant::create(
                &api.db,
                &rhizo_storage::repo::plant::NewPlant {
                    plant_id: id.to_owned(),
                    name: id.to_owned(),
                    profile_id: Some("shared-profile".to_owned()),
                    pot_volume_ml: Some(2_500.0),
                    ..Default::default()
                },
                base().timestamp_millis(),
            )
            .await
            .unwrap();
        }
        api.bind_control("plant-a").await;
        let loaded = super::super::load(&api.db, "plant-a")
            .await
            .unwrap()
            .unwrap();
        super::apply(
            &api.db,
            &loaded,
            "monstera-deliciosa",
            false,
            base().timestamp_millis(),
        )
        .await
        .unwrap();

        let a = rhizo_storage::repo::plant::get(&api.db, "plant-a")
            .await
            .unwrap()
            .unwrap();
        let b = rhizo_storage::repo::plant::get(&api.db, "plant-b")
            .await
            .unwrap()
            .unwrap();
        assert_ne!(a.profile_id, b.profile_id, "plant-a must own a clone");
        assert_eq!(b.profile_id.as_deref(), Some("shared-profile"));
        assert_eq!(
            rhizo_storage::repo::profile::get(&api.db, "shared-profile")
                .await
                .unwrap()
                .unwrap()
                .profile_json,
            original,
            "plant-b's effective configuration must remain byte-identical"
        );
        let applied_profile =
            rhizo_storage::repo::profile::get(&api.db, a.profile_id.as_deref().unwrap())
                .await
                .unwrap()
                .unwrap();
        assert_ne!(applied_profile.profile_json, original);
    }
}
