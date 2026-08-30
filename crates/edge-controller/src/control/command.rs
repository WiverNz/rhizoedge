//! The command lifecycle: persist, publish, retry, settle, reconcile
//! (M6-008 … M6-012).
//!
//! # The order is the safety property
//!
//! ```text
//! [TX: commands row 'issued' + irrigation transition + outbox] COMMIT
//!      -> publish QoS 1, retain = false
//!      -> record published_at
//! ```
//!
//! The commit happens **before** the publish, always. The reverse order permits
//! a pump to run with no record it was asked to (F-060-20, SAFETY-010). A crash
//! in between leaves an `issued` row with no result, which is exactly the state
//! [`reconcile`] reads on the next boot — and which it never re-publishes.
//!
//! # A retry reuses the `command_id`, and this is the paragraph that matters
//!
//! The edge cannot distinguish "the publish failed" from "it succeeded and the
//! acknowledgement was lost". Issuing a *fresh* command after a failure would
//! make the device see two distinct ids and water twice, which
//! [ADR-014](../../../../docs/adr/014-failure-and-retry-policy.md) calls the
//! single most important paragraph in that document. QoS 1 redelivery of the
//! identical payload is safe precisely because the device deduplicates on
//! `command_id` (SAFETY-001); a new id defeats that entirely.
//!
//! A failed publish is a missed dose, which is recoverable. A double dose is
//! not.

use std::sync::Arc;
use std::time::Duration as StdDuration;

use chrono::{DateTime, Duration, Utc};
use rhizo_domain::Clock;
use rhizo_domain::irrigation::budget;
use rhizo_domain::irrigation::types::EvaluationMode;
use rhizo_mqtt_contract::payload::{
    CalibrateCommand, CommandResult, CommandStatus, TareCommand, WaterCommand,
};
use rhizo_mqtt_contract::{CommandId, Envelope, MessageId, MessageKind, Topic, UtcMillis};
use rhizo_storage::EdgeDb;
use rhizo_storage::repo::command as command_repo;

use crate::error::EdgeError;
use crate::metrics::Metrics;

use super::transport::{Transport, TransportError};

/// How many times a command publication is attempted before it is failed.
///
/// Three, per ADR-014 §Backoff. Not "until it works": an unbounded retry would
/// hold the plant in `DoseIssued` for ever, and a missed dose is recoverable.
pub const MAX_PUBLISH_ATTEMPTS: usize = 3;
/// The publish backoff base.
pub const PUBLISH_BACKOFF_BASE: StdDuration = StdDuration::from_millis(200);
/// The publish backoff cap.
pub const PUBLISH_BACKOFF_CAP: StdDuration = StdDuration::from_secs(2);

/// What the caller asked to be delivered.
#[derive(Clone, Debug)]
pub struct DoseRequest {
    /// The plant the dose is for.
    pub plant_id: String,
    /// The device carrying its actuator.
    pub device_id: String,
    /// The volume the machine chose. Never larger than the profile dose.
    pub requested_ml: f32,
    /// Who asked.
    pub mode: EvaluationMode,
    /// The wire TTL, from the plant's profile.
    pub ttl: Duration,
    /// The irrigation state to commit alongside the command row.
    pub next_state: command_repo::IrrigationStateRow,
}

/// How an issue attempt ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Issued {
    /// Persisted and published. The `command_id` is on the wire.
    Published {
        /// The id the device deduplicates on.
        command_id: String,
        /// When it expires, in edge milliseconds.
        expires_at: i64,
    },
    /// Persisted, then every publish attempt failed. The command is `failed`,
    /// **no watering event exists**, and the plant is back in `Recheck`.
    PublishFailed {
        /// The id that was allocated, and never reused for a different command.
        command_id: String,
    },
}

/// Owns the one path from a decision to the wire.
#[derive(Clone)]
pub struct Commander {
    db: EdgeDb,
    clock: Arc<dyn Clock>,
    transport: Arc<dyn Transport>,
    metrics: Metrics,
}

impl Commander {
    /// Builds a commander.
    #[must_use]
    pub fn new(
        db: EdgeDb,
        clock: Arc<dyn Clock>,
        transport: Arc<dyn Transport>,
        metrics: Metrics,
    ) -> Self {
        Self {
            db,
            clock,
            transport,
            metrics,
        }
    }

    /// The database this commander writes to.
    #[must_use]
    pub const fn db(&self) -> &EdgeDb {
        &self.db
    }

    /// The transport, for the tare/calibrate and config paths.
    #[must_use]
    pub fn transport(&self) -> &Arc<dyn Transport> {
        &self.transport
    }

    /// The edge clock.
    #[must_use]
    pub fn now(&self) -> DateTime<Utc> {
        self.clock.now()
    }

    /// The process metric set.
    #[must_use]
    pub const fn metrics_ref(&self) -> &Metrics {
        &self.metrics
    }

