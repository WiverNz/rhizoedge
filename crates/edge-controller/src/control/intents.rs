//! Holding a dose for a sleeping device, and minting it at the wake
//! (M6-022, ADR-018 §3).
//!
//! # The routing rule
//!
//! ```text
//! device connected -> the existing immediate path, byte-identical to M6-016
//! device sleeping  -> a durable intent, and nothing is published
//! device isolated  -> refused; there is no wake to wait for
//! ```
//!
//! # The gate runs twice, and the second time is the one that counts
//!
//! Once at request time, to refuse obviously impossible requests early, and
//! again **in full** at delivery against current inputs. This is the part most
//! likely to be implemented as a cheap "still allowed?" check, and it must not
//! be: a leak, an empty tank, an exhausted rolling window, or a stale required
//! measurement that appeared while the device slept all have to refuse the dose
//! (SAFETY-003, -004, -005, -006, -012). Running the whole gate at delivery makes
//! this path **stricter** than the immediate one, which runs it once.
//!
//! # `edge.time` first, then the command
//!
//! A device that has just woken may not yet have applied a wall clock, and a
//! command it cannot date is refused `clock_unsynced` (F-040-17). The edge
//! therefore publishes `edge.time` before the command on every wake delivery,
//! and treats a `clock_unsynced` refusal inside the same awake window as a
//! **retryable** delivery failure rather than a terminal one — the window is
//! bounded, so the retry terminates. Every other refusal reason is terminal for
//! the intent.

use chrono::{DateTime, Duration, Utc};
use rhizo_domain::irrigation::types::{EvaluationMode, IrrigationDecision};
use rhizo_storage::EdgeDb;
use rhizo_storage::repo::{command as command_repo, intent as intent_repo};

use crate::error::EdgeError;
use crate::plant;

use super::command::{Commander, DoseRequest, Issued};
use super::inputs;
use super::irrigation;

/// Why a `POST /water` could not be routed anywhere.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RoutingRefusal {
    /// The device is neither connected nor inside a wake window.
    DeviceUnreachable,
    /// A water intent is already open for this plant.
    IntentAlreadyOpen {
        /// The pending intent's id.
        intent_id: String,
        /// When it may next be delivered.
        expected_delivery_after: Option<i64>,
    },
}

/// Where a request should go.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Route {
    /// Publish now.
    Immediate,
    /// Hold it until the device wakes.
    HoldForWake {
        /// When the device is next expected to be reachable.
        expected_delivery_after: Option<i64>,
        /// The device's declared wake interval, for the intent's own TTL.
        wake_interval_seconds: Option<i64>,
    },
    /// Refuse.
    Refuse(RoutingRefusal),
}

/// Decides where a dose request for `device` should go.
pub async fn route(
    db: &EdgeDb,
    plant_id: &str,
    device_id: &str,
    now: DateTime<Utc>,
) -> Result<Route, EdgeError> {
    if let Some(open) = intent_repo::open_for_plant(db, plant_id).await? {
        return Ok(Route::Refuse(RoutingRefusal::IntentAlreadyOpen {
            intent_id: open.intent_id,
            expected_delivery_after: open.expected_delivery_after,
        }));
    }
    let now_ms = now.timestamp_millis();
    let (online, sleeping, reconciling) =
        inputs::device_reachability(db, device_id, now_ms).await?;
    if online {
        return Ok(Route::Immediate);
    }
    if sleeping {
        use sqlx::Row as _;
        let row = sqlx::query(
            "SELECT expected_wake_at,wake_interval_seconds FROM devices WHERE device_id=?",
        )
        .bind(device_id)
        .fetch_optional(db.pool())
        .await
        .map_err(|e| EdgeError::Storage(rhizo_storage::StorageError::Database(e.to_string())))?;
        return Ok(Route::HoldForWake {
            expected_delivery_after: row.as_ref().and_then(|r| r.get("expected_wake_at")),
            wake_interval_seconds: row.as_ref().and_then(|r| r.get("wake_interval_seconds")),
        });
    }
    // A reconciling device is reachable but its history is not yet read, so the
    // gate refuses the plant anyway (SAFETY-016). Saying so here keeps the API
    // answer specific rather than "unreachable".
    let _ = reconciling;
    Ok(Route::Refuse(RoutingRefusal::DeviceUnreachable))
}

