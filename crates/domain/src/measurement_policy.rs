//! Per-plant, per-kind threshold configuration (M5-014, ADR-016).
//!
//! Different plants legitimately disagree about what a reading means: the same
//! room temperature is fine for one plant and critical for another. Thresholds
//! therefore belong to the **plant**, not to the sensor — which is why two
//! plants can hold different policies for one shared probe.
//!
//! # Profiles seed and then stop mattering
//!
//! [`MeasurementPolicy::seeded_from_profile`] copies numbers **once**, at plant
//! creation. Editing `monstera_default` afterwards must not rewrite twelve
//! plants' thresholds: silently changing the irrigation rules of existing plants
//! is not a feature (ADR-016). Nothing in this module reads a profile at
//! evaluation time, and there is no back-reference to read one through.
//!
//! # The bands must nest
//!
//! `critical_low <= warning_low < warning_high <= critical_high`. A
//! configuration where warning sits outside critical is incoherent and would
//! produce alerts in an order nobody expects, so it is rejected rather than
//! interpreted.
use rhizo_mqtt_contract::payload::MeasurementKind;

use crate::plant::MeasurementPolicy;
use crate::profile::PlantProfile;

/// A refused policy, one variant per rule.
#[derive(Clone, Debug, PartialEq)]
pub enum MeasurementPolicyError {
    /// A supplied value was not a finite number.
    NotFinite {
        /// Field name as the API spells it.
        field: &'static str,
    },
    /// `target_min >= target_max`.
    TargetRange {
        /// Configured minimum.
        min: f64,
        /// Configured maximum.
        max: f64,
    },
    /// The warning band is not inside the critical band.
    BandsNotNested {
        /// Human description of the offending pair.
        detail: &'static str,
    },
    /// `stale_after_ms` is required and must be positive.
    StaleAfterNotPositive,
    /// A duration was zero where a positive value is required.
    NonPositiveDuration {
        /// Field name as the API spells it.
        field: &'static str,
    },
    /// Hysteresis must be finite and not negative.
    HysteresisInvalid {
        /// Configured hysteresis.
        hysteresis: f64,
    },
    /// The kind is not one this contract version recognises.
    UnknownKind {
        /// Kind as it arrived.
        kind: String,
    },
}

impl MeasurementPolicyError {
    /// The stable API error code.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::NotFinite { .. } => "not_finite",
            Self::TargetRange { .. } => "target_range",
            Self::BandsNotNested { .. } => "bands_not_nested",
            Self::StaleAfterNotPositive => "stale_after_not_positive",
            Self::NonPositiveDuration { .. } => "non_positive_duration",
            Self::HysteresisInvalid { .. } => "hysteresis_invalid",
            Self::UnknownKind { .. } => "unknown_kind",
        }
    }
}

impl core::fmt::Display for MeasurementPolicyError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NotFinite { field } => write!(f, "{field} must be a finite number"),
            Self::TargetRange { min, max } => {
                write!(f, "target_min ({min}) must be below target_max ({max})")
            }
            Self::BandsNotNested { detail } => write!(
                f,
                "the warning band must lie inside the critical band: {detail}"
            ),
            Self::StaleAfterNotPositive => {
                write!(
                    f,
                    "stale_after_ms is required and must be greater than zero"
                )
            }
            Self::NonPositiveDuration { field } => write!(f, "{field} must be greater than zero"),
            Self::HysteresisInvalid { hysteresis } => {
                write!(
                    f,
                    "hysteresis ({hysteresis}) must be finite and not negative"
                )
            }
            Self::UnknownKind { kind } => write!(
                f,
                "{kind} is not a measurement kind this version recognises"
            ),
        }
    }
}

/// Extension methods on the ADR-016 policy record.
pub trait MeasurementPolicyRules {
    /// Validates the policy.
    ///
    /// # Errors
    ///
    /// Returns the first violated rule.
    fn validate(&self) -> Result<(), MeasurementPolicyError>;

    /// Seeds a policy for one kind from the plant's profile template.
    ///
    /// Only `soil_moisture` takes its target band from the profile; other kinds
    /// are seeded with a freshness horizon and nothing else, because a profile
    /// has nothing to say about ambient temperature. `None` optional fields are
    /// genuinely optional and never block evaluation.
    fn seeded_from_profile(
        kind: MeasurementKind,
        profile: &PlantProfile,
        stale_after_ms: u32,
    ) -> Self;
}

