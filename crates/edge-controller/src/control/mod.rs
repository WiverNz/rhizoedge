//! The periodic control plane.
//!
//! M5 evaluated and recorded. **M6 is where this loop can move water**, so it
//! now holds a [`transport::Transport`] — and every path to that transport goes
//! through [`command::Commander`], which persists before it publishes and never
//! mints a second `command_id` for the same dose.
pub mod clock_step;
pub mod command;
pub mod config;
pub mod inputs;
pub mod intents;
pub mod irrigation;
pub mod reconcile;
pub mod threshold;
pub mod tick;
pub mod transport;

use chrono::{DateTime, Utc};

/// Applies a detected wall-clock step (M6-015, F-060-51/52).
///
/// A forward step beyond ten minutes locks **every** plant `Uncertain` and holds
/// the lockout for one cooldown, because the rolling window is the mechanism it
/// would otherwise defeat: older `watering_events` drop out early and a plant
/// that had spent its allowance is handed a fresh one. A backward step is
/// recorded and nothing else — it makes the window include more history, so the
/// cap becomes more conservative on its own.
pub async fn clock_step_response(
    commander: &command::Commander,
    step: clock_step::Step,
    now: DateTime<Utc>,
) -> Result<usize, crate::error::EdgeError> {
    let db = commander.db();
    let now_ms = now.timestamp_millis();
    commander
        .metrics_ref()
        .clock_steps
        .with_label_values(&[step.direction.as_str()])
        .inc();
    let detail = serde_json::json!({
        "direction": step.direction.as_str(),
        "magnitude_ms": step.magnitude.num_milliseconds(),
    });
    if !step.locks_out() {
        tracing::warn!(
            direction = step.direction.as_str(),
            magnitude_ms = step.magnitude.num_milliseconds(),
            "the edge wall clock moved; the rolling window becomes more conservative and nothing is locked out"
        );
        rhizo_storage::repo::plant::record_plant_event(
            db,
            None,
            &format!("edge:clock_step:{now_ms}"),
            "clock_step",
            "info",
            Some(&detail),
            now_ms,
        )
        .await?;
        return Ok(0);
    }
    tracing::error!(
        direction = step.direction.as_str(),
        magnitude_ms = step.magnitude.num_milliseconds(),
        "the edge wall clock stepped forward; every plant is locked out for one cooldown"
    );
    rhizo_storage::repo::plant::record_plant_event(
        db,
        None,
        &format!("edge:clock_step:{now_ms}"),
        "clock_step",
        "critical",
        Some(&detail),
        now_ms,
    )
    .await?;
    let mut locked = 0;
    for row in rhizo_storage::repo::plant::list(db, None, 500).await? {
        let Some(loaded) = crate::plant::load(db, &row.plant_id).await? else {
            continue;
        };
        let hold = now_ms + (loaded.profile.cooldown_hours * 3_600_000.0).max(0.0) as i64;
        // A hold of its own, beside any latch the plant already has: it can
        // neither replace a leak nor, when it expires, take one with it. An
        // operator clearing a latch inside the cooldown leaves this hold in
        // force (F-060-51).
        rhizo_storage::repo::lockout::hold(
            db,
            &row.plant_id,
            &crate::plant::lockout_name(rhizo_domain::state::LockoutReason::Uncertain),
            hold,
            now_ms,
        )
        .await?;
        commander
            .metrics_ref()
            .lockouts
            .with_label_values(&["uncertain"])
            .inc();
        locked += 1;
    }
    Ok(locked)
}
