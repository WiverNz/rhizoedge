//! Persisting the operator-facing plant state (M5-010).
//!
//! Transitions are persisted; steady state is not. A 30-second tick that reaches
//! the same conclusion would otherwise write thousands of rows a day recording
//! that nothing changed (ADR-010).
use chrono::{DateTime, Utc};
use rhizo_domain::plant_state;
use rhizo_domain::recommend::Recommendation;
use rhizo_domain::state::PlantState;
use rhizo_storage::EdgeDb;
use rhizo_storage::repo::plant as plant_repo;

/// The wire name of a plant state.
#[must_use]
pub fn state_name(state: PlantState) -> String {
    serde_json::to_value(state)
        .ok()
        .and_then(|v| v.as_str().map(ToOwned::to_owned))
        .unwrap_or_else(|| "unknown".to_owned())
}

/// Decodes a stored state name. An unrecognised name reads as `Unknown` rather
/// than as any particular health.
#[must_use]
pub fn state_from_str(name: &str) -> PlantState {
    serde_json::from_value(serde_json::Value::String(name.to_owned()))
        .unwrap_or(PlantState::Unknown)
}

/// Derives the state from a recommendation and persists a transition if there
/// is one. Returns the state now in force and the transition, when one occurred.
pub async fn apply(
    db: &EdgeDb,
    plant_id: &str,
    recommendation: &Recommendation,
    now: DateTime<Utc>,
) -> Result<(PlantState, Option<plant_state::Transition>), rhizo_storage::StorageError> {
    let derived = plant_state::derive(recommendation);
    let previous = plant_repo::plant_state(db, plant_id)
        .await?
        .map(|name| state_from_str(&name));
    let Some(transition) = plant_state::transition(previous, derived) else {
        return Ok((derived, None));
    };
    plant_repo::record_state_transition(
        db,
        plant_id,
        transition.from.map(state_name).as_deref(),
        &state_name(transition.to),
        now.timestamp_millis(),
    )
    .await?;
    Ok((derived, Some(transition)))
}