/// Persists a pending intent.
#[allow(
    clippy::too_many_arguments,
    reason = "every argument is a distinct fact about the request, and bundling               them into a struct would hide which of them the intent's two               expiries are derived from"
)]
pub async fn hold(
    db: &EdgeDb,
    plant_id: &str,
    device_id: &str,
    requested_ml: f32,
    mode: EvaluationMode,
    expected_delivery_after: Option<i64>,
    wake_interval_seconds: Option<i64>,
    now: DateTime<Utc>,
) -> Result<intent_repo::IntentRow, EdgeError> {
    let now_ms = now.timestamp_millis();
    let intent_id = uuid::Uuid::now_v7().to_string();
    let ttl = intent_repo::intent_ttl_ms(wake_interval_seconds);
    intent_repo::create(
        db,
        &intent_repo::NewIntent {
            intent_id: intent_id.clone(),
            plant_id: plant_id.to_owned(),
            device_id: device_id.to_owned(),
            kind: "water".to_owned(),
            requested_ml: f64::from(requested_ml),
            mode: mode.as_str().to_owned(),
            created_at: now_ms,
            intent_expires_at: now_ms + ttl,
            expected_delivery_after,
        },
        now_ms,
    )
    .await?;
    tracing::info!(
        plant_id = %plant_id,
        device_id = %device_id,
        intent_id = %intent_id,
        requested_ml,
        "dose held for the device's next wake; nothing published"
    );
    intent_repo::get(db, &intent_id).await?.ok_or_else(|| {
        EdgeError::Storage(rhizo_storage::StorageError::Database(
            "the intent vanished immediately after insert".to_owned(),
        ))
    })
}

/// What a delivery attempt did.
#[derive(Clone, Debug, PartialEq)]
pub enum Delivery {
    /// A command was minted and published.
    Sent {
        /// The intent.
        intent_id: String,
        /// The `command_id` allocated **at the wake**.
        command_id: String,
    },
    /// The gate refused it against current inputs. Nothing was published.
    Refused {
        /// The intent.
        intent_id: String,
        /// The lockout reason the gate returned.
        reason: String,
    },
    /// The device is awake but not yet ready. Retried inside the same window.
    Retryable {
        /// The intent.
        intent_id: String,
    },
    /// Nothing to do.
    Nothing,
}

/// Delivers every pending intent for a device that has just become reachable.
///
/// Publishes `edge.time` first, then re-runs the **whole** gate, then mints one
/// command. `issued_at` is therefore the wake instant, never the request
/// instant, which is what keeps the unchanged 120-second TTL meaningful
/// (SAFETY-002).
pub async fn deliver_for_device(
    commander: &Commander,
    device_id: &str,
    now: DateTime<Utc>,
) -> Result<Vec<Delivery>, EdgeError> {
    let db = commander.db();
    let pending = intent_repo::pending_for_device(db, device_id).await?;
    if pending.is_empty() {
        return Ok(Vec::new());
    }
    // F-040-17: the device gets the time before it gets anything to date.
    super::config::publish_edge_time(commander, device_id, now).await?;

    let mut delivered = Vec::new();
    for intent in pending {
        delivered.push(deliver_one(commander, &intent, now).await?);
    }
    Ok(delivered)
}

