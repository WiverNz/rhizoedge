//! The versioned, embedded species preset catalogue (M5-017).
//!
//! Configuring a plant from nothing means choosing a target moisture band,
//! warning and critical thresholds, and a light and temperature expectation, per
//! measurement kind. An operator who has just bought a monstera does not know
//! any of those numbers, and the cost of guessing is either a dead plant or
//! automation they never enable.
//!
//! A **plant preset** is a reusable starting configuration for a species. Two
//! rules define its shape, and both are load-bearing:
//!
//! - **A preset is a template, exactly as `PlantProfile` is.** It is never
//!   runtime state. ADR-016 made per-plant bindings and [`MeasurementPolicy`]
//!   rows the authoritative configuration, and a preset reaches the plant
//!   *through* that model, never around it (M5-018).
//! - **A preset is not a schedule.** It stores what a species *prefers* — a
//!   moisture band, a temperature range, a pH range — and never "water every two
//!   days". A timer would be a second actuation authority that no sensor reading
//!   and no lockout could contradict, which is the failure this architecture
//!   exists to prevent. [`catalogue::tests`] asserts the absence over the whole
//!   catalogue.
//!
//! # Source facts and Rhizo defaults are different kinds of claim
//!
//! A horticultural source stating a figure, in that source's own units, is a
//! [`ProvenancedValue::SourceFact`]. A starting value Rhizo interpreted from it
//! is a [`ProvenancedValue::RhizoDefault`], and records what it was derived
//! from. A vendor's `soil_humidity = 6` on a 1-10 scale converted to a
//! volumetric water content is an interpretation with a guess inside it, and
//! presenting it as a measured fact gives a plausible number authority it has
//! not earned. The UI shows the difference (M12-017), and it cannot do that if
//! the domain has already flattened it. There is no third, unlabelled case: the
//! enum has exactly two variants and `provenance` is a required tag.
//!
//! # A preset describes a plant, not an installation
//!
//! Entries are keyed by [`MeasurementKind`] and carry **no** device, sensor,
//! point, or capability identity. A catalogue cannot know which probe is in
//! which pot, and the moment an entry named one it would be competing with
//! `SensorBinding` for the same decision.
//!
//! [`MeasurementPolicy`]: crate::plant::MeasurementPolicy
pub mod catalogue;

use rhizo_mqtt_contract::payload::MeasurementKind;
use serde::{Deserialize, Serialize};

use crate::plant::MeasurementPolicy;

/// One preference value, always labelled with where it came from.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "provenance", rename_all = "snake_case")]
pub enum ProvenancedValue {
    /// A figure a cited source stated, kept in that source's own units.
    SourceFact {
        /// The figure.
        value: f64,
        /// The units the source used, verbatim.
        units: String,
        /// What said it.
        source_ref: String,
    },
    /// A starting value Rhizo chose, with what it was derived from.
    RhizoDefault {
        /// The figure.
        value: f64,
        /// The reasoning, where one exists.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        derived_from: Option<String>,
    },
}

impl ProvenancedValue {
    /// The number, whatever kind of claim it is.
    #[must_use]
    pub const fn value(&self) -> f64 {
        match self {
            Self::SourceFact { value, .. } | Self::RhizoDefault { value, .. } => *value,
        }
    }
    /// Whether a cited source stated this figure.
    #[must_use]
    pub const fn is_source_fact(&self) -> bool {
        matches!(self, Self::SourceFact { .. })
    }
}

/// What a species prefers for one measurement kind.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MeasurementPreference {
    /// The kind. Never a sensor.
    pub kind: MeasurementKind,
    /// Lower end of the comfortable band.
    pub target_low: ProvenancedValue,
    /// Upper end of the comfortable band.
    pub target_high: ProvenancedValue,
    /// Lower warning bound.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning_low: Option<ProvenancedValue>,
    /// Upper warning bound.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning_high: Option<ProvenancedValue>,
    /// Lower critical bound.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub critical_low: Option<ProvenancedValue>,
    /// Upper critical bound.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub critical_high: Option<ProvenancedValue>,
}

impl MeasurementPreference {
    /// Materialises this preference into an ordinary per-plant policy.
    ///
    /// The result is indistinguishable from a hand-configured row: nothing here
    /// records that a preset produced it, because nothing downstream may behave
    /// differently if one did (M5-018).
    #[must_use]
    pub fn to_policy(
        &self,
        stale_after_ms: u32,
        confirm_duration_ms: Option<u32>,
    ) -> MeasurementPolicy {
        MeasurementPolicy {
            kind: self.kind.clone(),
            target_min: Some(self.target_low.value()),
            target_max: Some(self.target_high.value()),
            warning_low: self.warning_low.as_ref().map(ProvenancedValue::value),
            warning_high: self.warning_high.as_ref().map(ProvenancedValue::value),
            critical_low: self.critical_low.as_ref().map(ProvenancedValue::value),
            critical_high: self.critical_high.as_ref().map(ProvenancedValue::value),
            stale_after_ms,
            hysteresis: None,
            confirm_duration_ms,
        }
    }

    /// Every value the preference carries, in a stable order.
    #[must_use]
    pub fn values(&self) -> Vec<&ProvenancedValue> {
        let mut out = vec![&self.target_low, &self.target_high];
        out.extend(self.warning_low.as_ref());
        out.extend(self.warning_high.as_ref());
        out.extend(self.critical_low.as_ref());
        out.extend(self.critical_high.as_ref());
        out
    }
}

