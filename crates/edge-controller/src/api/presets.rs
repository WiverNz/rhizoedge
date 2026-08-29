//! Preset catalogue endpoints (M5-017), PRD 050 §Preset endpoints.
//!
//! Read-only. The catalogue is compiled into the binary, so these handlers
//! perform no I/O at all — which is what makes creating a plant work on a site
//! that has been off the internet for a week.
#![allow(missing_docs)]
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use rhizo_domain::preset::{MeasurementPreference, PlantPreset, ProvenancedValue, catalogue};
use serde::Deserialize;

use super::ApiState;
use super::support::error;

/// Renders a value with its provenance intact.
///
/// The UI needs the difference (M12-017), and it cannot show it if the API has
/// already flattened a Rhizo interpretation into something that looks like a
/// measured fact.
fn value_json(value: &ProvenancedValue) -> serde_json::Value {
    match value {
        ProvenancedValue::SourceFact {
            value,
            units,
            source_ref,
        } => serde_json::json!({
            "value": value,
            "provenance": "source_fact",
            "units": units,
            "source_ref": source_ref,
        }),
        ProvenancedValue::RhizoDefault {
            value,
            derived_from,
        } => serde_json::json!({
            "value": value,
            "provenance": "rhizo_default",
            "derived_from": derived_from,
        }),
    }
}

fn preference_json(preference: &MeasurementPreference) -> serde_json::Value {
    serde_json::json!({
        "kind": preference.kind.as_str(),
        "target_low": value_json(&preference.target_low),
        "target_high": value_json(&preference.target_high),
        "warning_low": preference.warning_low.as_ref().map(value_json),
        "warning_high": preference.warning_high.as_ref().map(value_json),
        "critical_low": preference.critical_low.as_ref().map(value_json),
        "critical_high": preference.critical_high.as_ref().map(value_json),
    })
}

fn summary_json(preset: &PlantPreset) -> serde_json::Value {
    serde_json::json!({
        "preset_id": preset.preset_id,
        "display_name": preset.display_name,
        "scientific_name": preset.scientific_name,
        "synonyms": preset.synonyms,
        "dose_class": preset.dose_class,
        "cooldown_class": preset.cooldown_class,
        "kinds": preset.measurements.iter().map(|m| m.kind.as_str()).collect::<Vec<_>>(),
    })
}

fn detail_json(preset: &PlantPreset) -> serde_json::Value {
    let mut value = summary_json(preset);
    if let Some(object) = value.as_object_mut() {
        object.insert(
            "provenance".to_owned(),
            serde_json::json!({
                "source": preset.source,
                "source_ref": preset.source_ref,
                "license": preset.license,
                "retrieved_at": preset.retrieved_at,
            }),
        );
        object.insert(
            "measurements".to_owned(),
            serde_json::Value::Array(preset.measurements.iter().map(preference_json).collect()),
        );
    }
    value
}

#[derive(Deserialize)]
pub struct SearchQuery {
    q: Option<String>,
}

pub async fn list(State(_): State<ApiState>, Query(query): Query<SearchQuery>) -> Response {
    let found = catalogue::search(query.q.as_deref().unwrap_or(""));
    Json(serde_json::json!({
        "catalogue_version": catalogue::catalogue().catalogue_version,
        "presets": found.iter().map(|p| summary_json(p)).collect::<Vec<_>>(),
    }))
    .into_response()
}

pub async fn get(State(_): State<ApiState>, Path(id): Path<String>) -> Response {
    match catalogue::get(&id) {
        Some(preset) => Json(detail_json(preset)).into_response(),
        None => error(
            StatusCode::NOT_FOUND,
            "preset_not_found",
            "no such entry in the embedded catalogue",
        ),
    }
}