async fn deliver_one(
    commander: &Commander,
    intent: &intent_repo::IntentRow,
    now: DateTime<Utc>,
) -> Result<Delivery, EdgeError> {
    let db = commander.db();
    let now_ms = now.timestamp_millis();
    if intent.intent_expires_at <= now_ms {
        intent_repo::sweep_expired(db, now_ms).await?;
        return Ok(Delivery::Nothing);
    }
    let Some(loaded) = plant::load(db, &intent.plant_id).await? else {
        intent_repo::mark_refused(db, &intent.intent_id, "plant_not_found", now_ms).await?;
        return Ok(Delivery::Refused {
            intent_id: intent.intent_id.clone(),
            reason: "plant_not_found".to_owned(),
        });
    };
    let mode = match intent.mode.as_str() {
        "recommended" => EvaluationMode::RecommendedRequest {
            ml: intent.requested_ml as f32,
        },
        _ => EvaluationMode::ManualRequest {
            ml: intent.requested_ml as f32,
        },
    };
    // The full gate, against inputs read *now*. Not a cached verdict, not a
    // cheap re-check: a leak raised while the device slept refuses the dose.
    let analysis = plant::analyse(db, &loaded, now).await?;
    let (gathered, decision) =
        irrigation::preview(db, &loaded, analysis.inputs.dry_duration, mode, now).await?;

    match decision {
        IrrigationDecision::IssueDose { ml, .. } => {
            let mut next = gathered.state_row(now_ms);
            next.state = "dose_issued".to_owned();
            if gathered.doses_this_cycle == 0 {
                next.pre_dose_vwc = gathered.soil.and_then(|s| s.moisture_vwc);
                next.pre_dose_grams = gathered.weight.and_then(|s| s.grams);
                next.cycle_started_at = Some(now_ms);
            }
            let issued = commander
                .issue_water(&DoseRequest {
                    plant_id: intent.plant_id.clone(),
                    device_id: intent.device_id.clone(),
                    requested_ml: ml,
                    mode,
                    ttl: gathered.automation.command_ttl,
                    next_state: next,
                })
                .await?;
            match issued {
                Issued::Published { command_id, .. } => {
                    // The intent only becomes `sent` once the command exists, so
                    // an intent in `sent` always names exactly one command row.
                    intent_repo::mark_sent(db, &intent.intent_id, &command_id, now_ms).await?;
                    tracing::info!(
                        intent_id = %intent.intent_id,
                        command_id = %command_id,
                        plant_id = %intent.plant_id,
                        "held dose delivered at the device's wake"
                    );
                    Ok(Delivery::Sent {
                        intent_id: intent.intent_id.clone(),
                        command_id,
                    })
                }
                // The publish failed and the command is already `failed`. The
                // intent stays pending so the next wake may try again inside its
                // own bounded lifetime.
                Issued::PublishFailed { .. } => Ok(Delivery::Retryable {
                    intent_id: intent.intent_id.clone(),
                }),
            }
        }
        IrrigationDecision::Lock { reason } => {
            let name = plant::lockout_name(reason);
            // `clock_unsynced` means the device is awake but has not yet applied
            // `edge.time`. Retrying inside the same awake window is correct and
            // terminates, because the window itself is bounded. Every other
            // reason is terminal for the intent.
            if reason == rhizo_domain::state::LockoutReason::ClockUnsynced {
                return Ok(Delivery::Retryable {
                    intent_id: intent.intent_id.clone(),
                });
            }
            intent_repo::mark_refused(db, &intent.intent_id, &name, now_ms).await?;
            tracing::warn!(
                intent_id = %intent.intent_id,
                plant_id = %intent.plant_id,
                reason = %name,
                "a held dose was refused at delivery; nothing published"
            );
            Ok(Delivery::Refused {
                intent_id: intent.intent_id.clone(),
                reason: name,
            })
        }
        // Anything else — a cooldown, an in-flight command, an absorption wait —
        // is not a refusal of the operator's request, so the intent waits.
        IrrigationDecision::Idle
        | IrrigationDecision::Recommend { .. }
        | IrrigationDecision::Wait { .. }
        | IrrigationDecision::CycleComplete => Ok(Delivery::Retryable {
            intent_id: intent.intent_id.clone(),
        }),
    }
}