/// How much water a species wants per dose, expressed as a class.
///
/// Deliberately a class rather than a figure: a catalogue cannot know the pot,
/// and millilitres without a pot volume are meaningless. The class is resolved
/// against `pot_volume_ml` at application time, which keeps the dose a property
/// of the plant.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DoseClass {
    /// A sip. Succulents and anything that resents wet feet.
    Light,
    /// The usual houseplant dose.
    Moderate,
    /// A thirsty plant in active growth.
    Generous,
}

impl DoseClass {
    /// The fraction of pot volume one dose delivers.
    #[must_use]
    pub const fn fraction_of_pot(self) -> f64 {
        match self {
            Self::Light => 0.015,
            Self::Moderate => 0.025,
            Self::Generous => 0.040,
        }
    }
    /// The dose for a given pot.
    ///
    /// **Never clamped.** A pot large enough to want more than the firmware hard
    /// limit produces a value profile validation refuses, which is the correct
    /// answer: a curated catalogue is an input, not a trusted one (ADR-011).
    #[must_use]
    pub fn dose_ml(self, pot_volume_ml: f64) -> f32 {
        (pot_volume_ml * self.fraction_of_pot()) as f32
    }
}

/// How long a species wants between waterings, expressed as a class.
///
/// A **minimum spacing**, not a timer: it can only ever delay a dose the
/// measurements already justified. Nothing here can cause a watering.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CooldownClass {
    /// Thirsty: a short minimum spacing.
    Frequent,
    /// The usual houseplant spacing.
    Standard,
    /// Drought-adapted: a long minimum spacing.
    Infrequent,
}

impl CooldownClass {
    /// The minimum spacing in hours.
    #[must_use]
    pub const fn hours(self) -> f64 {
        match self {
            Self::Frequent => 12.0,
            Self::Standard => 24.0,
            Self::Infrequent => 72.0,
        }
    }
}

/// One curated species entry.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlantPreset {
    /// Stable identity. Never renumbered.
    pub preset_id: String,
    /// What an operator calls it.
    pub display_name: String,
    /// Botanical name.
    pub scientific_name: String,
    /// Common names, for search.
    #[serde(default)]
    pub synonyms: Vec<String>,
    /// Who compiled the entry.
    pub source: String,
    /// What the entry was compiled from.
    pub source_ref: String,
    /// The licence the entry ships under.
    pub license: String,
    /// When the entry was compiled, ISO 8601 date.
    pub retrieved_at: String,
    /// Suggested dose class.
    pub dose_class: DoseClass,
    /// Suggested minimum spacing class.
    pub cooldown_class: CooldownClass,
    /// Per-kind preferences.
    pub measurements: Vec<MeasurementPreference>,
}

impl PlantPreset {
    /// Whether the entry matches a free-text query by name or synonym.
    #[must_use]
    pub fn matches(&self, query: &str) -> bool {
        let needle = query.trim().to_lowercase();
        if needle.is_empty() {
            return true;
        }
        self.preset_id.to_lowercase().contains(&needle)
            || self.display_name.to_lowercase().contains(&needle)
            || self.scientific_name.to_lowercase().contains(&needle)
            || self
                .synonyms
                .iter()
                .any(|s| s.to_lowercase().contains(&needle))
    }

    /// The preference for one kind, if the entry has one.
    #[must_use]
    pub fn preference(&self, kind: &MeasurementKind) -> Option<&MeasurementPreference> {
        self.measurements.iter().find(|m| &m.kind == kind)
    }
}

/// The embedded catalogue.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Catalogue {
    /// Versioned with the binary, so a release always produces the same
    /// starting configuration.
    pub catalogue_version: u32,
    /// The curated entries.
    pub presets: Vec<PlantPreset>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_value_without_a_provenance_tag_does_not_exist() {
        let untagged = r#"{"value": 28.0}"#;
        assert!(
            serde_json::from_str::<ProvenancedValue>(untagged).is_err(),
            "there is no third, unlabelled case"
        );
        let fact: ProvenancedValue = serde_json::from_str(
            r#"{"provenance":"source_fact","value":6.0,"units":"1-10 soil humidity","source_ref":"a vendor scale"}"#,
        )
        .unwrap();
        assert!(fact.is_source_fact());
        assert_eq!(fact.value(), 6.0);
        let derived: ProvenancedValue = serde_json::from_str(
            r#"{"provenance":"rhizo_default","value":28.0,"derived_from":"the vendor scale, converted"}"#,
        )
        .unwrap();
        assert!(!derived.is_source_fact());
        assert_eq!(derived.value(), 28.0);
    }

    #[test]
    fn a_dose_class_is_resolved_against_the_pot_and_never_clamped() {
        assert!((DoseClass::Moderate.dose_ml(2_000.0) - 50.0).abs() < 1e-4);
        assert!((DoseClass::Light.dose_ml(2_000.0) - 30.0).abs() < 1e-4);
        // A large pot produces a dose above the firmware ceiling. That is the
        // catalogue being an input rather than an authority: profile validation
        // refuses it, and nothing here quietly shrinks it.
        assert!(DoseClass::Generous.dose_ml(4_000.0) > 80.0);
    }

    #[test]
    fn cooldown_classes_are_minimum_spacings() {
        assert!(CooldownClass::Frequent.hours() < CooldownClass::Standard.hours());
        assert!(CooldownClass::Standard.hours() < CooldownClass::Infrequent.hours());
    }
}
