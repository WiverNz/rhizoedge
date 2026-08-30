//! The connected control loop: evaluate, persist, act (M6-006, M6-014).
//!
//! One pass per plant per tick:
//!
//! ```text
//! load state from SQLite -> gather inputs -> evaluate() -> persist -> maybe publish
//! ```
//!
//! State is read from storage **every** tick and never from memory alone
//! (F-060-13), and every transition is persisted with its side effect
//! (F-060-14). Persisting each transition is what makes "what did the system
//! think, and when" reconstructable months later — the question actually asked
//! when a plant dies.
//!
//! # Nothing here decides anything
//!
//! Every decision comes from `rhizo_domain::irrigation::evaluate`, which runs
//! the safety gate as its first statement. This module reads, writes, publishes,
//! and counts. If a rule appears here that is not in the domain, the property
//! tests stop proving anything about the running system.

use chrono::{DateTime, Duration, Utc};
use rhizo_domain::irrigation::types::{EvaluationMode, IrrigationDecision, state_name};
use rhizo_domain::irrigation::{evaluate, next_state};
use rhizo_domain::state::{IrrigationState, LockoutReason};
use rhizo_storage::EdgeDb;
use rhizo_storage::repo::command as command_repo;

use crate::error::EdgeError;
use crate::plant::{self, Loaded};

use super::command::{Commander, DoseRequest, Issued};
use super::inputs::{self, Gathered};

/// What one pass concluded, for logs and tests.
#[derive(Clone, Debug, PartialEq)]
pub struct Pass {
    /// The decision the pure machine reached.
    pub decision: IrrigationDecision,
    /// Where the plant is now.
    pub state: IrrigationState,
    /// The command that was published, if one was.
    pub command_id: Option<String>,
    /// The lockout now in force.
    pub lockout: Option<LockoutReason>,
}

/// Evaluates one plant and applies the result.
///
/// `mode` is [`EvaluationMode::Automatic`] from the control loop; the REST
/// watering endpoint calls the same function with an operator mode, so **every**
/// actuation path runs the same gate through the same entry point (F-060-01).
pub async fn run_pass(
    commander: &Commander,
    loaded: &Loaded,
    dry_duration: Duration,
    mode: EvaluationMode,
    now: DateTime<Utc>,
) -> Result<Pass, EdgeError> {
    let gathered = inputs::gather(commander.db(), loaded, dry_duration, now).await?;
    let decision = evaluate(gathered.inputs(now, mode));
    apply(commander, loaded, &gathered, decision, mode, now).await
}

/// Evaluates without acting, for the read-only paths and for tests.
pub async fn preview(
    db: &EdgeDb,
    loaded: &Loaded,
    dry_duration: Duration,
    mode: EvaluationMode,
    now: DateTime<Utc>,
) -> Result<(Gathered, IrrigationDecision), EdgeError> {
    let gathered = inputs::gather(db, loaded, dry_duration, now).await?;
    let decision = evaluate(gathered.inputs(now, mode));
    Ok((gathered, decision))
}

