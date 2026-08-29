//! The recommendation endpoint (M5-012), `http-api-boundaries.md` §2.5.
//!
//! Reads the last persisted answer, or evaluates on demand when the tick has not
//! reached this plant yet. Evaluating here publishes nothing: the whole path is
//! the same one the tick uses, and it holds no MQTT client.
#![allow(missing_docs)]
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use rhizo_storage::repo::plant as plant_repo;

use super::ApiState;
use super::support::{error, storage_error, timestamp};

pub async fn get(State(state): State<ApiState>, Path(id): Path<String>) -> Response {
    match super::plants::exists(&state, &id).await {
        Ok(true) => {}
        Ok(false) => return error(StatusCode::NOT_FOUND, "plant_not_found", "unknown plant"),
        Err(_) => return storage_error(),
    }
    match plant_repo::latest_recommendation(&state.db, &id).await {
        Ok(Some(row)) => {
            let reasons: serde_json::Value =
                serde_json::from_str(&row.reasons_json).unwrap_or_else(|_| serde_json::json!([]));
            Json(serde_json::json!({
                "recommendation": row.decision,
                "recommended_ml": row.recommended_ml,
                "confidence": row.confidence,
                "reasons": reasons,
                "blocked_by": row.blocked_by,
                "evaluated_at": timestamp(row.evaluated_at),
            }))
            .into_response()
        }
        // No row yet means the tick has not reached this plant. Evaluate once so
        // a freshly created plant answers something true rather than 404.
        Ok(None) => {
            match crate::control::tick::evaluate_plant(
                &state.db,
                &id,
                state.clock.as_ref(),
                &state.metrics,
            )
            .await
            {
                Ok(Some(recommendation)) => Json(crate::control::tick::recommendation_json(
                    &recommendation,
                    Some(state.clock.now().timestamp_millis()),
                ))
                .into_response(),
                Ok(None) => error(StatusCode::NOT_FOUND, "plant_not_found", "unknown plant"),
                Err(_) => storage_error(),
            }
        }
        Err(_) => storage_error(),
    }
}
