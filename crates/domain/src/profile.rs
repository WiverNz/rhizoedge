//! The plant-profile template and its validation.
//!
//! A profile is a **template** ([ADR-016](../../../docs/adr/016-plant-binding-and-policy-model.md)):
//! it seeds a plant's `MeasurementPolicy` rows and automation defaults at
//! creation time and then stops mattering. Nothing here is authoritative
//! runtime configuration.
//!
//! # Rejection, never clamping
//!
//! [ADR-011](../../../docs/adr/011-configuration-and-secrets-model.md): silent
//! clamping means the operator believes something false about their system and
//! discovers it during an incident. Every rule below returns its own error
//! variant naming the value and the limit, so the message an operator sees at
//! edit time teaches the real limit while they are still paying attention.
//!
//! The hard-limit checks read the constants from `rhizo-mqtt-contract`, so a
//! firmware limit change automatically tightens profile validation rather than
//! leaving a second copy of the number to drift.
use crate::ProfileId;
use chrono::{DateTime, Duration, Utc};
use rhizo_mqtt_contract::safety::{FIRMWARE_MAX_DAILY_ML, FIRMWARE_MAX_ML_PER_RUN};
use serde::{Deserialize, Serialize};

/// Template used only to seed plant-owned policies.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlantProfile {
    /** Template id. */
    pub profile_id: ProfileId,
    /** Human name. */
    pub name: String,
    /** Suggested lower moisture target. */
    pub target_min_vwc: f64,
    /** Suggested upper moisture target. */
    pub target_max_vwc: f64,
    /** Suggested fixed dose. */
    pub dose_ml: f32,
    /** Doses allowed inside one drying cycle. */
    pub max_doses_per_cycle: u16,
    /** Rolling 24-hour automatic ceiling. */
    pub max_daily_ml: f32,
    /** Debounce separating `Drying` from `DryConfirmed`. */
    pub dry_confirm_minutes: u32,
    /** Minimum spacing between waterings. */
    pub cooldown_hours: f64,
    /** How long a dose is given to reach the probe. */
    pub absorption_minutes: u32,
}

/// One violated profile rule. Each rule has its own variant on purpose: the API
/// renders the variant, and a test asserts on it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ProfileError {
    /// A field was not a finite number.
    NotFinite {
        /// Field name as the API spells it.
        field: &'static str,
    },
    /// `target_min_vwc >= target_max_vwc`, or either outside 0..=100.
    TargetRange {
        /// Configured minimum.
        min: f64,
        /// Configured maximum.
        max: f64,
    },
    /// `dose_ml` is zero or negative.
    DoseInvalid {
        /// Configured dose.
        dose_ml: f32,
    },
    /// `max_doses_per_cycle` is zero, so no cycle could ever water.
    ZeroDoses,
    /// One cycle at the configured dose would exceed the daily ceiling.
    CycleVolumeAboveDailyMax {
        /// Configured dose.
        dose_ml: f32,
        /// Configured doses per cycle.
        max_doses_per_cycle: u16,
        /// Configured daily ceiling.
        max_daily_ml: f32,
    },
    /// An interval was zero or negative.
    NonPositiveInterval {
        /// Field name as the API spells it.
        field: &'static str,
    },
    /// `dose_ml` exceeds the device hard limit. **Rejected, never clamped.**
    DoseAboveFirmwareLimit {
        /// Configured dose.
        dose_ml: f32,
        /// `FIRMWARE_MAX_ML_PER_RUN`.
        limit: f32,
    },
    /// `max_daily_ml` exceeds the device rolling budget.
    DailyAboveFirmwareLimit {
        /// Configured ceiling.
        max_daily_ml: f32,
        /// `FIRMWARE_MAX_DAILY_ML`.
        limit: f32,
    },
}