impl MeasurementPolicyRules for MeasurementPolicy {
    fn validate(&self) -> Result<(), MeasurementPolicyError> {
        if !self.kind.is_known() {
            return Err(MeasurementPolicyError::UnknownKind {
                kind: self.kind.as_str().to_owned(),
            });
        }
        for (field, value) in [
            ("target_min", self.target_min),
            ("target_max", self.target_max),
            ("warning_low", self.warning_low),
            ("warning_high", self.warning_high),
            ("critical_low", self.critical_low),
            ("critical_high", self.critical_high),
        ] {
            if value.is_some_and(|v| !v.is_finite()) {
                return Err(MeasurementPolicyError::NotFinite { field });
            }
        }
        if let (Some(min), Some(max)) = (self.target_min, self.target_max)
            && min >= max
        {
            return Err(MeasurementPolicyError::TargetRange { min, max });
        }
        if self.stale_after_ms == 0 {
            return Err(MeasurementPolicyError::StaleAfterNotPositive);
        }
        if self.confirm_duration_ms == Some(0) {
            return Err(MeasurementPolicyError::NonPositiveDuration {
                field: "confirm_duration_ms",
            });
        }
        if let Some(h) = self.hysteresis
            && (!h.is_finite() || h < 0.0)
        {
            return Err(MeasurementPolicyError::HysteresisInvalid { hysteresis: h });
        }
        if let (Some(low), Some(high)) = (self.warning_low, self.warning_high)
            && low >= high
        {
            return Err(MeasurementPolicyError::BandsNotNested {
                detail: "warning_low must be below warning_high",
            });
        }
        if let (Some(low), Some(high)) = (self.critical_low, self.critical_high)
            && low >= high
        {
            return Err(MeasurementPolicyError::BandsNotNested {
                detail: "critical_low must be below critical_high",
            });
        }
        if let (Some(critical), Some(warning)) = (self.critical_low, self.warning_low)
            && critical > warning
        {
            return Err(MeasurementPolicyError::BandsNotNested {
                detail: "critical_low must be at or below warning_low",
            });
        }
        if let (Some(warning), Some(critical)) = (self.warning_high, self.critical_high)
            && warning > critical
        {
            return Err(MeasurementPolicyError::BandsNotNested {
                detail: "warning_high must be at or below critical_high",
            });
        }
        Ok(())
    }