    /// Persists a water command and publishes it.
    ///
    /// # Errors
    ///
    /// A storage failure. A publish failure is **not** an error: it is a
    /// documented outcome that leaves the command `failed` and the plant in
    /// `Recheck` with no watering event (F-060-24).
    pub async fn issue_water(&self, request: &DoseRequest) -> Result<Issued, EdgeError> {
        let now = self.clock.now();
        let issued_at = now.timestamp_millis();
        let expires_at = issued_at + request.ttl.num_milliseconds().max(1);
        // UUIDv7 so the ledger sorts by issue time without a second column.
        let command_id = uuid::Uuid::now_v7().to_string();

        let mut state = request.next_state.clone();
        state.active_command_id = Some(command_id.clone());

        command_repo::issue(
            &self.db,
            &command_repo::NewCommand {
                command_id: command_id.clone(),
                device_id: request.device_id.clone(),
                plant_id: Some(request.plant_id.clone()),
                kind: "water".to_owned(),
                requested_ml: Some(f64::from(request.requested_ml)),
                mode: request.mode.as_str().to_owned(),
                issued_at,
                expires_at,
            },
            &state,
            issued_at,
        )
        .await?;

        tracing::info!(
            plant_id = %request.plant_id,
            device_id = %request.device_id,
            command_id = %command_id,
            requested_ml = request.requested_ml,
            mode = request.mode.as_str(),
            "dose issued"
        );

        // Built from the **persisted row**, not from the in-memory decision, so
        // what reaches the device is exactly what was recorded.
        let Some(row) = command_repo::get(&self.db, &command_id).await? else {
            return Err(EdgeError::Storage(rhizo_storage::StorageError::Database(
                "the command vanished between commit and publish".to_owned(),
            )));
        };
        let payload = water_payload(&row)?;
        let topic = Topic::CommandWater(device_id(&row.device_id)?).as_string();

        match self.publish_with_retry(&topic, payload).await {
            Ok(()) => {
                command_repo::mark_published(
                    &self.db,
                    &command_id,
                    self.clock.now().timestamp_millis(),
                )
                .await?;
                Ok(Issued::Published {
                    command_id,
                    expires_at,
                })
            }
            Err(error) => {
                tracing::error!(
                    plant_id = %request.plant_id,
                    command_id = %command_id,
                    %error,
                    "publication failed after every attempt; the command is failed and no watering event exists"
                );
                self.metrics
                    .watering_commands
                    .with_label_values(&[request.mode.as_str(), "publish_failed"])
                    .inc();
                self.metrics
                    .watering_failures
                    .with_label_values(&["publish_failed"])
                    .inc();
                let mut recheck = request.next_state.clone();
                recheck.state = "recheck".to_owned();
                recheck.active_command_id = None;
                command_repo::settle(
                    &self.db,
                    &command_id,
                    "failed",
                    Some("publish_failed"),
                    // No watering event. A failed publish delivered nothing.
                    None,
                    Some((&request.plant_id, &recheck)),
                    self.clock.now().timestamp_millis(),
                )
                .await?;
                Ok(Issued::PublishFailed { command_id })
            }
        }
    }

    /// Publishes a `tare` or `calibrate` command through the same path.
    ///
    /// Same persist-before-publish order, same retry, same `retain = false`.
    /// Calibration is a real dose into a real pot and goes through the device's
    /// full §5.8 gate on arrival; nothing here shortcuts that.
    pub async fn issue_device_command(
        &self,
        device: &str,
        kind: DeviceCommandKind,
        ttl: Duration,
    ) -> Result<Issued, EdgeError> {
        let now = self.clock.now();
        let issued_at = now.timestamp_millis();
        let expires_at = issued_at + ttl.num_milliseconds().max(1);
        let command_id = uuid::Uuid::now_v7().to_string();
        command_repo::issue(
            &self.db,
            &command_repo::NewCommand {
                command_id: command_id.clone(),
                device_id: device.to_owned(),
                plant_id: None,
                kind: kind.name().to_owned(),
                requested_ml: None,
                mode: "manual".to_owned(),
                issued_at,
                expires_at,
            },
            &command_repo::IrrigationStateRow::default(),
            issued_at,
        )
        .await?;
        let Some(row) = command_repo::get(&self.db, &command_id).await? else {
            return Err(EdgeError::Storage(rhizo_storage::StorageError::Database(
                "the command vanished between commit and publish".to_owned(),
            )));
        };
        let id = device_id(device)?;
        let (topic, payload) = match kind {
            DeviceCommandKind::Tare => (Topic::CommandTare(id).as_string(), tare_payload(&row)?),
            DeviceCommandKind::Calibrate { run_seconds } => (
                Topic::CommandCalibrate(id).as_string(),
                calibrate_payload(&row, run_seconds)?,
            ),
        };
        match self.publish_with_retry(&topic, payload).await {
            Ok(()) => {
                command_repo::mark_published(
                    &self.db,
                    &command_id,
                    self.clock.now().timestamp_millis(),
                )
                .await?;
                Ok(Issued::Published {
                    command_id,
                    expires_at,
                })
            }
            Err(_) => {
                command_repo::settle(
                    &self.db,
                    &command_id,
                    "failed",
                    Some("publish_failed"),
                    None,
                    None,
                    self.clock.now().timestamp_millis(),
                )
                .await?;
                Ok(Issued::PublishFailed { command_id })
            }
        }
    }

    /// Publishes with ADR-014's bounded backoff, **reusing the same payload**.
    ///
    /// The payload is built once, before the first attempt, and every retry
    /// sends those exact bytes. There is no code path here that could allocate a
    /// second `command_id`, which is the property M6-011 is about.
    async fn publish_with_retry(
        &self,
        topic: &str,
        payload: Vec<u8>,
    ) -> Result<(), TransportError> {
        let mut backoff = rhizo_telemetry::Backoff::new(PUBLISH_BACKOFF_BASE, PUBLISH_BACKOFF_CAP);
        let mut last = TransportError("no attempt was made".to_owned());
        for attempt in 1..=MAX_PUBLISH_ATTEMPTS {
            // `retain = false`. ADR-002 calls retaining a command topic the
            // single most damaging mistake available in this protocol: the
            // broker would redeliver it on every reconnect, indefinitely.
            match self
                .transport
                .publish(topic.to_owned(), payload.clone(), false)
                .await
            {
                Ok(()) => return Ok(()),
                Err(error) => {
                    tracing::warn!(
                        attempt,
                        max = MAX_PUBLISH_ATTEMPTS,
                        %error,
                        topic,
                        "command publication failed; retrying with the same command_id"
                    );
                    last = error;
                    if attempt < MAX_PUBLISH_ATTEMPTS {
                        tokio::time::sleep(backoff.next_delay()).await;
                    }
                }
            }
        }
        Err(last)
    }