impl ProfileError {
    /// The stable machine-readable rule name, used as the API error code.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::NotFinite { .. } => "not_finite",
            Self::TargetRange { .. } => "target_range",
            Self::DoseInvalid { .. } => "dose_invalid",
            Self::ZeroDoses => "zero_doses",
            Self::CycleVolumeAboveDailyMax { .. } => "cycle_volume_above_daily_max",
            Self::NonPositiveInterval { .. } => "non_positive_interval",
            Self::DoseAboveFirmwareLimit { .. } => "dose_above_firmware_limit",
            Self::DailyAboveFirmwareLimit { .. } => "daily_above_firmware_limit",
        }
    }
}

impl core::fmt::Display for ProfileError {
    /// The sentence an operator reads. It names the value **and** the limit,
    /// because a bare rejection teaches nothing (ADR-011).
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NotFinite { field } => write!(f, "{field} must be a finite number"),
            Self::TargetRange { min, max } => write!(
                f,
                "target_min_vwc ({min}) must be below target_max_vwc ({max}), and both within 0-100"
            ),
            Self::DoseInvalid { dose_ml } => {
                write!(f, "dose_ml ({dose_ml}) must be greater than zero")
            }
            Self::ZeroDoses => write!(f, "max_doses_per_cycle must be at least 1"),
            Self::CycleVolumeAboveDailyMax {
                dose_ml,
                max_doses_per_cycle,
                max_daily_ml,
            } => write!(
                f,
                "dose_ml ({dose_ml}) x max_doses_per_cycle ({max_doses_per_cycle}) exceeds max_daily_ml ({max_daily_ml})"
            ),
            Self::NonPositiveInterval { field } => {
                write!(f, "{field} must be greater than zero")
            }
            Self::DoseAboveFirmwareLimit { dose_ml, limit } => write!(
                f,
                "dose_ml ({dose_ml}) exceeds the device hard limit FIRMWARE_MAX_ML_PER_RUN ({limit})"
            ),
            Self::DailyAboveFirmwareLimit {
                max_daily_ml,
                limit,
            } => write!(
                f,
                "max_daily_ml ({max_daily_ml}) exceeds the device hard limit FIRMWARE_MAX_DAILY_ML ({limit})"
            ),
        }
    }
}

impl PlantProfile {
    /// Validates internal coherence and the firmware hard limits.
    ///
    /// Pure: no I/O, no clock. The same rules therefore apply whether a profile
    /// arrives over HTTP, from a fixture, from a preset materialisation
    /// (M5-018), or from a future import.
    ///
    /// # Errors
    ///
    /// Returns the first violated rule, as its own variant.
    pub fn validate(&self) -> Result<(), ProfileError> {
        for (field, value) in [
            ("target_min_vwc", self.target_min_vwc),
            ("target_max_vwc", self.target_max_vwc),
            ("cooldown_hours", self.cooldown_hours),
        ] {
            if !value.is_finite() {
                return Err(ProfileError::NotFinite { field });
            }
        }
        for (field, value) in [
            ("dose_ml", self.dose_ml),
            ("max_daily_ml", self.max_daily_ml),
        ] {
            if !value.is_finite() {
                return Err(ProfileError::NotFinite { field });
            }
        }
        if self.target_min_vwc >= self.target_max_vwc
            || self.target_min_vwc < 0.0
            || self.target_max_vwc > 100.0
        {
            return Err(ProfileError::TargetRange {
                min: self.target_min_vwc,
                max: self.target_max_vwc,
            });
        }
        if self.dose_ml <= 0.0 {
            return Err(ProfileError::DoseInvalid {
                dose_ml: self.dose_ml,
            });
        }
        if self.max_doses_per_cycle == 0 {
            return Err(ProfileError::ZeroDoses);
        }
        if self.dry_confirm_minutes == 0 {
            return Err(ProfileError::NonPositiveInterval {
                field: "dry_confirm_minutes",
            });
        }
        if self.absorption_minutes == 0 {
            return Err(ProfileError::NonPositiveInterval {
                field: "absorption_minutes",
            });
        }
        if self.cooldown_hours <= 0.0 {
            return Err(ProfileError::NonPositiveInterval {
                field: "cooldown_hours",
            });
        }
        // The hard limits come before the internal-consistency volume rule so a
        // profile that breaks a device ceiling names the ceiling, which is the
        // number the operator actually needs (ADR-011).
        if self.dose_ml > FIRMWARE_MAX_ML_PER_RUN {
            return Err(ProfileError::DoseAboveFirmwareLimit {
                dose_ml: self.dose_ml,
                limit: FIRMWARE_MAX_ML_PER_RUN,
            });
        }
        if self.max_daily_ml > FIRMWARE_MAX_DAILY_ML {
            return Err(ProfileError::DailyAboveFirmwareLimit {
                max_daily_ml: self.max_daily_ml,
                limit: FIRMWARE_MAX_DAILY_ML,
            });
        }
        if self.dose_ml * f32::from(self.max_doses_per_cycle) > self.max_daily_ml {
            return Err(ProfileError::CycleVolumeAboveDailyMax {
                dose_ml: self.dose_ml,
                max_doses_per_cycle: self.max_doses_per_cycle,
                max_daily_ml: self.max_daily_ml,
            });
        }
        Ok(())
    }