    fn seeded_from_profile(
        kind: MeasurementKind,
        profile: &PlantProfile,
        stale_after_ms: u32,
    ) -> Self {
        let moisture = kind == MeasurementKind::SoilMoisture;
        Self {
            kind,
            target_min: moisture.then_some(profile.target_min_vwc),
            target_max: moisture.then_some(profile.target_max_vwc),
            warning_low: None,
            warning_high: None,
            critical_low: None,
            critical_high: None,
            stale_after_ms,
            hysteresis: None,
            confirm_duration_ms: moisture
                .then(|| profile.dry_confirm_minutes.saturating_mul(60_000)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ProfileId;

    fn policy() -> MeasurementPolicy {
        MeasurementPolicy {
            kind: MeasurementKind::AmbientTemperature,
            target_min: Some(18.0),
            target_max: Some(27.0),
            warning_low: Some(12.0),
            warning_high: Some(30.0),
            critical_low: Some(5.0),
            critical_high: Some(35.0),
            stale_after_ms: 900_000,
            hysteresis: Some(0.5),
            confirm_duration_ms: Some(600_000),
        }
    }

    #[test]
    fn a_coherent_policy_passes() {
        assert_eq!(policy().validate(), Ok(()));
    }

    #[test]
    fn each_rule_rejects_with_its_own_variant() {
        let mut p = policy();
        p.target_min = Some(30.0);
        assert_eq!(
            p.validate(),
            Err(MeasurementPolicyError::TargetRange {
                min: 30.0,
                max: 27.0
            })
        );

        let mut p = policy();
        p.stale_after_ms = 0;
        assert_eq!(
            p.validate(),
            Err(MeasurementPolicyError::StaleAfterNotPositive)
        );

        let mut p = policy();
        p.confirm_duration_ms = Some(0);
        assert_eq!(
            p.validate(),
            Err(MeasurementPolicyError::NonPositiveDuration {
                field: "confirm_duration_ms"
            })
        );

        let mut p = policy();
        p.hysteresis = Some(-1.0);
        assert_eq!(
            p.validate(),
            Err(MeasurementPolicyError::HysteresisInvalid { hysteresis: -1.0 })
        );

        let mut p = policy();
        p.warning_high = Some(f64::NAN);
        assert_eq!(
            p.validate(),
            Err(MeasurementPolicyError::NotFinite {
                field: "warning_high"
            })
        );

        let mut p = policy();
        p.kind = MeasurementKind::Unknown("future_kind".into());
        assert_eq!(
            p.validate(),
            Err(MeasurementPolicyError::UnknownKind {
                kind: "future_kind".into()
            })
        );
    }

    /// A warning band outside the critical band would alert in an order nobody
    /// expects, so it is refused rather than interpreted.
    #[test]
    fn the_warning_band_must_nest_inside_the_critical_band() {
        let mut p = policy();
        p.critical_low = Some(15.0);
        assert!(matches!(
            p.validate(),
            Err(MeasurementPolicyError::BandsNotNested { .. })
        ));

        let mut p = policy();
        p.critical_high = Some(28.0);
        assert!(matches!(
            p.validate(),
            Err(MeasurementPolicyError::BandsNotNested { .. })
        ));

        let mut p = policy();
        p.warning_low = Some(31.0);
        assert!(matches!(
            p.validate(),
            Err(MeasurementPolicyError::BandsNotNested { .. })
        ));

        // Exactly coincident bounds nest.
        let mut p = policy();
        p.critical_low = Some(12.0);
        p.critical_high = Some(30.0);
        assert_eq!(p.validate(), Ok(()));
    }

    #[test]
    fn optional_fields_are_genuinely_optional() {
        let bare = MeasurementPolicy {
            kind: MeasurementKind::Illuminance,
            target_min: None,
            target_max: None,
            warning_low: None,
            warning_high: None,
            critical_low: None,
            critical_high: None,
            stale_after_ms: 900_000,
            hysteresis: None,
            confirm_duration_ms: None,
        };
        assert_eq!(bare.validate(), Ok(()));
    }

    /// A profile seeds once. Editing it afterwards changes nothing that has
    /// already been seeded — there is no reference back to it to change.
    #[test]
    fn a_profile_seeds_and_then_stops_mattering() {
        let mut profile = PlantProfile::default_seed(ProfileId::from_uuid(uuid::Uuid::nil()));
        let seeded = MeasurementPolicy::seeded_from_profile(
            MeasurementKind::SoilMoisture,
            &profile,
            900_000,
        );
        assert_eq!(seeded.target_min, Some(28.0));
        assert_eq!(seeded.confirm_duration_ms, Some(30 * 60_000));
        assert_eq!(seeded.validate(), Ok(()));

        profile.target_min_vwc = 5.0;
        assert_eq!(
            seeded.target_min,
            Some(28.0),
            "an already-seeded policy is the plant's, not the template's"
        );

        // A kind the profile has nothing to say about is seeded with a
        // freshness horizon and no invented band.
        let ambient = MeasurementPolicy::seeded_from_profile(
            MeasurementKind::AmbientTemperature,
            &profile,
            900_000,
        );
        assert_eq!(ambient.target_min, None);
        assert_eq!(ambient.confirm_duration_ms, None);
        assert_eq!(ambient.validate(), Ok(()));
    }

    /// Two plants sharing one probe hold their own interpretations of it.
    #[test]
    fn two_plants_may_hold_different_thresholds_for_one_shared_sensor() {
        let fern = MeasurementPolicy {
            critical_low: Some(10.0),
            ..policy()
        };
        let succulent = MeasurementPolicy {
            critical_low: Some(0.0),
            warning_low: Some(4.0),
            ..policy()
        };
        assert_eq!(fern.validate(), Ok(()));
        assert_eq!(succulent.validate(), Ok(()));
        assert_ne!(fern.critical_low, succulent.critical_low);
        assert_eq!(fern.kind, succulent.kind);
    }
}