    /// Applies a `command.result`, settling the command and its consequences.
    ///
    /// One transaction covers the status change, the optional `watering_event`,
    /// and the irrigation transition. Returns what happened so the caller can
    /// log and count it.
    pub async fn apply_result(&self, result: &CommandResult) -> Result<Settled, EdgeError> {
        let now = self.clock.now();
        let now_ms = now.timestamp_millis();
        let command_id = result.command_id.to_string();
        let Some(row) = command_repo::get(&self.db, &command_id).await? else {
            // The edge does not create a command row to match a result it never
            // issued. Logged, counted, and otherwise ignored.
            tracing::warn!(%command_id, "a result arrived for a command this edge never issued");
            return Ok(Settled::UnknownCommand);
        };
        if command_repo::is_terminal(&row.status) {
            return Ok(Settled::AlreadyTerminal);
        }

        let status = status_name(result.status);
        let mode = row.mode.clone();
        let requested = row.requested_ml.unwrap_or(0.0);
        let credited = budget::credited_ml(
            result.status,
            requested as f32,
            result.delivered_ml.filter(|v| v.is_finite()),
        );

        // Only `completed` asserts that water reached the plant. A rejected or
        // failed command must never create a watering event: the event is the
        // ledger, and one recorded for a refused command corrupts the daily
        // total and the cooldown in the *permissive* direction.
        let watering = (budget::creates_watering_event(result.status) && row.plant_id.is_some())
            .then(|| command_repo::NewWateringEvent {
                watering_event_id: uuid::Uuid::now_v7().to_string(),
                plant_id: row.plant_id.clone().unwrap_or_default(),
                device_id: row.device_id.clone(),
                mode: mode.clone(),
                started_at: row.published_at.unwrap_or(row.issued_at),
                completed_at: now_ms,
                requested_ml: row.requested_ml,
                delivered_ml: result.delivered_ml.filter(|v| v.is_finite()).map(f64::from),
                reason_json: None,
            });

        let next = next_state_for(&self.db, row.plant_id.as_deref(), result, now_ms).await?;
        let reason = result
            .reason
            .and_then(|r| serde_json::to_value(r).ok())
            .and_then(|v| v.as_str().map(ToOwned::to_owned));

        let applied = command_repo::settle(
            &self.db,
            &command_id,
            status,
            reason.as_deref(),
            watering.as_ref(),
            next.as_ref().map(|(id, state)| (id.as_str(), state)),
            now_ms,
        )
        .await?;
        if !applied {
            return Ok(Settled::AlreadyTerminal);
        }

        self.metrics
            .watering_commands
            .with_label_values(&[&mode, status])
            .inc();
        if let Some(event) = &watering {
            let delivered = event.delivered_ml.unwrap_or(0.0);
            self.metrics
                .watering_delivered_ml
                .with_label_values(&[&mode])
                .inc_by(delivered.max(0.0));
        }
        if !matches!(result.status, CommandStatus::Completed) {
            self.metrics
                .watering_failures
                .with_label_values(&[status])
                .inc();
        }
        tracing::info!(
            %command_id,
            plant_id = ?row.plant_id,
            status,
            delivered_ml = ?result.delivered_ml,
            credited_ml = credited,
            "command settled"
        );
        Ok(Settled::Applied {
            status: status.to_owned(),
            credited_ml: credited,
            created_watering_event: watering.is_some(),
        })
    }

    /// Reconciles in-flight commands on boot (M6-012, SAFETY-010).
    ///
    /// Expired commands become `expired` and their plant moves to `Recheck`;
    /// live ones are awaited until `expires_at`. **Nothing is re-published.**
    /// The original may well have been delivered, and republishing under a new
    /// id would double-water.
    pub async fn reconcile(&self) -> Result<Recovery, EdgeError> {
        let now_ms = self.clock.now().timestamp_millis();
        let mut recovery = Recovery::default();
        for row in command_repo::open_commands(&self.db).await? {
            if row.expires_at < now_ms {
                let next = match row.plant_id.as_deref() {
                    None => None,
                    Some(plant_id) => {
                        let mut state = command_repo::irrigation_state(&self.db, plant_id)
                            .await?
                            .unwrap_or_default();
                        state.state = "recheck".to_owned();
                        state.state_since = now_ms;
                        state.active_command_id = None;
                        Some((plant_id.to_owned(), state))
                    }
                };
                command_repo::settle(
                    &self.db,
                    &row.command_id,
                    "expired",
                    Some("expired_before_result"),
                    None,
                    next.as_ref().map(|(id, state)| (id.as_str(), state)),
                    now_ms,
                )
                .await?;
                self.metrics
                    .watering_commands
                    .with_label_values(&[&row.mode, "expired"])
                    .inc();
                recovery.expired += 1;
            } else {
                // Still live: mark it in flight and wait. The device may already
                // have it.
                command_repo::mark_published(&self.db, &row.command_id, now_ms).await?;
                recovery.awaiting += 1;
            }
        }
        recovery.republished = 0;
        tracing::info!(
            expired = recovery.expired,
            awaiting = recovery.awaiting,
            republished = recovery.republished,
            "reconciled in-flight commands; nothing was re-published"
        );
        Ok(recovery)
    }
}

/// A `tare` or a `calibrate`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DeviceCommandKind {
    /// Zero the scale.
    Tare,
    /// Run the pump for a fixed duration so an operator can measure it.
    Calibrate {
        /// How long to run.
        run_seconds: f32,
    },
}

impl DeviceCommandKind {
    /// The stored kind name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Tare => "tare",
            Self::Calibrate { .. } => "calibrate",
        }
    }
}

/// What settling a result did.
#[derive(Clone, Debug, PartialEq)]
pub enum Settled {
    /// The command settled.
    Applied {
        /// The terminal status.
        status: String,
        /// The volume charged to the rolling window.
        credited_ml: f32,
        /// Whether a `watering_event` was created.
        created_watering_event: bool,
    },
    /// The command was already terminal. Nothing was written.
    AlreadyTerminal,
    /// No such command. Nothing was written.
    UnknownCommand,
}