    /// Whether the profile satisfies every rule.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.validate().is_ok()
    }

    /// The profile a first-run system starts with (M5-004).
    ///
    /// Every number is a conservative starting point, not a claim about any
    /// particular plant: the dose is well under the firmware ceiling and the
    /// daily budget allows a full cycle with headroom.
    #[must_use]
    pub fn default_seed(profile_id: ProfileId) -> Self {
        Self {
            profile_id,
            name: String::from("Default"),
            target_min_vwc: 28.0,
            target_max_vwc: 45.0,
            dose_ml: 40.0,
            max_doses_per_cycle: 3,
            max_daily_ml: 300.0,
            dry_confirm_minutes: 30,
            cooldown_hours: 6.0,
            absorption_minutes: 30,
        }
    }
}

/// Backward-compatible soil-domain view used by the recommendation work.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SoilSample {
    /** Moisture percentage. */
    pub moisture_vwc: Option<f64>,
    /** Edge authoritative receipt time. */
    pub received_at: DateTime<Utc>,
}
impl SoilSample {
    /** Physical plausibility only. */
    pub fn is_valid(&self) -> bool {
        self.moisture_vwc
            .is_some_and(|v| v.is_finite() && (0.0..=100.0).contains(&v))
    }
    /** Uses edge receipt time and is stale at the exact boundary. */
    pub fn is_stale(&self, now: DateTime<Utc>, max_age: Duration) -> bool {
        now.signed_duration_since(self.received_at) >= max_age
    }
}

#[cfg(test)]
mod validate {
    use super::*;
    use chrono::TimeZone;

    fn profile() -> PlantProfile {
        PlantProfile::default_seed(ProfileId::from_uuid(uuid::Uuid::nil()))
    }

    #[test]
    fn validity_and_staleness_boundaries() {
        let at = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        let s = SoilSample {
            moisture_vwc: Some(0.),
            received_at: at,
        };
        assert!(s.is_valid());
        assert!(!s.is_stale(at + Duration::seconds(9), Duration::seconds(10)));
        assert!(s.is_stale(at + Duration::seconds(10), Duration::seconds(10)));
    }

    #[test]
    fn the_seeded_default_profile_is_valid() {
        assert_eq!(profile().validate(), Ok(()));
    }

    /// One rule at a time, each with its own variant.
    #[test]
    fn each_rule_rejects_with_its_own_variant() {
        let mut p = profile();
        p.target_min_vwc = 45.0;
        p.target_max_vwc = 45.0;
        assert_eq!(
            p.validate(),
            Err(ProfileError::TargetRange {
                min: 45.0,
                max: 45.0
            })
        );

        let mut p = profile();
        p.dose_ml = 0.0;
        assert_eq!(
            p.validate(),
            Err(ProfileError::DoseInvalid { dose_ml: 0.0 })
        );

        let mut p = profile();
        p.max_doses_per_cycle = 0;
        assert_eq!(p.validate(), Err(ProfileError::ZeroDoses));

        let mut p = profile();
        p.max_doses_per_cycle = 10;
        p.max_daily_ml = 300.0;
        assert_eq!(
            p.validate(),
            Err(ProfileError::CycleVolumeAboveDailyMax {
                dose_ml: 40.0,
                max_doses_per_cycle: 10,
                max_daily_ml: 300.0
            })
        );

        for field in [
            "dry_confirm_minutes",
            "absorption_minutes",
            "cooldown_hours",
        ] {
            let mut p = profile();
            match field {
                "dry_confirm_minutes" => p.dry_confirm_minutes = 0,
                "absorption_minutes" => p.absorption_minutes = 0,
                _ => p.cooldown_hours = 0.0,
            }
            assert_eq!(
                p.validate(),
                Err(ProfileError::NonPositiveInterval { field })
            );
        }

        let mut p = profile();
        p.cooldown_hours = f64::NAN;
        assert_eq!(
            p.validate(),
            Err(ProfileError::NotFinite {
                field: "cooldown_hours"
            })
        );
    }