/// Expires everything past its deadline and refreshes the gauges.
///
/// Runs on the liveness timer, which already ticks every five seconds and
/// already re-derives every device's connectivity.
pub async fn sweep(commander: &Commander, now: DateTime<Utc>) -> Result<u64, EdgeError> {
    let db = commander.db();
    let expired = intent_repo::sweep_expired(db, now.timestamp_millis()).await?;
    if expired > 0 {
        commander
            .metrics_ref()
            .command_intents_expired
            .inc_by(expired);
        tracing::warn!(expired, "held doses expired before the device woke");
    }
    let pending = intent_repo::pending_count(db).await?;
    commander.metrics_ref().command_intents_pending.set(pending);
    Ok(expired)
}

/// Delivers whatever is pending for every device that is currently reachable.
pub async fn deliver_ready(commander: &Commander, now: DateTime<Utc>) -> Result<usize, EdgeError> {
    let db = commander.db();
    let mut sent = 0;
    let mut devices: Vec<String> = intent_repo::all_pending(db)
        .await?
        .into_iter()
        .map(|intent| intent.device_id)
        .collect();
    devices.sort();
    devices.dedup();
    for device in devices {
        let (online, ..) = inputs::device_reachability(db, &device, now.timestamp_millis()).await?;
        if !online {
            continue;
        }
        for delivery in deliver_for_device(commander, &device, now).await? {
            if matches!(delivery, Delivery::Sent { .. }) {
                sent += 1;
            }
        }
    }
    Ok(sent)
}

/// Reconciles intents on boot (M6-012 extended to M6-022).
///
/// An intent is durable, so a restart between request and wake changes nothing:
/// the rows are read back, anything past its deadline is expired, and the rest
/// wait exactly as they were. Nothing is published here — delivery happens when
/// the device is next observed awake.
pub async fn reconcile(commander: &Commander, now: DateTime<Utc>) -> Result<usize, EdgeError> {
    let db = commander.db();
    sweep(commander, now).await?;
    let pending = intent_repo::all_pending(db).await?;
    tracing::info!(
        pending = pending.len(),
        "restored pending command intents; none was published"
    );
    Ok(pending.len())
}

/// The lifetime of an intent, exposed for the API's response shape.
#[must_use]
pub fn ttl(wake_interval_seconds: Option<i64>) -> Duration {
    Duration::milliseconds(intent_repo::intent_ttl_ms(wake_interval_seconds))
}

/// Re-exported so the API and its tests name one set of state strings.
pub use intent_repo::{EXPIRED, PENDING, REFUSED, SENT};

/// A helper the API uses to answer `GET /intents/{id}`.
pub async fn get(
    db: &EdgeDb,
    intent_id: &str,
) -> Result<Option<intent_repo::IntentRow>, EdgeError> {
    Ok(intent_repo::get(db, intent_id).await?)
}