/// What boot reconciliation found.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Recovery {
    /// Commands whose TTL had passed.
    pub expired: usize,
    /// Commands still inside their TTL, now awaited.
    pub awaiting: usize,
    /// Always zero, and asserted to be (SAFETY-010).
    pub republished: usize,
}

/// The irrigation state a settled result leaves behind.
async fn next_state_for(
    db: &EdgeDb,
    plant_id: Option<&str>,
    result: &CommandResult,
    now_ms: i64,
) -> Result<Option<(String, command_repo::IrrigationStateRow)>, EdgeError> {
    let Some(plant_id) = plant_id else {
        return Ok(None);
    };
    let mut state = command_repo::irrigation_state(db, plant_id)
        .await?
        .unwrap_or_default();
    state.state_since = now_ms;
    state.active_command_id = None;
    match result.status {
        CommandStatus::Completed => {
            state.state = "wait_for_absorption".to_owned();
            state.doses_this_cycle += 1;
            if state.cycle_started_at.is_none() {
                state.cycle_started_at = Some(now_ms);
            }
            // The absorption window comes from the plant's own profile, read
            // here rather than defaulted: a plant configured for a ten-minute
            // absorption must not silently wait thirty.
            state.wait_until = Some(now_ms + absorption_ms(db, plant_id).await?);
        }
        // `rejected`, `interrupted`, and `failed` all return the plant to
        // `Recheck`. The first delivered nothing and the other two delivered an
        // unknown amount, which the budget has already been charged for.
        CommandStatus::Rejected
        | CommandStatus::Interrupted
        | CommandStatus::Failed
        | CommandStatus::Unknown => {
            state.state = "recheck".to_owned();
        }
    }
    Ok(Some((plant_id.to_owned(), state)))
}

/// The plant's configured absorption window, in milliseconds.
///
/// Falls back to the built-in template when the plant names no profile, which is
/// the same fallback every other read of a plant's configuration uses.
async fn absorption_ms(db: &EdgeDb, plant_id: &str) -> Result<i64, EdgeError> {
    let minutes = crate::plant::load(db, plant_id).await?.map_or_else(
        || crate::plant::default_profile().absorption_minutes,
        |loaded| loaded.profile.absorption_minutes,
    );
    Ok(i64::from(minutes) * 60_000)
}

/// The stored status name for a wire status.
#[must_use]
pub const fn status_name(status: CommandStatus) -> &'static str {
    match status {
        CommandStatus::Completed => "completed",
        CommandStatus::Rejected => "rejected",
        CommandStatus::Interrupted => "interrupted",
        // A status this contract version does not recognise settles the command
        // as `failed` rather than leaving it open: an unreadable outcome is an
        // outcome, and the budget has already been charged conservatively.
        CommandStatus::Failed | CommandStatus::Unknown => "failed",
    }
}

fn device_id(value: &str) -> Result<rhizo_mqtt_contract::DeviceId, EdgeError> {
    rhizo_mqtt_contract::DeviceId::parse(value)
        .map_err(|_| EdgeError::Decode(format!("`{value}` is not a valid device id")))
}

fn command_uuid(value: &str) -> Result<CommandId, EdgeError> {
    value
        .parse::<uuid::Uuid>()
        .map(CommandId::from_uuid)
        .map_err(|_| EdgeError::Decode(format!("`{value}` is not a valid command id")))
}

fn envelope<T: serde::Serialize>(
    kind: MessageKind,
    device: rhizo_mqtt_contract::DeviceId,
    data: T,
) -> Result<Vec<u8>, EdgeError> {
    Envelope {
        v: rhizo_mqtt_contract::PROTOCOL_VERSION,
        kind,
        message_id: MessageId::from_uuid(uuid::Uuid::now_v7()),
        device_id: device,
        boot_id: None,
        sequence: None,
        device_time_ms: None,
        clock_synced: None,
        data,
    }
    .to_json()
    .map(String::into_bytes)
    .map_err(|e| EdgeError::Decode(e.to_string()))
}

/// Builds the `command.water` envelope **from the persisted row**.
pub fn water_payload(row: &command_repo::CommandRow) -> Result<Vec<u8>, EdgeError> {
    let command = WaterCommand {
        command_id: command_uuid(&row.command_id)?,
        requested_ml: row.requested_ml.unwrap_or(0.0) as f32,
        issued_at_ms: UtcMillis(row.issued_at),
        expires_at_ms: UtcMillis(row.expires_at),
    };
    command
        .validate()
        .map_err(|e| EdgeError::Decode(format!("refusing to publish an invalid command: {e:?}")))?;
    envelope(
        MessageKind::CommandWater,
        device_id(&row.device_id)?,
        command,
    )
}

/// Builds the `command.tare` envelope from the persisted row.
pub fn tare_payload(row: &command_repo::CommandRow) -> Result<Vec<u8>, EdgeError> {
    let command = TareCommand {
        command_id: command_uuid(&row.command_id)?,
        issued_at_ms: UtcMillis(row.issued_at),
        expires_at_ms: UtcMillis(row.expires_at),
    };
    command
        .validate()
        .map_err(|e| EdgeError::Decode(format!("refusing to publish an invalid command: {e:?}")))?;
    envelope(
        MessageKind::CommandTare,
        device_id(&row.device_id)?,
        command,
    )
}

/// Builds the `command.calibrate` envelope from the persisted row.
pub fn calibrate_payload(
    row: &command_repo::CommandRow,
    run_seconds: f32,
) -> Result<Vec<u8>, EdgeError> {
    let command = CalibrateCommand {
        command_id: command_uuid(&row.command_id)?,
        run_seconds,
        issued_at_ms: UtcMillis(row.issued_at),
        expires_at_ms: UtcMillis(row.expires_at),
    };
    command
        .validate()
        .map_err(|e| EdgeError::Decode(format!("refusing to publish an invalid command: {e:?}")))?;
    envelope(
        MessageKind::CommandCalibrate,
        device_id(&row.device_id)?,
        command,
    )
}