    /// The rule PRD 050 F-050-03 names explicitly, and the one ADR-011 exists
    /// for: 200 ml against an 80 ml hard limit is **refused**, and the refusal
    /// carries the limit so the operator learns it.
    #[test]
    fn a_dose_above_the_firmware_limit_is_rejected_and_never_clamped() {
        let mut p = profile();
        p.dose_ml = 200.0;
        p.max_daily_ml = 500.0;
        let error = p.validate().unwrap_err();
        assert_eq!(
            error,
            ProfileError::DoseAboveFirmwareLimit {
                dose_ml: 200.0,
                limit: FIRMWARE_MAX_ML_PER_RUN
            }
        );
        let rendered = error.to_string();
        assert!(rendered.contains("FIRMWARE_MAX_ML_PER_RUN"), "{rendered}");
        assert!(rendered.contains("200"), "{rendered}");
        assert!(rendered.contains("80"), "{rendered}");
        // Validation is a read-only check: the offending value is untouched.
        assert_eq!(p.dose_ml, 200.0, "validation must never clamp");

        let mut p = profile();
        p.max_daily_ml = FIRMWARE_MAX_DAILY_ML + 0.1;
        assert_eq!(
            p.validate(),
            Err(ProfileError::DailyAboveFirmwareLimit {
                max_daily_ml: FIRMWARE_MAX_DAILY_ML + 0.1,
                limit: FIRMWARE_MAX_DAILY_ML
            })
        );
    }

    /// A value exactly at a limit is inside it.
    #[test]
    fn boundary_values_are_accepted() {
        let mut p = profile();
        p.dose_ml = FIRMWARE_MAX_ML_PER_RUN;
        p.max_doses_per_cycle = 1;
        p.max_daily_ml = FIRMWARE_MAX_DAILY_ML;
        assert_eq!(p.validate(), Ok(()));

        // dose x doses exactly equal to the daily ceiling is allowed.
        let mut p = profile();
        p.dose_ml = 50.0;
        p.max_doses_per_cycle = 6;
        p.max_daily_ml = 300.0;
        assert_eq!(p.validate(), Ok(()));

        let mut p = profile();
        p.target_min_vwc = 0.0;
        p.target_max_vwc = 100.0;
        assert_eq!(p.validate(), Ok(()));
    }

    /// Every variant renders a distinct code, so the API can key on it.
    #[test]
    fn every_error_code_is_distinct() {
        let codes = [
            ProfileError::NotFinite { field: "x" }.code(),
            ProfileError::TargetRange { min: 0., max: 0. }.code(),
            ProfileError::DoseInvalid { dose_ml: 0. }.code(),
            ProfileError::ZeroDoses.code(),
            ProfileError::CycleVolumeAboveDailyMax {
                dose_ml: 0.,
                max_doses_per_cycle: 0,
                max_daily_ml: 0.,
            }
            .code(),
            ProfileError::NonPositiveInterval { field: "x" }.code(),
            ProfileError::DoseAboveFirmwareLimit {
                dose_ml: 0.,
                limit: 0.,
            }
            .code(),
            ProfileError::DailyAboveFirmwareLimit {
                max_daily_ml: 0.,
                limit: 0.,
            }
            .code(),
        ];
        let mut sorted = codes.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), codes.len());
    }
}
