//! The NVS offline-policy store and its atomic activation (M9-015, SAFETY-019).
//!
//! Power loss during a policy write is not exotic; it is the normal way an
//! unmaintained device eventually fails, and a half-written policy taking
//! effect is the failure this store exists to prevent.
//!
//! # The sequence, and why every step before the last is non-destructive
//!
//! ```text
//! validate -> stage -> verify read-back -> ACTIVATE (one flip) -> acknowledge
//! ```
//!
//! Everything up to the flip writes only `policy_staging`, so an interruption
//! anywhere leaves `policy_active` exactly as it was. After the flip the new
//! policy is complete, because the value flipped to has already been read back
//! and verified. [`UpdateStep`] enumerates the interruption points so a test
//! can stop at each one and assert that exactly one valid policy is active.
//!
//! # A corrupt store refuses; it does not fall back
//!
//! No default is substituted, ever. A default threshold nobody authorised is
//! precisely what SAFETY-013 forbids, and "absence is never permission" is the
//! rule the whole of offline autonomy rests on.

use rhizo_mqtt_contract::payload::{OfflinePolicy, OfflinePolicySet, PolicyError};

use crate::persist::{PersistedState, VersionedPolicy};

/// The steps of a policy update, in order.
///
/// Named so a test can interrupt at each index rather than at "somewhere in the
/// middle", which is the difference between checking the property and hoping.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum UpdateStep {
    /// Nothing has been written.
    BeforeValidate,
    /// Validation passed; nothing written yet.
    BeforeStage,
    /// The candidate is in `policy_staging`.
    AfterStage,
    /// The staged copy has been read back and its checksum verified.
    AfterVerify,
    /// `policy_active` now names the new policy.
    AfterActivate,
    /// Staging has been cleared.
    Complete,
}

impl UpdateStep {
    /// Every step, for exhaustive interruption tests.
    pub const ALL: [Self; 6] = [
        Self::BeforeValidate,
        Self::BeforeStage,
        Self::AfterStage,
        Self::AfterVerify,
        Self::AfterActivate,
        Self::Complete,
    ];
}

/// Why an update did not activate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PolicyRejection {
    /// The offered version is not newer than the active one.
    NotNewer,
    /// A policy in the set failed the shared validator.
    Invalid(PolicyError),
    /// The staged copy did not read back intact.
    StagingCorrupt,
}

/// What an update did.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PolicyOutcome {
    /// Activated; `applied_policy_versions` should report it.
    Activated {
        /// The version now in force.
        policy_version: u32,
    },
    /// Refused; the previous active policy is untouched.
    Refused(PolicyRejection),
}

/// The highest `policy_version` in a set.
///
/// The set is versioned as a whole for storage, because activation is one flip
/// and a per-policy flip would not be atomic across the set.
#[must_use]
pub fn set_version(set: &OfflinePolicySet) -> u32 {
    set.policies
        .iter()
        .map(|policy| policy.policy_version)
        .max()
        .unwrap_or(0)
}

/// Applies a retained `device.policy`, interrupted at `stop_after`.
///
/// `stop_after` exists for the interruption tests SAFETY-019 requires and is
/// [`UpdateStep::Complete`] in production. Simulating power loss by *not doing
/// the rest of the work* is the honest simulation: a real interruption is
/// exactly the absence of the remaining writes.
pub fn apply(
    state: &mut PersistedState,
    incoming: &OfflinePolicySet,
    stop_after: UpdateStep,
) -> PolicyOutcome {
    let incoming_version = set_version(incoming);
    if let Some(active) = state.policy_active.as_ref()
        && incoming_version <= active.policy_version
    {
        return PolicyOutcome::Refused(PolicyRejection::NotNewer);
    }
    for policy in &incoming.policies {
        if let Err(error) = policy.validate() {
            return PolicyOutcome::Refused(PolicyRejection::Invalid(error));
        }
    }
    if stop_after < UpdateStep::AfterStage {
        return PolicyOutcome::Refused(PolicyRejection::NotNewer);
    }

    state.policy_staging = Some(VersionedPolicy::seal(incoming_version, incoming.clone()));
    if stop_after < UpdateStep::AfterVerify {
        return PolicyOutcome::Refused(PolicyRejection::NotNewer);
    }

    // Read back and verify before anything destructive happens. A staged copy
    // that does not verify is discarded and the active policy never moves.
    let Some(staged) = state.policy_staging.clone() else {
        return PolicyOutcome::Refused(PolicyRejection::StagingCorrupt);
    };
    if !staged.checksum_valid() {
        state.policy_staging = None;
        return PolicyOutcome::Refused(PolicyRejection::StagingCorrupt);
    }
    if stop_after < UpdateStep::AfterActivate {
        return PolicyOutcome::Refused(PolicyRejection::NotNewer);
    }

    // The one atomic operation.
    state.policy_active = Some(staged);
    if stop_after < UpdateStep::Complete {
        return PolicyOutcome::Activated {
            policy_version: incoming_version,
        };
    }

    state.policy_staging = None;
    PolicyOutcome::Activated {
        policy_version: incoming_version,
    }
}

/// The active policy set, or `None` when there is none or it is corrupt.
///
/// **Corruption resolves to `None`, and `None` is not permission.** The caller
/// refuses to actuate; it never substitutes a default.
#[must_use]
pub fn active(state: &PersistedState) -> Option<&OfflinePolicySet> {
    let active = state.policy_active.as_ref()?;
    if !active.checksum_valid() {
        return None;
    }
    Some(&active.policies)
}