/// The command lifecycle's own tests.
///
/// The module names are the API: the M6 issues quote `command::persist`,
/// `command::publish`, `command::retry`, `command::result`, and
/// `startup::reconcile` literally, and each path below spells one of them out.
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
#[allow(
    clippy::module_inception,
    reason = "the module name is the verification filter the issue quotes literally"
)]
mod command {
    use super::*;
    use crate::api::testsupport::TestApi;
    use rhizo_mqtt_contract::payload::{CommandOrigin, RejectReason};

    fn water_result(
        command_id: &str,
        status: CommandStatus,
        delivered_ml: Option<f32>,
    ) -> CommandResult {
        CommandResult {
            command_id: CommandId::from_uuid(command_id.parse().unwrap()),
            status,
            requested_ml: 40.0,
            delivered_ml,
            duration_ms: Some(4_878),
            clamped: false,
            reason: (status == CommandStatus::Rejected).then_some(RejectReason::LeakDetected),
            delivered_today_ml: 40.0,
            origin: CommandOrigin::EdgeCommand,
            detail: None,
        }
    }

    async fn issue(api: &TestApi) -> String {
        api.waterable("monstera-01").await;
        api.device_connected().await;
        let (status, _) = api
            .json(
                "POST",
                "/api/v1/plants/monstera-01/water",
                serde_json::json!({ "ml": 40.0, "mode": "manual" }),
            )
            .await;
        assert_eq!(status, axum::http::StatusCode::ACCEPTED);
        let published = api.transport.commands();
        assert_eq!(published.len(), 1);
        let body: serde_json::Value = serde_json::from_slice(&published[0].payload).unwrap();
        body["data"]["command_id"].as_str().unwrap().to_owned()
    }

    // ------------------------------------------------------------- persistence

    mod persist {
        use super::*;

        /// F-060-20: the row is committed, with status `issued`, **before** the
        /// publish — so the state a crash in between leaves is a recorded
        /// command with no result, never a pump that ran unrecorded.
        #[tokio::test]
        async fn the_row_is_committed_before_the_publish() {
            let api = TestApi::start().await;
            api.waterable("monstera-01").await;
            api.device_connected().await;
            // A transport that always fails: the publish never happens, and the
            // row must exist anyway.
            api.transport.fail_next(usize::MAX);
            let pass = api.irrigate("monstera-01").await;
            assert!(pass.command_id.is_none(), "automation is off by default");

            api.transport.clear();
            api.transport.fail_next(usize::MAX);
            let (status, body) = api
                .json(
                    "POST",
                    "/api/v1/plants/monstera-01/water",
                    serde_json::json!({ "ml": 40.0 }),
                )
                .await;
            assert_eq!(
                status,
                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                "{body}"
            );
            let command_id = body["error"]["details"]["command_id"].as_str().unwrap();
            let row = command_repo::get(&api.db, command_id)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(row.status, "failed", "every publish attempt failed");
            assert_eq!(row.published_at, None);
            // F-060-24: no watering event exists for a command that never
            // reached the device.
            let events: i64 = sqlx::query_scalar("SELECT count(*) FROM watering_events")
                .fetch_one(api.db.pool())
                .await
                .unwrap();
            assert_eq!(events, 0);
        }

        /// The insert, the irrigation transition, and the outbox row share one
        /// transaction (F-060-14).
        #[tokio::test]
        async fn the_transition_and_the_outbox_row_share_the_transaction() {
            let api = TestApi::start().await;
            let command_id = issue(&api).await;
            let state = command_repo::irrigation_state(&api.db, "monstera-01")
                .await
                .unwrap()
                .unwrap();
            assert_eq!(state.state, "dose_issued");
            assert_eq!(
                state.active_command_id.as_deref(),
                Some(command_id.as_str())
            );
            let outbox: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM pending_cloud_events WHERE kind='command.issued'",
            )
            .fetch_one(api.db.pool())
            .await
            .unwrap();
            assert_eq!(outbox, 1);
        }

