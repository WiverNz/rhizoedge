//! Authoring the policy a device may act on alone (M5-016, ADR-015).
//!
//! The edge derives a candidate policy from what the plant already has — its
//! bindings and its per-measurement policies — and then validates it with
//! [`rhizo_policy::validate_authored`], which is the **same rule set the device
//! will apply**. Validating with a second set of rules is how an edge publishes
//! a policy the device rejects, leaving a plant with autonomy that silently
//! never activates.
//!
//! Two defaults are load-bearing:
//!
//! - **A plant with no `ActuatorBinding` cannot have an offline policy at all.**
//!   It is refused at authoring time with a specific message, rather than
//!   letting an operator configure autonomy that can never run (SAFETY-018).
//! - **`enabled` defaults to `false`.** Creating a policy is not the same act as
//!   authorising a device to water unsupervised, and the two should require
//!   separate decisions.
use rhizo_mqtt_contract::payload::{
    ActuatorKind, AdvisoryMeasurement, ControlMeasurement, MeasurementKind, OfflineActuator,
    OfflineLimits, OfflinePolicy, OfflineSafety, PolicyError, RequiredMeasurement, SensorId,
};

use crate::binding::is_safety_kind;
use crate::plant::{ActuatorBinding, BindingRole, MeasurementPolicy, SensorBinding};
use crate::profile::PlantProfile;

/// The rolling window an offline budget is measured over.
pub const OFFLINE_WINDOW_MS: u32 = 24 * 60 * 60 * 1000;
/// The tank floor an offline dose requires, in percent.
pub const OFFLINE_TANK_FLOOR_PERCENT: f32 = 15.0;

/// Why a policy could not be authored.
#[derive(Clone, Debug, PartialEq)]
pub enum OfflinePolicyError {
    /// The plant has no actuator, so autonomy could never run (SAFETY-018).
    NoActuatorBinding,
    /// The plant has no `control`-role binding, so nothing would trigger a dose.
    NoControlBinding,
    /// The control kind has no measurement policy, or the policy states no band.
    ControlPolicyIncomplete {
        /// The control kind.
        kind: String,
    },
    /// The plant id cannot be carried on the wire.
    PlantIdNotRepresentable {
        /// The identifier as supplied.
        plant_id: String,
    },
    /// The shared validator refused it. The variant is the contract's own.
    Rejected(PolicyError),
}

impl OfflinePolicyError {
    /// The stable API error code.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::NoActuatorBinding => "no_actuator_bound",
            Self::NoControlBinding => "no_control_binding",
            Self::ControlPolicyIncomplete { .. } => "control_policy_incomplete",
            Self::PlantIdNotRepresentable { .. } => "plant_id_not_representable",
            Self::Rejected(_) => "policy_rejected",
        }
    }
}

impl core::fmt::Display for OfflinePolicyError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NoActuatorBinding => write!(
                f,
                "this plant has no actuator, so it cannot have an offline policy"
            ),
            Self::NoControlBinding => write!(
                f,
                "an offline policy needs a control binding to decide with"
            ),
            Self::ControlPolicyIncomplete { kind } => write!(
                f,
                "the measurement policy for {kind} must state both target_min and target_max"
            ),
            Self::PlantIdNotRepresentable { plant_id } => {
                write!(f, "{plant_id} cannot be carried as a wire identifier")
            }
            Self::Rejected(error) => write!(f, "the shared policy validator refused it: {error:?}"),
        }
    }
}

/// Everything authoring reads. All of it is already the plant's own
/// configuration — nothing new is invented here.
#[derive(Clone, Debug)]
pub struct AuthoringInputs<'a> {
    /// The wire-safe plant identifier.
    pub plant_id: &'a str,
    /// The plant's sensor bindings.
    pub bindings: &'a [SensorBinding],
    /// The plant's optional actuator binding.
    pub actuator: Option<&'a ActuatorBinding>,
    /// The plant's per-measurement policies.
    pub measurement_policies: &'a [MeasurementPolicy],
    /// The profile template, for dose, cooldown, and absorption.
    pub profile: &'a PlantProfile,
    /// The version to allocate. Callers pass `current + 1`; monotonicity is the
    /// caller's to enforce because only storage knows what is current.
    pub policy_version: u32,
}

