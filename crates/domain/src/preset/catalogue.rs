//! Loading, validating, and searching the embedded catalogue.
//!
//! **Offline is not a feature here, it is the architecture.** The README puts
//! the cloud at optional and absent-for-a-week; a catalogue behind an HTTP call
//! would make creating a plant the one operation that needs the internet.
//! Embedding it also means the catalogue is versioned with the binary, so a
//! given release always produces the same starting configuration.
//!
//! The JSON is compiled in with [`include_str!`], so nothing here performs I/O
//! and `rhizo-domain` stays pure. Parsing happens once, lazily, into a
//! process-lifetime value.
use std::sync::OnceLock;

use super::{Catalogue, PlantPreset};

/// The catalogue data, compiled into the binary.
const CATALOGUE_JSON: &str = include_str!("../../data/presets.v1.json");

/// A structurally invalid catalogue.
#[derive(Clone, Debug, PartialEq)]
pub enum CatalogueError {
    /// The embedded JSON did not parse.
    Malformed(String),
    /// Two entries share a `preset_id`.
    DuplicateId(String),
    /// An entry is missing provenance metadata.
    MissingProvenanceMetadata {
        /// The offending entry.
        preset_id: String,
        /// The missing field.
        field: &'static str,
    },
    /// A range is inverted.
    InvertedRange {
        /// The offending entry.
        preset_id: String,
        /// The offending kind.
        kind: String,
        /// Which pair.
        detail: &'static str,
    },
    /// A value was not a finite number.
    NotFinite {
        /// The offending entry.
        preset_id: String,
        /// The offending kind.
        kind: String,
    },
    /// An entry names a kind this contract version does not recognise.
    UnknownKind {
        /// The offending entry.
        preset_id: String,
        /// The kind as written.
        kind: String,
    },
}

/// Parses and validates the embedded catalogue.
///
/// # Errors
///
/// Returns the first structural defect found.
pub fn parse(json: &str) -> Result<Catalogue, CatalogueError> {
    let catalogue: Catalogue =
        serde_json::from_str(json).map_err(|e| CatalogueError::Malformed(e.to_string()))?;
    validate(&catalogue)?;
    Ok(catalogue)
}

/// Checks the rules a catalogue entry must satisfy to be usable.
///
/// # Errors
///
/// Returns the first structural defect found.
pub fn validate(catalogue: &Catalogue) -> Result<(), CatalogueError> {
    let mut seen: Vec<&str> = Vec::new();
    for preset in &catalogue.presets {
        if seen.contains(&preset.preset_id.as_str()) {
            return Err(CatalogueError::DuplicateId(preset.preset_id.clone()));
        }
        seen.push(&preset.preset_id);
        for (field, value) in [
            ("source", &preset.source),
            ("source_ref", &preset.source_ref),
            ("license", &preset.license),
            ("retrieved_at", &preset.retrieved_at),
            ("display_name", &preset.display_name),
            ("scientific_name", &preset.scientific_name),
        ] {
            if value.trim().is_empty() {
                return Err(CatalogueError::MissingProvenanceMetadata {
                    preset_id: preset.preset_id.clone(),
                    field,
                });
            }
        }
        for preference in &preset.measurements {
            let kind = preference.kind.as_str().to_owned();
            if !preference.kind.is_known() {
                return Err(CatalogueError::UnknownKind {
                    preset_id: preset.preset_id.clone(),
                    kind,
                });
            }
            if preference.values().iter().any(|v| !v.value().is_finite()) {
                return Err(CatalogueError::NotFinite {
                    preset_id: preset.preset_id.clone(),
                    kind,
                });
            }
            let inverted = |detail: &'static str| CatalogueError::InvertedRange {
                preset_id: preset.preset_id.clone(),
                kind: kind.clone(),
                detail,
            };
            if preference.target_low.value() > preference.target_high.value() {
                return Err(inverted("target_low above target_high"));
            }
            let pairs: [(Option<f64>, Option<f64>, &'static str); 3] = [
                (
                    preference
                        .warning_low
                        .as_ref()
                        .map(super::ProvenancedValue::value),
                    preference
                        .warning_high
                        .as_ref()
                        .map(super::ProvenancedValue::value),
                    "warning_low above warning_high",
                ),
                (
                    preference
                        .critical_low
                        .as_ref()
                        .map(super::ProvenancedValue::value),
                    preference
                        .critical_high
                        .as_ref()
                        .map(super::ProvenancedValue::value),
                    "critical_low above critical_high",
                ),
                (
                    preference
                        .critical_low
                        .as_ref()
                        .map(super::ProvenancedValue::value),
                    preference
                        .warning_low
                        .as_ref()
                        .map(super::ProvenancedValue::value),
                    "critical_low above warning_low",
                ),
            ];
            for (low, high, detail) in pairs {
                if let (Some(low), Some(high)) = (low, high)
                    && low > high
                {
                    return Err(inverted(detail));
                }
            }
            if let (Some(warning), Some(critical)) = (
                preference
                    .warning_high
                    .as_ref()
                    .map(super::ProvenancedValue::value),
                preference
                    .critical_high
                    .as_ref()
                    .map(super::ProvenancedValue::value),
            ) && warning > critical
            {
                return Err(inverted("warning_high above critical_high"));
            }
        }
    }
    Ok(())
}

