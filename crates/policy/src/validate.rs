//! The one place an offline policy is judged fit to publish.
//!
//! [ADR-015](../../../docs/adr/015-device-offline-autonomy.md): a policy the
//! edge cannot evaluate is a policy it must not publish. If the edge validated
//! with its own rules it could publish a policy the device then rejects, leaving
//! a plant with autonomy that silently never activates — the worst of both
//! states, because nothing reports it.
//!
//! So there is no second rule set here. [`validate_authored`] delegates every
//! numeric and structural rule to [`OfflinePolicy::validate`] — the same
//! function the firmware calls — and adds exactly one thing the wire form cannot
//! express: **a policy is checked at authoring time even while it is disabled.**
//!
//! That difference is deliberate. §5.11 makes disabling a plant an
//! `enabled: false` republish of its existing policy, so the contract's
//! validation short-circuits on a disabled policy: there are no rules to break
//! when nothing will act on it. An operator authoring a policy is in the
//! opposite position — `enabled` defaults to `false`, and they need to know
//! *now* whether the numbers they just typed would be accepted, not at the
//! moment they authorise unsupervised watering.
use rhizo_mqtt_contract::payload::{OfflinePolicy, PolicyError};

/// Validates a policy the edge is about to persist or publish.
///
/// # Errors
///
/// Returns the contract's own [`PolicyError`], so an edge-side rejection and a
/// device-side rejection are the same value with the same name.
pub fn validate_authored(policy: &OfflinePolicy) -> Result<(), PolicyError> {
    // A plant with no actuator cannot have an offline policy at all
    // (SAFETY-018). The contract reaches the same conclusion, but only for an
    // enabled policy, and this rule must hold for a disabled one too.
    if policy.actuator.is_none() {
        return Err(PolicyError::MissingActuator);
    }
    let mut as_enabled = policy.clone();
    as_enabled.enabled = true;
    as_enabled.validate()
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec::Vec;
    use rhizo_mqtt_contract::payload::{
        ActuatorKind, ControlMeasurement, MeasurementKind, MeasurementPoint, OfflineActuator,
        OfflineLimits, OfflineSafety, SensorId,
    };
    use rhizo_mqtt_contract::safety::FIRMWARE_MAX_ML_PER_RUN;

    fn policy() -> OfflinePolicy {
        OfflinePolicy {
            plant_id: SensorId::parse("plant-01").unwrap(),
            policy_version: 1,
            enabled: false,
            actuator: Some(OfflineActuator {
                actuator_id: SensorId::parse("pump-0").unwrap(),
                kind: ActuatorKind::IrrigationPump,
                dose_ml: 35.,
                max_doses_per_cycle: 3,
                absorption_wait_ms: 1_800_000,
            }),
            control_measurement: ControlMeasurement {
                kind: MeasurementKind::SoilMoisture,
                point: MeasurementPoint::parse("default").unwrap(),
                trigger_below: 28.,
                resume_above: 45.,
                confirm_duration_ms: 1_800_000,
                max_age_ms: 900_000,
            },
            required_measurements: Vec::new(),
            advisory_measurements: Vec::new(),
            limits: OfflineLimits {
                cooldown_ms: 21_600_000,
                max_volume_per_window_ml: 300.,
                window_ms: 86_400_000,
            },
            safety: OfflineSafety {
                require_leak_clear: true,
                require_tank_above_percent: 15.,
                require_pump_healthy: true,
            },
        }
    }

    /// The authoring difference: the contract accepts a disabled policy
    /// unconditionally; authoring does not.
    #[test]
    fn a_disabled_policy_is_still_checked_at_authoring_time() {
        let mut bad = policy();
        bad.actuator.as_mut().unwrap().dose_ml = FIRMWARE_MAX_ML_PER_RUN + 0.1;
        assert_eq!(
            bad.validate(),
            Ok(()),
            "the contract skips a disabled policy"
        );
        assert_eq!(
            validate_authored(&bad),
            Err(PolicyError::DoseAboveHardLimit),
            "an operator must learn the limit while they are typing it"
        );
    }

    /// SAFETY-018: no actuator, no policy — whether it is enabled or not.
    #[test]
    fn safety_018_a_policy_without_an_actuator_is_rejected() {
        let mut orphan = policy();
        orphan.actuator = None;
        assert_eq!(
            validate_authored(&orphan),
            Err(PolicyError::MissingActuator)
        );
        orphan.enabled = true;
        assert_eq!(
            validate_authored(&orphan),
            Err(PolicyError::MissingActuator)
        );
    }

    /// Every other rule is the contract's, unchanged, and answers with the
    /// contract's own error value.
    #[test]
    fn every_other_rule_is_the_contracts_own() {
        assert_eq!(validate_authored(&policy()), Ok(()));

        let mut bad = policy();
        bad.control_measurement.resume_above = 10.0;
        assert_eq!(validate_authored(&bad), Err(PolicyError::InvalidHysteresis));

        let mut bad = policy();
        bad.control_measurement.kind = MeasurementKind::LeakState;
        assert_eq!(
            validate_authored(&bad),
            Err(PolicyError::NonScalarControlKind)
        );

        let mut bad = policy();
        bad.limits.max_volume_per_window_ml = 10.0;
        assert_eq!(
            validate_authored(&bad),
            Err(PolicyError::CycleVolumeAboveWindow)
        );

        // Validating does not enable: the caller's own value is untouched.
        let subject = policy();
        assert!(validate_authored(&subject).is_ok());
        assert!(!subject.enabled);
    }
}