        /// F-060-22: the TTL comes from the profile, not from the request.
        #[tokio::test]
        async fn the_ttl_comes_from_the_profile() {
            let api = TestApi::start().await;
            let command_id = issue(&api).await;
            let row = command_repo::get(&api.db, &command_id)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                row.expires_at - row.issued_at,
                120_000,
                "the documented 120-second default"
            );
        }

        /// F-060-21: a duplicate `command_id` is refused by the primary key, so
        /// the guarantee holds even against a check-then-insert race.
        #[tokio::test]
        async fn a_duplicate_command_id_is_refused_at_the_storage_layer() {
            let api = TestApi::start().await;
            let command_id = issue(&api).await;
            let row = command_repo::get(&api.db, &command_id)
                .await
                .unwrap()
                .unwrap();
            let error = command_repo::issue(
                &api.db,
                &command_repo::NewCommand {
                    command_id: row.command_id.clone(),
                    device_id: row.device_id.clone(),
                    plant_id: row.plant_id.clone(),
                    kind: row.kind.clone(),
                    requested_ml: row.requested_ml,
                    mode: row.mode.clone(),
                    issued_at: row.issued_at,
                    expires_at: row.expires_at,
                },
                &command_repo::IrrigationStateRow::default(),
                row.issued_at,
            )
            .await
            .unwrap_err();
            assert!(matches!(error, rhizo_storage::StorageError::Constraint(_)));
        }
    }

    // ------------------------------------------------------------- publication

    mod publish {
        use super::*;

        /// A valid envelope, QoS 1, on the right topic — and built from the
        /// **persisted row**, so what is published is exactly what was recorded.
        #[tokio::test]
        async fn the_envelope_matches_the_persisted_row() {
            let api = TestApi::start().await;
            let command_id = issue(&api).await;
            let published = api.transport.commands();
            assert_eq!(
                published[0].topic,
                "rhizo/v1/devices/plant-node-01/commands/water"
            );
            let envelope: rhizo_mqtt_contract::Envelope<
                rhizo_mqtt_contract::payload::WaterCommand,
            > = rhizo_mqtt_contract::Envelope::from_json(&published[0].payload).unwrap();
            let row = command_repo::get(&api.db, &command_id)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(envelope.data.command_id.to_string(), row.command_id);
            assert_eq!(envelope.data.issued_at_ms.0, row.issued_at);
            assert_eq!(envelope.data.expires_at_ms.0, row.expires_at);
            assert!(
                (f64::from(envelope.data.requested_ml) - row.requested_ml.unwrap()).abs() < 1e-6
            );
            assert!(envelope.data.validate().is_ok());
        }

        /// ADR-002: **`retain` is false on every command publish.** A retained
        /// command would be redelivered on every reconnect, indefinitely.
        #[tokio::test]
        async fn no_retained_commands() {
            let api = TestApi::start().await;
            issue(&api).await;
            for message in api.transport.published() {
                assert!(!message.retain, "{} was published retained", message.topic);
            }
        }

        /// `published_at` is recorded once the broker has the message.
        #[tokio::test]
        async fn publication_is_recorded() {
            let api = TestApi::start().await;
            let command_id = issue(&api).await;
            let row = command_repo::get(&api.db, &command_id)
                .await
                .unwrap()
                .unwrap();
            assert!(row.published_at.is_some());
            assert_eq!(row.status, "in_flight");
        }

        /// Tare and calibrate take the same path, and are equally unretained.
        #[tokio::test]
        async fn tare_and_calibrate_use_the_same_path() {
            let api = TestApi::start().await;
            api.with_device().await;
            for (path, body) in [
                ("tare", serde_json::json!({})),
                ("calibrate", serde_json::json!({ "run_seconds": 10.0 })),
            ] {
                let (status, response) = api
                    .json(
                        "POST",
                        &format!("/api/v1/devices/plant-node-01/commands/{path}"),
                        body,
                    )
                    .await;
                assert_eq!(status, axum::http::StatusCode::ACCEPTED, "{response}");
                assert!(response["command_id"].is_string());
            }
            let topics: Vec<String> = api
                .transport
                .commands()
                .into_iter()
                .map(|m| m.topic)
                .collect();
            assert!(topics.iter().any(|t| t.ends_with("/commands/tare")));
            assert!(topics.iter().any(|t| t.ends_with("/commands/calibrate")));
            assert!(api.transport.published().iter().all(|m| !m.retain));
        }
    }

    // ------------------------------------------------------------------- retry

    mod retry {
        use super::*;

        /// **The assertion ADR-014 calls the most important in the document.**
        /// Two failures then a success: the device sees **one** `command_id`.
        #[tokio::test]
        async fn a_transient_failure_is_retried_with_the_same_command_id() {
            let api = TestApi::start().await;
            api.waterable("monstera-01").await;
            api.device_connected().await;
            api.transport.fail_next(2);

            let (status, body) = api
                .json(
                    "POST",
                    "/api/v1/plants/monstera-01/water",
                    serde_json::json!({ "ml": 40.0 }),
                )
                .await;
            assert_eq!(status, axum::http::StatusCode::ACCEPTED, "{body}");
            assert_eq!(api.transport.attempts(), 3, "one attempt plus two retries");

            let published = api.transport.commands();
            assert_eq!(published.len(), 1, "one message reached the broker");
            let envelope: rhizo_mqtt_contract::Envelope<
                rhizo_mqtt_contract::payload::WaterCommand,
            > = rhizo_mqtt_contract::Envelope::from_json(&published[0].payload).unwrap();
            assert_eq!(
                envelope.data.command_id.to_string(),
                body["command_id"].as_str().unwrap(),
                "the retry reused the id, and did not mint a new one"
            );
            let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM commands")
                .fetch_one(api.db.pool())
                .await
                .unwrap();
            assert_eq!(rows, 1, "and no second command row exists");
        }

        /// F-060-24: after three failures the command is `failed`, the plant is
        /// back in `Recheck`, and **no watering event is created**.
        #[tokio::test]
        async fn exhausting_the_retries_fails_the_command_and_creates_no_event() {
            let api = TestApi::start().await;
            api.waterable("monstera-01").await;
            api.device_connected().await;
            api.transport.fail_next(usize::MAX);

            let (status, body) = api
                .json(
                    "POST",
                    "/api/v1/plants/monstera-01/water",
                    serde_json::json!({ "ml": 40.0 }),
                )
                .await;
            assert_eq!(status, axum::http::StatusCode::SERVICE_UNAVAILABLE);
            assert_eq!(api.transport.attempts(), MAX_PUBLISH_ATTEMPTS);
            let command_id = body["error"]["details"]["command_id"].as_str().unwrap();
            let row = command_repo::get(&api.db, command_id)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(row.status, "failed");
            assert_eq!(row.reason.as_deref(), Some("publish_failed"));
            let state = command_repo::irrigation_state(&api.db, "monstera-01")
                .await
                .unwrap()
                .unwrap();
            assert_eq!(state.state, "recheck");
            assert_eq!(state.active_command_id, None);
            let events: i64 = sqlx::query_scalar("SELECT count(*) FROM watering_events")
                .fetch_one(api.db.pool())
                .await
                .unwrap();
            assert_eq!(events, 0);
        }

        /// The backoff bounds ADR-014 fixes for this site.
        #[test]
        fn the_backoff_bounds_are_the_documented_ones() {
            assert_eq!(MAX_PUBLISH_ATTEMPTS, 3);
            assert_eq!(PUBLISH_BACKOFF_BASE, StdDuration::from_millis(200));
            assert_eq!(PUBLISH_BACKOFF_CAP, StdDuration::from_secs(2));
            let mut backoff =
                rhizo_telemetry::Backoff::new(PUBLISH_BACKOFF_BASE, PUBLISH_BACKOFF_CAP);
            for _ in 0..10 {
                assert!(backoff.next_delay() <= PUBLISH_BACKOFF_CAP);
            }
        }

        /// **No code path mints a second `command_id` for one dose.** Checked
        /// structurally as well as behaviourally: a retry that allocated a fresh
        /// id is the most plausible route to duplicate watering in the design.
        #[test]
        fn no_code_path_generates_a_new_command_id_on_retry() {
            // Only the production half: the tests below legitimately mint ids
            // of their own, and scanning them would make this assertion about
            // the wrong code.
            let source = include_str!("command.rs");
            let production = source
                .split(
                    "
#[cfg(test)]",
                )
                .next()
                .expect("the file has a production half");
            let allocations = production
                .lines()
                .filter(|line| {
                    let trimmed = line.trim_start();
                    !trimmed.starts_with("//")
                        && line.contains("let command_id = uuid::Uuid::now_v7()")
                })
                .count();
            assert_eq!(
                allocations, 2,
                "one allocation for a water command and one for a device command; a third would be a retry minting a fresh id"
            );
            let after = production
                .split("async fn publish_with_retry")
                .nth(1)
                .expect("the retry loop exists");
            // Bounded to the function's own body: the next doc comment starts
            // the following method, and scanning past it would prove nothing.
            let retry = after
                .split(
                    "
    ///",
                )
                .next()
                .unwrap_or(after);
            assert!(
                !retry.contains("Uuid::"),
                "the retry loop must not be able to allocate an id at all"
            );
        }
    }

    // ------------------------------------------------------------------ result

    mod result {
        use super::*;

        /// `completed` creates exactly one `watering_event` and moves the plant
        /// into its absorption wait.
        #[tokio::test]
        async fn completed_creates_one_watering_event() {
            let api = TestApi::start().await;
            let command_id = issue(&api).await;
            let settled = api
                .commander
                .apply_result(&water_result(
                    &command_id,
                    CommandStatus::Completed,
                    Some(38.5),
                ))
                .await
                .unwrap();
            assert!(matches!(
                settled,
                Settled::Applied {
                    created_watering_event: true,
                    ..
                }
            ));
            let rows: Vec<(String, f64)> = sqlx::query_as(
                "SELECT plant_id,delivered_ml FROM watering_events WHERE command_id=?",
            )
            .bind(&command_id)
            .fetch_all(api.db.pool())
            .await
            .unwrap();
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].0, "monstera-01");
            assert!((rows[0].1 - 38.5).abs() < 1e-6);
            let state = command_repo::irrigation_state(&api.db, "monstera-01")
                .await
                .unwrap()
                .unwrap();
            assert_eq!(state.state, "wait_for_absorption");
            assert_eq!(state.doses_this_cycle, 1);
            assert!(state.wait_until.is_some());
        }

        /// `rejected` creates **none** and records the reason. A watering event
        /// asserts that water reached the plant.
        #[tokio::test]
        async fn rejected_creates_no_event_and_records_the_reason() {
            let api = TestApi::start().await;
            let command_id = issue(&api).await;
            api.commander
                .apply_result(&water_result(&command_id, CommandStatus::Rejected, None))
                .await
                .unwrap();
            let row = command_repo::get(&api.db, &command_id)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(row.status, "rejected");
            assert_eq!(row.reason.as_deref(), Some("leak_detected"));
            let events: i64 = sqlx::query_scalar("SELECT count(*) FROM watering_events")
                .fetch_one(api.db.pool())
                .await
                .unwrap();
            assert_eq!(events, 0);
            assert_eq!(
                command_repo::delivered_in_window(&api.db, "monstera-01", 0)
                    .await
                    .unwrap(),
                0.0
            );
        }

        /// `interrupted` creates no event but credits the **full** requested
        /// volume: over-counting reduces the next dose, under-counting could
        /// permit an extra one (F-060-26).
        #[tokio::test]
        async fn interrupted_credits_the_full_request_without_an_event() {
            let api = TestApi::start().await;
            let command_id = issue(&api).await;
            let settled = api
                .commander
                .apply_result(&water_result(&command_id, CommandStatus::Interrupted, None))
                .await
                .unwrap();
            assert!(matches!(
                settled,
                Settled::Applied {
                    created_watering_event: false,
                    credited_ml,
                    ..
                } if (credited_ml - 40.0).abs() < f32::EPSILON
            ));
            let events: i64 = sqlx::query_scalar("SELECT count(*) FROM watering_events")
                .fetch_one(api.db.pool())
                .await
                .unwrap();
            assert_eq!(events, 0, "no water is known to have reached the plant");
        }

        /// A duplicate result creates no second event, whatever it says.
        #[tokio::test]
        async fn a_duplicate_result_changes_nothing() {
            let api = TestApi::start().await;
            let command_id = issue(&api).await;
            api.commander
                .apply_result(&water_result(
                    &command_id,
                    CommandStatus::Completed,
                    Some(40.0),
                ))
                .await
                .unwrap();
            for status in [
                CommandStatus::Completed,
                CommandStatus::Failed,
                CommandStatus::Rejected,
            ] {
                assert_eq!(
                    api.commander
                        .apply_result(&water_result(&command_id, status, Some(40.0)))
                        .await
                        .unwrap(),
                    Settled::AlreadyTerminal
                );
            }
            let events: i64 = sqlx::query_scalar("SELECT count(*) FROM watering_events")
                .fetch_one(api.db.pool())
                .await
                .unwrap();
            assert_eq!(events, 1);
        }

        /// A result for a command this edge never issued is logged and ignored.
        /// The edge does not invent a command row to match it.
        #[tokio::test]
        async fn an_unknown_command_id_is_ignored() {
            let api = TestApi::start().await;
            api.waterable("monstera-01").await;
            let stranger = uuid::Uuid::now_v7().to_string();
            assert_eq!(
                api.commander
                    .apply_result(&water_result(
                        &stranger,
                        CommandStatus::Completed,
                        Some(40.0)
                    ))
                    .await
                    .unwrap(),
                Settled::UnknownCommand
            );
            let commands: i64 = sqlx::query_scalar("SELECT count(*) FROM commands")
                .fetch_one(api.db.pool())
                .await
                .unwrap();
            assert_eq!(commands, 0, "the edge does not invent a command row");
            let events: i64 = sqlx::query_scalar("SELECT count(*) FROM watering_events")
                .fetch_one(api.db.pool())
                .await
                .unwrap();
            assert_eq!(events, 0);
        }

        /// **M6-010's carried-forward requirement.** A result is not treated as
        /// delivered until the edge has committed it: the ingress uses manual
        /// acknowledgement and the pipeline acknowledges only after `process`
        /// returns, so the device's retry stops on the edge's durable commit
        /// rather than on the broker's receipt.
        #[test]
        fn the_acknowledgement_follows_the_commit() {
            let ingress = include_str!("../mqtt/ingress.rs");
            assert!(
                ingress.contains("set_manual_acks(true)"),
                "automatic acknowledgement would PUBACK before the transaction commits"
            );
            let pipeline = include_str!("../pipeline/mod.rs");
            let ack_after_ok = pipeline.contains("Ok(()) => acknowledge(&client, &item)");
            assert!(
                ack_after_ok,
                "the acknowledgement must be sent on the success arm of `process`"
            );
            assert!(
                !pipeline.contains("acknowledge(&client, &item);\n                let outcome"),
                "and never before it"
            );
        }
    }

    // ------------------------------------------------------- boot reconciliation

    mod startup {
        use super::*;

        mod reconcile {
            use super::*;

            /// SAFETY-010. A restart expires what has timed out, awaits what has
            /// not, and **re-publishes nothing**.
            #[tokio::test]
            async fn safety_010_restart_mid_command_no_replay() {
                let api = TestApi::start().await;
                let command_id = issue(&api).await;
                api.transport.clear();

                // "Restarting" is running the recovery procedure against the
                // same durable rows. The command is still inside its TTL.
                let recovery = api.commander.reconcile().await.unwrap();
                assert_eq!(recovery.awaiting, 1);
                assert_eq!(recovery.expired, 0);
                assert_eq!(recovery.republished, 0);
                assert!(
                    api.transport.commands().is_empty(),
                    "a restart must publish nothing at all"
                );

                // The device's result then arrives once, and produces one event.
                api.commander
                    .apply_result(&water_result(
                        &command_id,
                        CommandStatus::Completed,
                        Some(40.0),
                    ))
                    .await
                    .unwrap();
                let events: i64 = sqlx::query_scalar("SELECT count(*) FROM watering_events")
                    .fetch_one(api.db.pool())
                    .await
                    .unwrap();
                assert_eq!(events, 1);
                let commands: i64 = sqlx::query_scalar("SELECT count(*) FROM commands")
                    .fetch_one(api.db.pool())
                    .await
                    .unwrap();
                assert_eq!(commands, 1, "no second command exists");
            }

            /// An expired in-flight command becomes `expired` and the plant
            /// moves to `Recheck`.
            #[tokio::test]
            async fn an_expired_command_is_expired_and_the_plant_rechecks() {
                let api = TestApi::start().await;
                let command_id = issue(&api).await;
                api.clock.advance(chrono::Duration::seconds(121));
                let recovery = api.commander.reconcile().await.unwrap();
                assert_eq!(recovery.expired, 1);
                let row = command_repo::get(&api.db, &command_id)
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(row.status, "expired");
                assert_eq!(row.reason.as_deref(), Some("expired_before_result"));
                let state = command_repo::irrigation_state(&api.db, "monstera-01")
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(state.state, "recheck");
                assert!(
                    api.transport.commands().len() <= 1,
                    "and nothing new is sent"
                );
            }

            /// A late result for an expired command settles nothing: terminal is
            /// terminal.
            #[tokio::test]
            async fn safety_010_terminal_commands_are_never_reissued() {
                let api = TestApi::start().await;
                let command_id = issue(&api).await;
                api.clock.advance(chrono::Duration::seconds(121));
                api.commander.reconcile().await.unwrap();
                assert_eq!(
                    api.commander
                        .apply_result(&water_result(
                            &command_id,
                            CommandStatus::Completed,
                            Some(40.0)
                        ))
                        .await
                        .unwrap(),
                    Settled::AlreadyTerminal
                );
                let events: i64 = sqlx::query_scalar("SELECT count(*) FROM watering_events")
                    .fetch_one(api.db.pool())
                    .await
                    .unwrap();
                assert_eq!(events, 0);
            }

            /// SCEN-052: a restart during `WaitForAbsorption` resumes with the
            /// original `wait_until`, never a fresh default.
            #[tokio::test]
            async fn a_restart_during_absorption_preserves_the_original_deadline() {
                let api = TestApi::start().await;
                let command_id = issue(&api).await;
                api.commander
                    .apply_result(&water_result(
                        &command_id,
                        CommandStatus::Completed,
                        Some(40.0),
                    ))
                    .await
                    .unwrap();
                let before = command_repo::irrigation_state(&api.db, "monstera-01")
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(before.state, "wait_for_absorption");

                api.commander.reconcile().await.unwrap();
                let after = command_repo::irrigation_state(&api.db, "monstera-01")
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(after.wait_until, before.wait_until);
                assert_eq!(after.doses_this_cycle, before.doses_this_cycle);
                assert_eq!(after.state, "wait_for_absorption");
            }
        }
    }
}