/// The embedded catalogue, parsed once.
///
/// # Panics
///
/// Panics if the compiled-in catalogue is malformed. That is a build defect
/// rather than a runtime condition: the data is a `const` in this binary, and
/// [`tests::the_embedded_catalogue_is_valid`] fails the build's own test suite
/// before anything ships.
#[must_use]
pub fn catalogue() -> &'static Catalogue {
    static PARSED: OnceLock<Catalogue> = OnceLock::new();
    PARSED.get_or_init(|| match parse(CATALOGUE_JSON) {
        Ok(catalogue) => catalogue,
        Err(e) => panic!("the embedded preset catalogue is malformed: {e:?}"),
    })
}

/// The raw embedded JSON, for the assertions that must read the data itself.
#[must_use]
pub const fn raw_json() -> &'static str {
    CATALOGUE_JSON
}

/// Every entry, in catalogue order.
#[must_use]
pub fn list() -> &'static [PlantPreset] {
    &catalogue().presets
}

/// One entry by id.
#[must_use]
pub fn get(preset_id: &str) -> Option<&'static PlantPreset> {
    catalogue()
        .presets
        .iter()
        .find(|p| p.preset_id == preset_id)
}

/// Entries matching a free-text query by display name, botanical name, id, or
/// synonym. An empty query lists everything.
#[must_use]
pub fn search(query: &str) -> Vec<&'static PlantPreset> {
    catalogue()
        .presets
        .iter()
        .filter(|p| p.matches(query))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    /// Recursively collects every object key in the catalogue.
    fn keys(value: &Value, into: &mut Vec<String>) {
        match value {
            Value::Object(map) => {
                for (k, v) in map {
                    into.push(k.clone());
                    keys(v, into);
                }
            }
            Value::Array(items) => {
                for item in items {
                    keys(item, into);
                }
            }
            _ => {}
        }
    }

    fn strings(value: &Value, into: &mut Vec<String>) {
        match value {
            Value::Object(map) => map.values().for_each(|v| strings(v, into)),
            Value::Array(items) => items.iter().for_each(|v| strings(v, into)),
            Value::String(s) => into.push(s.clone()),
            _ => {}
        }
    }

    #[test]
    fn the_embedded_catalogue_is_valid() {
        let catalogue = catalogue();
        assert_eq!(catalogue.catalogue_version, 1);
        assert!(
            catalogue.presets.len() >= 20,
            "the first catalogue is twenty genuinely curated entries, not four hundred scraped ones"
        );
        assert_eq!(validate(catalogue), Ok(()));
    }

    /// It needs no network and no database: the data is a compile-time constant.
    #[test]
    fn the_catalogue_is_queryable_with_no_network_and_no_database() {
        assert!(!raw_json().is_empty());
        assert_eq!(parse(raw_json()).unwrap().presets.len(), list().len());
    }

    #[test]
    fn every_entry_carries_its_provenance_metadata() {
        for preset in list() {
            for (field, value) in [
                ("source", &preset.source),
                ("source_ref", &preset.source_ref),
                ("license", &preset.license),
                ("retrieved_at", &preset.retrieved_at),
            ] {
                assert!(
                    !value.trim().is_empty(),
                    "{}: {field} is empty",
                    preset.preset_id
                );
            }
            assert!(
                preset.retrieved_at.len() >= 10,
                "{}: retrieved_at must be an ISO 8601 date",
                preset.preset_id
            );
        }
    }

    /// Every preference value is labelled, and both labels are actually used —
    /// a catalogue where everything was one kind of claim would make the
    /// distinction untested.
    #[test]
    fn every_value_is_labelled_and_both_labels_are_used() {
        let mut facts = 0;
        let mut defaults = 0;
        for preset in list() {
            for preference in &preset.measurements {
                for value in preference.values() {
                    if value.is_source_fact() {
                        facts += 1;
                    } else {
                        defaults += 1;
                    }
                }
            }
        }
        assert!(facts > 0, "no value is attributed to a cited source");
        assert!(defaults > 0, "no value is labelled as Rhizo-derived");
    }

    /// **No catalogue field is an interval, a frequency, or a schedule.** A
    /// timer would be a second actuation authority no lockout could contradict.
    #[test]
    fn no_catalogue_field_is_a_schedule() {
        let value: Value = serde_json::from_str(raw_json()).unwrap();
        let mut found = Vec::new();
        keys(&value, &mut found);
        for key in &found {
            let lower = key.to_lowercase();
            for forbidden in [
                "interval",
                "frequency",
                "schedule",
                "every",
                "period",
                "cadence",
                "timer",
                "days",
                "weekly",
                "daily",
                "cron",
            ] {
                assert!(
                    !lower.contains(forbidden),
                    "catalogue field {key:?} looks like a schedule ({forbidden})"
                );
            }
        }
        assert!(found.contains(&"cooldown_class".to_owned()));
    }

    /// **No catalogue field names a device, sensor, point, or capability.** A
    /// preset describes a plant; a binding describes an installation.
    #[test]
    fn no_catalogue_field_names_physical_hardware() {
        let value: Value = serde_json::from_str(raw_json()).unwrap();
        let mut found = Vec::new();
        keys(&value, &mut found);
        for key in &found {
            let lower = key.to_lowercase();
            for forbidden in [
                "device",
                "sensor",
                "point",
                "capability",
                "actuator",
                "probe",
                "pump",
                "channel",
                "gpio",
            ] {
                assert!(
                    !lower.contains(forbidden),
                    "catalogue field {key:?} names hardware ({forbidden})"
                );
            }
        }
        // A value cannot smuggle one in either.
        let mut literals = Vec::new();
        strings(&value, &mut literals);
        for literal in &literals {
            for forbidden in ["device_id", "sensor_id", "actuator_id", "capability_id"] {
                assert!(
                    !literal.contains(forbidden),
                    "catalogue value {literal:?} names {forbidden}"
                );
            }
        }
    }

    #[test]
    fn search_finds_a_species_by_name_and_by_synonym() {
        assert!(
            search("monstera")
                .iter()
                .any(|p| p.preset_id == "monstera-deliciosa")
        );
        assert!(
            search("swiss cheese")
                .iter()
                .any(|p| p.preset_id == "monstera-deliciosa"),
            "search must find a species by a common name nobody spells botanically"
        );
        assert!(
            search("Sansevieria")
                .iter()
                .any(|p| p.preset_id == "sansevieria-trifasciata"),
            "search is case-insensitive"
        );
        assert!(
            search("triffid").is_empty(),
            "an unlisted species is a miss, not a guess"
        );
        assert_eq!(search("").len(), list().len());
        assert!(get("monstera-deliciosa").is_some());
        assert_eq!(get("nothing-like-this"), None);
    }

    /// A malformed entry fails validation rather than shipping.
    #[test]
    fn a_malformed_entry_fails_validation() {
        let mut broken = catalogue().clone();
        let first = broken.presets[0].clone();
        broken.presets.push(first);
        assert!(matches!(
            validate(&broken),
            Err(CatalogueError::DuplicateId(_))
        ));

        let mut broken = catalogue().clone();
        broken.presets[0].license = String::new();
        assert!(matches!(
            validate(&broken),
            Err(CatalogueError::MissingProvenanceMetadata { .. })
        ));

        let mut broken = catalogue().clone();
        let preference = &mut broken.presets[0].measurements[0];
        preference.target_low = super::super::ProvenancedValue::RhizoDefault {
            value: 99.0,
            derived_from: None,
        };
        assert!(matches!(
            validate(&broken),
            Err(CatalogueError::InvertedRange { .. })
        ));

        assert!(matches!(parse("{"), Err(CatalogueError::Malformed(_))));
    }

    /// Materialised rows must satisfy exactly the validation a hand-configured
    /// row satisfies, or a preset would be a way to write a policy an operator
    /// could not.
    #[test]
    fn every_entry_materialises_into_a_valid_measurement_policy() {
        use crate::measurement_policy::MeasurementPolicyRules as _;
        for preset in list() {
            for preference in &preset.measurements {
                let policy = preference.to_policy(900_000, None);
                assert_eq!(
                    policy.validate(),
                    Ok(()),
                    "{} / {}",
                    preset.preset_id,
                    preference.kind.as_str()
                );
            }
        }
    }
}