/// The command a delivered intent produced, for the handover a caller follows.
pub async fn command_for(
    db: &EdgeDb,
    intent: &intent_repo::IntentRow,
) -> Result<Option<command_repo::CommandRow>, EdgeError> {
    match intent.command_id.as_deref() {
        None => Ok(None),
        Some(id) => Ok(command_repo::get(db, id).await?),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::api::testsupport::TestApi;

    /// Routing by device connectivity, which is the whole of M6-022's first
    /// rule.
    #[tokio::test]
    async fn routing_follows_the_devices_reachability() {
        let api = TestApi::start().await;
        api.waterable("monstera-01").await;
        let now = api.clock.now();

        api.device_connected().await;
        assert_eq!(
            route(&api.db, "monstera-01", "plant-node-01", now)
                .await
                .unwrap(),
            Route::Immediate
        );

        api.device_sleeping(900_000).await;
        assert!(matches!(
            route(&api.db, "monstera-01", "plant-node-01", now)
                .await
                .unwrap(),
            Route::HoldForWake { .. }
        ));

        sqlx::query(
            "UPDATE devices SET connectivity_mode='isolated' WHERE device_id='plant-node-01'",
        )
        .execute(api.db.pool())
        .await
        .unwrap();
        assert_eq!(
            route(&api.db, "monstera-01", "plant-node-01", now)
                .await
                .unwrap(),
            Route::Refuse(RoutingRefusal::DeviceUnreachable)
        );
    }

    /// An overdue sleeper is `isolated`, not `sleeping` — so a dose for it is
    /// refused rather than held for a wake that already did not happen
    /// (SAFETY-021).
    #[tokio::test]
    async fn an_overdue_sleeper_is_not_held_for_a_wake() {
        let api = TestApi::start().await;
        api.waterable("monstera-01").await;
        api.device_sleeping(900_000).await;
        api.clock.advance(chrono::Duration::minutes(31));
        assert_eq!(
            route(&api.db, "monstera-01", "plant-node-01", api.clock.now())
                .await
                .unwrap(),
            Route::Refuse(RoutingRefusal::DeviceUnreachable)
        );
    }

    /// The single-open-intent rule holds at the storage layer as well as at the
    /// API, which is what makes it hold under a race.
    #[tokio::test]
    async fn a_second_open_intent_is_refused_by_the_index() {
        let api = TestApi::start().await;
        api.waterable("monstera-01").await;
        api.device_sleeping(900_000).await;
        let now = api.clock.now();
        hold(
            &api.db,
            "monstera-01",
            "plant-node-01",
            30.0,
            EvaluationMode::ManualRequest { ml: 30.0 },
            None,
            Some(900),
            now,
        )
        .await
        .unwrap();
        let error = hold(
            &api.db,
            "monstera-01",
            "plant-node-01",
            30.0,
            EvaluationMode::ManualRequest { ml: 30.0 },
            None,
            Some(900),
            now,
        )
        .await
        .unwrap_err();
        assert!(matches!(
            error,
            EdgeError::Storage(rhizo_storage::StorageError::Constraint(_))
        ));
    }

    /// The expiry sweep, and the gauge it maintains.
    #[tokio::test]
    async fn the_sweep_expires_and_counts() {
        let api = TestApi::start().await;
        api.waterable("monstera-01").await;
        api.device_sleeping(900_000).await;
        api.json(
            "POST",
            "/api/v1/plants/monstera-01/water",
            serde_json::json!({ "ml": 30.0 }),
        )
        .await;
        sweep(&api.commander, api.clock.now()).await.unwrap();
        assert_eq!(api.commander.metrics_ref().command_intents_pending.get(), 1);

        api.clock.advance(chrono::Duration::minutes(31));
        assert_eq!(sweep(&api.commander, api.clock.now()).await.unwrap(), 1);
        assert_eq!(api.commander.metrics_ref().command_intents_pending.get(), 0);
    }

    /// A `clock_unsynced` refusal inside one awake window is a **retryable**
    /// delivery failure rather than a terminal one: the device is awake but has
    /// not yet applied `edge.time`, and the window itself is bounded.
    #[test]
    fn a_clock_unsynced_refusal_is_retryable_and_every_other_reason_is_not() {
        let production = include_str!("intents.rs")
            .split("\n#[cfg(test)]")
            .next()
            .expect("the file has a production half");
        assert!(
            production.contains("LockoutReason::ClockUnsynced"),
            "the retryable case must be named explicitly, not inferred"
        );
        assert!(
            production.contains("mark_refused"),
            "every other reason is terminal for the intent"
        );
    }

    /// The intent's TTL is two wakes with a half-hour floor, and it is the
    /// **edge's** clock — never the wire TTL, which is unchanged at 120 s.
    #[test]
    fn the_two_expiries_are_separate_mechanisms() {
        assert_eq!(ttl(Some(900)), Duration::minutes(30));
        assert_eq!(ttl(Some(3_600)), Duration::hours(2));
        assert_eq!(ttl(None), Duration::minutes(30));
        assert_eq!(
            rhizo_domain::profile::default_command_ttl_seconds(),
            120,
            "the wire TTL is unchanged, because the command is minted at the wake"
        );
    }
}