/// Persists a decision and, for [`IrrigationDecision::IssueDose`], publishes it.
pub async fn apply(
    commander: &Commander,
    loaded: &Loaded,
    gathered: &Gathered,
    decision: IrrigationDecision,
    mode: EvaluationMode,
    now: DateTime<Utc>,
) -> Result<Pass, EdgeError> {
    let db = commander.db();
    let now_ms = now.timestamp_millis();
    let plant_id = loaded.plant.plant_id.as_str();
    let inputs = gathered.inputs(now, mode);
    let target = next_state(&inputs, &decision);
    let mut command_id = None;
    let mut lockout = None;

    let mut row = gathered.state_row(now_ms);
    row.state = state_name(target).to_owned();
    row.state_since = if gathered.state == target {
        // A tick that changes nothing is not a transition, and re-stamping it
        // would make "how long has this plant been locked out?" unanswerable.
        command_repo::irrigation_state(db, plant_id)
            .await?
            .map_or(now_ms, |stored| stored.state_since)
    } else {
        now_ms
    };

    match &decision {
        IrrigationDecision::IssueDose { ml, .. } => {
            let Some(device) = gathered.actuator_device.clone() else {
                // Unreachable: the gate refuses a plant with no actuator. Kept
                // as a refusal rather than an `unwrap`, because a panic in the
                // control loop stops every other plant too.
                return Err(EdgeError::Decode(
                    "the machine asked for a dose on a plant with no actuator".to_owned(),
                ));
            };
            // The baseline recovery and no-delivery detection compare against is
            // taken at the *start* of a cycle, not before every dose: two doses
            // that each fail to move the probe must look like two, not like two
            // fresh starts (F-060-33).
            if gathered.doses_this_cycle == 0 {
                row.pre_dose_vwc = gathered.soil.and_then(|s| s.moisture_vwc);
                row.pre_dose_grams = gathered.weight.and_then(|s| s.grams);
                row.cycle_started_at = Some(now_ms);
            }
            let request = DoseRequest {
                plant_id: plant_id.to_owned(),
                device_id: device,
                requested_ml: *ml,
                mode,
                ttl: gathered.automation.command_ttl,
                next_state: row.clone(),
            };
            match commander.issue_water(&request).await? {
                Issued::Published { command_id: id, .. } => command_id = Some(id),
                Issued::PublishFailed { command_id: id } => {
                    // The command row is already `failed` and the plant already
                    // back in `Recheck`, both committed by `issue_water`.
                    command_id = Some(id);
                }
            }
        }
        IrrigationDecision::Lock { reason } => {
            lockout = Some(*reason);
            let existing = gathered.active_lockout;
            if existing != Some(*reason) {
                command_repo::set_lockout(
                    db,
                    plant_id,
                    Some(&plant::lockout_name(*reason)),
                    Some(now_ms),
                    gathered.lockout_held_until.map(|v| v.timestamp_millis()),
                    None,
                    now_ms,
                )
                .await?;
                commander
                    .metrics_ref()
                    .lockouts
                    .with_label_values(&[&plant::lockout_name(*reason)])
                    .inc();
                tracing::info!(
                    plant_id = %plant_id,
                    reason = %plant::lockout_name(*reason),
                    clearable = rhizo_domain::irrigation::is_auto_clearable(*reason),
                    "watering locked out"
                );
            }
            command_repo::put_irrigation_state(db, plant_id, &row, now_ms).await?;
        }
        IrrigationDecision::CycleComplete => {
            row.last_cycle_completed_at = Some(now_ms);
            row.doses_this_cycle = 0;
            row.cycle_started_at = None;
            row.wait_until = None;
            row.pre_dose_vwc = None;
            row.pre_dose_grams = None;
            row.active_command_id = None;
            command_repo::put_irrigation_state(db, plant_id, &row, now_ms).await?;
            tracing::info!(plant_id = %plant_id, "watering cycle complete");
        }
        IrrigationDecision::Wait { until } => {
            if gathered.state == IrrigationState::WaitForAbsorption {
                row.wait_until = Some(until.timestamp_millis());
            }
            command_repo::put_irrigation_state(db, plant_id, &row, now_ms).await?;
        }
        IrrigationDecision::Idle | IrrigationDecision::Recommend { .. } => {
            if target != IrrigationState::WaitForAbsorption {
                row.wait_until = None;
            }
            command_repo::put_irrigation_state(db, plant_id, &row, now_ms).await?;
        }
    }

    // An auto-clearing lockout is lifted the moment its condition resolves. The
    // gate has already decided that by not returning it.
    if !matches!(decision, IrrigationDecision::Lock { .. })
        && let Some(previous) = gathered.active_lockout
        && rhizo_domain::irrigation::is_auto_clearable(previous)
        && gathered.lockout_held_until.is_none_or(|until| now >= until)
    {
        command_repo::set_lockout(db, plant_id, None, None, None, Some("auto"), now_ms).await?;
        tracing::info!(
            plant_id = %plant_id,
            reason = %plant::lockout_name(previous),
            "lockout cleared automatically"
        );
    }

    if gathered.state != target {
        commander
            .metrics_ref()
            .irrigation_transitions
            .with_label_values(&[state_name(gathered.state), state_name(target)])
            .inc();
        plant::record_irrigation_transition(db, plant_id, gathered.state, target, now_ms).await?;
        tracing::info!(
            plant_id = %plant_id,
            from = state_name(gathered.state),
            to = state_name(target),
            decision = decision.as_str(),
            "irrigation state changed"
        );
    }

    Ok(Pass {
        decision,
        state: target,
        command_id,
        lockout,
    })
}