/// Derives and validates the candidate policy.
///
/// The result is always `enabled: false`. Enabling is a separate operation.
///
/// # Errors
///
/// Returns the first authoring rule violated, or the shared validator's own
/// refusal.
pub fn author(inputs: &AuthoringInputs<'_>) -> Result<OfflinePolicy, OfflinePolicyError> {
    let actuator = inputs
        .actuator
        .ok_or(OfflinePolicyError::NoActuatorBinding)?;
    let control = inputs
        .bindings
        .iter()
        .find(|b| b.role == BindingRole::Control)
        .ok_or(OfflinePolicyError::NoControlBinding)?;
    let policy_for =
        |kind: &MeasurementKind| inputs.measurement_policies.iter().find(|p| &p.kind == kind);
    let control_policy =
        policy_for(&control.kind).ok_or_else(|| OfflinePolicyError::ControlPolicyIncomplete {
            kind: control.kind.as_str().to_owned(),
        })?;
    let (Some(trigger_below), Some(resume_above)) =
        (control_policy.target_min, control_policy.target_max)
    else {
        return Err(OfflinePolicyError::ControlPolicyIncomplete {
            kind: control.kind.as_str().to_owned(),
        });
    };
    let plant_id = SensorId::parse(inputs.plant_id).map_err(|_| {
        OfflinePolicyError::PlantIdNotRepresentable {
            plant_id: inputs.plant_id.to_owned(),
        }
    })?;

    // Required measurements come from `required`-role bindings, which is the
    // whole point of the role: the operator declared what must be healthy, and
    // the policy carries exactly that.
    let required_measurements: Vec<RequiredMeasurement> = inputs
        .bindings
        .iter()
        .filter(|b| b.role == BindingRole::Required)
        .map(|b| RequiredMeasurement {
            kind: b.kind.clone(),
            point: b.point.clone(),
            max_age_ms: policy_for(&b.kind)
                .map_or(control_policy.stale_after_ms, |p| p.stale_after_ms),
        })
        .collect();
    let advisory_measurements: Vec<AdvisoryMeasurement> = inputs
        .bindings
        .iter()
        .filter(|b| b.role == BindingRole::Advisory)
        .map(|b| AdvisoryMeasurement {
            kind: b.kind.clone(),
            point: b.point.clone(),
        })
        .collect();

    let policy = OfflinePolicy {
        plant_id,
        policy_version: inputs.policy_version,
        // Authoring a policy is not authorising unsupervised watering.
        enabled: false,
        actuator: Some(OfflineActuator {
            actuator_id: actuator.actuator_id.clone(),
            kind: ActuatorKind::IrrigationPump,
            dose_ml: inputs.profile.dose_ml,
            max_doses_per_cycle: inputs.profile.max_doses_per_cycle,
            absorption_wait_ms: inputs.profile.absorption_minutes.saturating_mul(60_000),
        }),
        control_measurement: ControlMeasurement {
            kind: control.kind.clone(),
            point: control.point.clone(),
            trigger_below,
            resume_above,
            confirm_duration_ms: control_policy
                .confirm_duration_ms
                .unwrap_or_else(|| inputs.profile.dry_confirm_minutes.saturating_mul(60_000)),
            max_age_ms: control_policy.stale_after_ms,
        },
        required_measurements,
        advisory_measurements,
        limits: OfflineLimits {
            cooldown_ms: hours_to_ms(inputs.profile.cooldown_hours),
            max_volume_per_window_ml: inputs.profile.max_daily_ml,
            window_ms: OFFLINE_WINDOW_MS,
        },
        safety: OfflineSafety {
            // Both vetoes are demanded unconditionally. A policy that could opt
            // out of the leak check would be a policy that waters into a flood.
            require_leak_clear: true,
            require_tank_above_percent: OFFLINE_TANK_FLOOR_PERCENT,
            require_pump_healthy: true,
        },
    };
    // The shared validator, not a second copy of the rules.
    rhizo_policy::validate_authored(&policy).map_err(OfflinePolicyError::Rejected)?;
    Ok(policy)
}

/// Whether the plant's bindings cover every safety kind the policy demands.
///
/// Advisory: the gate refuses at runtime anyway, but telling an operator at
/// authoring time that their policy can never fire is worth more than letting
/// them discover it during a heatwave.
#[must_use]
pub fn missing_safety_bindings(bindings: &[SensorBinding]) -> Vec<MeasurementKind> {
    [MeasurementKind::LeakState, MeasurementKind::TankLevel]
        .into_iter()
        .filter(|kind| {
            is_safety_kind(kind)
                && !bindings
                    .iter()
                    .any(|b| &b.kind == kind && b.role == BindingRole::Required)
        })
        .collect()
}