/// The active policy for one plant.
#[must_use]
pub fn active_for_plant<'a>(
    state: &'a PersistedState,
    plant_id: &str,
) -> Option<&'a OfflinePolicy> {
    active(state)?
        .policies
        .iter()
        .find(|policy| policy.plant_id.as_str() == plant_id)
}

/// The `applied_policy_versions` map for `device.status`.
#[must_use]
pub fn applied_versions(state: &PersistedState) -> std::collections::BTreeMap<String, u32> {
    active(state)
        .map(|set| {
            set.policies
                .iter()
                .map(|policy| (policy.plant_id.as_str().to_owned(), policy.policy_version))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rhizo_mqtt_contract::payload::{
        ActuatorKind, ControlMeasurement, MeasurementKind, MeasurementPoint, OfflineActuator,
        OfflineLimits, OfflineSafety, SensorId,
    };

    fn policy(version: u32, dose_ml: f32) -> OfflinePolicy {
        OfflinePolicy {
            plant_id: SensorId::parse("basil").expect("valid id"),
            policy_version: version,
            enabled: true,
            actuator: Some(OfflineActuator {
                actuator_id: SensorId::parse("pump-1").expect("valid id"),
                kind: ActuatorKind::IrrigationPump,
                dose_ml,
                max_doses_per_cycle: 2,
                absorption_wait_ms: 900_000,
            }),
            control_measurement: ControlMeasurement {
                kind: MeasurementKind::SoilMoisture,
                point: MeasurementPoint::parse("default").expect("valid point"),
                trigger_below: 25.0,
                resume_above: 35.0,
                confirm_duration_ms: 600_000,
                max_age_ms: 900_000,
            },
            required_measurements: Vec::new(),
            advisory_measurements: Vec::new(),
            limits: OfflineLimits {
                cooldown_ms: 3_600_000,
                max_volume_per_window_ml: 200.0,
                window_ms: 86_400_000,
            },
            safety: OfflineSafety {
                require_leak_clear: true,
                require_tank_above_percent: 15.0,
                require_pump_healthy: true,
            },
        }
    }

    fn set(version: u32, dose_ml: f32) -> OfflinePolicySet {
        OfflinePolicySet {
            policies: vec![policy(version, dose_ml)],
        }
    }

    #[test]
    fn a_valid_policy_is_staged_verified_activated_and_reported() {
        let mut state = PersistedState::default();
        assert_eq!(
            apply(&mut state, &set(3, 40.0), UpdateStep::Complete),
            PolicyOutcome::Activated { policy_version: 3 }
        );
        assert!(state.policy_staging.is_none());
        assert_eq!(applied_versions(&state).get("basil"), Some(&3));
    }

    /// SAFETY-019. Interruption at every step leaves exactly one valid active
    /// policy: either the previous one, or the complete new one — never a
    /// half-written third thing.
    #[test]
    fn safety_019_interruption_at_every_step_leaves_one_valid_active_policy() {
        for stop in UpdateStep::ALL {
            let mut state = PersistedState::default();
            apply(&mut state, &set(1, 20.0), UpdateStep::Complete);
            let before = state.policy_active.clone().expect("a policy is active");

            apply(&mut state, &set(2, 40.0), stop);

            let after = state
                .policy_active
                .clone()
                .expect("a policy is still active");
            assert!(after.checksum_valid(), "{stop:?}: active policy is intact");
            let activated = stop >= UpdateStep::AfterActivate;
            if activated {
                assert_eq!(after.policy_version, 2, "{stop:?}");
            } else {
                assert_eq!(after, before, "{stop:?}: the previous policy is untouched");
            }
            // Whatever is in staging, it never becomes active by itself.
            if let Some(staged) = state.policy_staging.as_ref() {
                assert!(staged.checksum_valid(), "{stop:?}");
            }
        }
    }

    #[test]
    fn a_lower_or_equal_version_is_ignored() {
        let mut state = PersistedState::default();
        apply(&mut state, &set(5, 40.0), UpdateStep::Complete);
        assert_eq!(
            apply(&mut state, &set(5, 10.0), UpdateStep::Complete),
            PolicyOutcome::Refused(PolicyRejection::NotNewer)
        );
        assert_eq!(
            apply(&mut state, &set(4, 10.0), UpdateStep::Complete),
            PolicyOutcome::Refused(PolicyRejection::NotNewer)
        );
        assert_eq!(applied_versions(&state).get("basil"), Some(&5));
    }

    #[test]
    fn an_invalid_policy_leaves_the_previous_one_active() {
        let mut state = PersistedState::default();
        apply(&mut state, &set(1, 40.0), UpdateStep::Complete);
        assert_eq!(
            apply(&mut state, &set(2, 5_000.0), UpdateStep::Complete),
            PolicyOutcome::Refused(PolicyRejection::Invalid(PolicyError::DoseAboveHardLimit))
        );
        assert_eq!(
            state.policy_active.as_ref().map(|p| p.policy_version),
            Some(1)
        );
    }

    /// SAFETY-013: a corrupt store refuses and substitutes **no** default.
    #[test]
    fn safety_013_a_corrupt_store_refuses_and_substitutes_no_default() {
        let mut state = PersistedState::default();
        apply(&mut state, &set(1, 40.0), UpdateStep::Complete);
        if let Some(active) = state.policy_active.as_mut() {
            active.crc32 ^= 0xffff_ffff;
        }
        assert!(active(&state).is_none(), "a corrupt policy is not usable");
        assert!(active_for_plant(&state, "basil").is_none());
        assert!(applied_versions(&state).is_empty());
    }

    #[test]
    fn a_device_with_no_policy_has_none_and_that_is_not_permission() {
        let state = PersistedState::default();
        assert!(active(&state).is_none());
        assert!(active_for_plant(&state, "basil").is_none());
    }
}