fn hours_to_ms(hours: f64) -> u32 {
    let ms = hours * 3_600_000.0;
    if !ms.is_finite() || ms <= 0.0 {
        return 1;
    }
    // Saturating rather than wrapping: an absurd cooldown becomes the longest
    // representable one, never a short one.
    if ms >= f64::from(u32::MAX) {
        u32::MAX
    } else {
        ms as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ProfileId;
    use crate::measurement_policy::MeasurementPolicyRules as _;
    use rhizo_mqtt_contract::DeviceId;
    use rhizo_mqtt_contract::payload::MeasurementPoint;
    use rhizo_mqtt_contract::safety::FIRMWARE_MAX_ML_PER_RUN;

    fn profile() -> PlantProfile {
        PlantProfile::default_seed(ProfileId::from_uuid(uuid::Uuid::nil()))
    }

    fn binding(kind: MeasurementKind, role: BindingRole) -> SensorBinding {
        SensorBinding {
            device_id: DeviceId::parse("plant-node-01").unwrap(),
            sensor_id: SensorId::parse("soil-0").unwrap(),
            point: MeasurementPoint::parse("default").unwrap(),
            kind,
            role,
        }
    }

    fn bindings() -> Vec<SensorBinding> {
        vec![
            binding(MeasurementKind::SoilMoisture, BindingRole::Control),
            binding(MeasurementKind::LeakState, BindingRole::Required),
            binding(MeasurementKind::TankLevel, BindingRole::Required),
            binding(MeasurementKind::SoilEc, BindingRole::Advisory),
        ]
    }

    fn policies(profile: &PlantProfile) -> Vec<MeasurementPolicy> {
        vec![
            MeasurementPolicy::seeded_from_profile(MeasurementKind::SoilMoisture, profile, 900_000),
            MeasurementPolicy::seeded_from_profile(MeasurementKind::LeakState, profile, 600_000),
            MeasurementPolicy::seeded_from_profile(MeasurementKind::TankLevel, profile, 600_000),
        ]
    }

    fn actuator() -> ActuatorBinding {
        ActuatorBinding {
            device_id: DeviceId::parse("plant-node-01").unwrap(),
            actuator_id: SensorId::parse("pump-0").unwrap(),
            kind: ActuatorKind::IrrigationPump,
        }
    }

    fn inputs<'a>(
        profile: &'a PlantProfile,
        bindings: &'a [SensorBinding],
        policies: &'a [MeasurementPolicy],
        actuator: Option<&'a ActuatorBinding>,
        version: u32,
    ) -> AuthoringInputs<'a> {
        AuthoringInputs {
            plant_id: "monstera-01",
            bindings,
            actuator,
            measurement_policies: policies,
            profile,
            policy_version: version,
        }
    }

    #[test]
    fn a_valid_policy_is_authored_versioned_and_disabled() {
        let profile = profile();
        let b = bindings();
        let p = policies(&profile);
        let a = actuator();
        let policy = author(&inputs(&profile, &b, &p, Some(&a), 1)).unwrap();
        assert_eq!(policy.policy_version, 1);
        assert!(!policy.enabled, "authoring is not authorising");
        assert_eq!(policy.control_measurement.trigger_below, 28.0);
        assert_eq!(policy.control_measurement.resume_above, 45.0);
        assert_eq!(policy.control_measurement.max_age_ms, 900_000);
        assert_eq!(policy.limits.cooldown_ms, 6 * 3_600_000);
        assert_eq!(policy.limits.window_ms, OFFLINE_WINDOW_MS);
        assert!(policy.safety.require_leak_clear);
        assert!(policy.safety.require_pump_healthy);
    }

    /// Required measurements are derived from `required`-role bindings, and
    /// advisory ones stay advisory.
    #[test]
    fn required_measurements_come_from_required_role_bindings() {
        let profile = profile();
        let b = bindings();
        let p = policies(&profile);
        let a = actuator();
        let policy = author(&inputs(&profile, &b, &p, Some(&a), 1)).unwrap();
        let required: Vec<&str> = policy
            .required_measurements
            .iter()
            .map(|r| r.kind.as_str())
            .collect();
        assert_eq!(required, vec!["leak_state", "tank_level"]);
        assert_eq!(policy.required_measurements[0].max_age_ms, 600_000);
        let advisory: Vec<&str> = policy
            .advisory_measurements
            .iter()
            .map(|r| r.kind.as_str())
            .collect();
        assert_eq!(advisory, vec!["soil_ec"]);
        assert!(missing_safety_bindings(&b).is_empty());
        assert_eq!(
            missing_safety_bindings(&b[..1]),
            vec![MeasurementKind::LeakState, MeasurementKind::TankLevel]
        );
    }

    /// SAFETY-018 at authoring time, with a message that names the reason.
    #[test]
    fn safety_018_a_plant_with_no_actuator_cannot_have_an_offline_policy() {
        let profile = profile();
        let b = bindings();
        let p = policies(&profile);
        let error = author(&inputs(&profile, &b, &p, None, 1)).unwrap_err();
        assert_eq!(error, OfflinePolicyError::NoActuatorBinding);
        assert_eq!(error.code(), "no_actuator_bound");
        assert!(error.to_string().contains("no actuator"));
    }

    #[test]
    fn a_plant_with_no_control_binding_is_refused() {
        let profile = profile();
        let b: Vec<SensorBinding> = bindings()
            .into_iter()
            .filter(|x| x.role != BindingRole::Control)
            .collect();
        let p = policies(&profile);
        let a = actuator();
        assert_eq!(
            author(&inputs(&profile, &b, &p, Some(&a), 1)),
            Err(OfflinePolicyError::NoControlBinding)
        );
    }

    #[test]
    fn a_control_kind_with_no_band_is_refused() {
        let profile = profile();
        let b = bindings();
        let mut p = policies(&profile);
        p[0].target_max = None;
        let a = actuator();
        assert_eq!(
            author(&inputs(&profile, &b, &p, Some(&a), 1)),
            Err(OfflinePolicyError::ControlPolicyIncomplete {
                kind: "soil_moisture".into()
            })
        );
    }

    /// Rejected, never clamped — and by the contract's own error value, which is
    /// the visible proof that no second rule set exists here.
    #[test]
    fn a_dose_above_the_firmware_limit_is_rejected_by_the_shared_validator() {
        let mut profile = profile();
        profile.dose_ml = FIRMWARE_MAX_ML_PER_RUN + 120.0;
        profile.max_daily_ml = 500.0;
        let b = bindings();
        let p = policies(&profile);
        let a = actuator();
        assert_eq!(
            author(&inputs(&profile, &b, &p, Some(&a), 1)),
            Err(OfflinePolicyError::Rejected(
                PolicyError::DoseAboveHardLimit
            ))
        );
        assert_eq!(
            profile.dose_ml,
            FIRMWARE_MAX_ML_PER_RUN + 120.0,
            "authoring never clamps"
        );
    }

    /// The shared-validator assertion. Authoring holds no numeric rule of its
    /// own: everything the device would check is checked here by calling the
    /// device's function.
    #[test]
    fn validation_uses_the_shared_validator_and_not_a_second_rule_set() {
        let source = include_str!("offline_policy.rs");
        assert!(
            source.contains("rhizo_policy::validate_authored"),
            "authoring must call the shared validator"
        );
        // Assembled from fragments so this test's own source does not contain
        // the strings it forbids.
        for forbidden in [
            concat!("FIRMWARE_MAX_ML_PER_RUN", " >"),
            concat!("> ", "FIRMWARE_MAX"),
            concat!("resume_above <", "= "),
        ] {
            assert!(
                !source.contains(forbidden),
                "offline_policy.rs re-implements {forbidden}"
            );
        }
        // And the policy it produces satisfies the contract's own validator.
        let profile = profile();
        let b = bindings();
        let p = policies(&profile);
        let a = actuator();
        let mut policy = author(&inputs(&profile, &b, &p, Some(&a), 3)).unwrap();
        policy.enabled = true;
        assert_eq!(policy.validate(), Ok(()));
    }

    #[test]
    fn an_unrepresentable_plant_id_is_refused_rather_than_mangled() {
        let profile = profile();
        let b = bindings();
        let p = policies(&profile);
        let a = actuator();
        let mut i = inputs(&profile, &b, &p, Some(&a), 1);
        i.plant_id = "Monstera #1";
        assert_eq!(
            author(&i),
            Err(OfflinePolicyError::PlantIdNotRepresentable {
                plant_id: "Monstera #1".into()
            })
        );
    }
}
