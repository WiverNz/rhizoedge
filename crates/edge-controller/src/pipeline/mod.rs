//! Decode, validate, and transactionally persist inbound device messages.
mod quarantine;
use crate::{
    error::EdgeError, metrics::Metrics, mqtt::ingress::Inbound, state::cache::LatestSampleCache,
};
use rhizo_domain::Clock;
use rhizo_mqtt_contract::payload::{
    ActuatorState, CommandResult, DeviceEventBatch, DeviceStatus, TelemetryBatch,
};
use rhizo_mqtt_contract::{Envelope, Topic};
use rhizo_storage::StorageError;
use rhizo_storage::repo::ingest::{self, Dedup};
use rhizo_telemetry::{Classify, FailureKind};
use std::{future::Future, sync::Arc, time::Duration};
use tokio::sync::{mpsc, watch};

/// Owns the single logical SQLite writer path.
///
/// # The acknowledgement follows the commit
///
/// `process` returns only after its transaction has committed, and **only then**
/// is the PUBACK sent. That ordering is what makes a device's "stop retrying"
/// condition depend on the edge's durable commit rather than on the broker's
/// receipt (M6-010). A message whose processing failed transiently is not
/// acknowledged, so the broker redelivers it; a message that was quarantined
/// **is** acknowledged, because redelivering a permanently unparseable payload
/// for ever would wedge every device behind it.
#[allow(
    clippy::too_many_arguments,
    reason = "the pipeline owns the single writer path and needs every one of               them; a bundle would hide that the commander is optional"
)]
pub async fn run(
    mut rx: mpsc::Receiver<Inbound>,
    db: rhizo_storage::EdgeDb,
    clock: Arc<dyn Clock>,
    client: rumqttc::AsyncClient,
    commander: Option<crate::control::command::Commander>,
    cache: LatestSampleCache,
    mut shutdown: watch::Receiver<bool>,
    metrics: Metrics,
) -> Result<(), String> {
    let mut limiter = quarantine::Limiter::default();
    loop {
        tokio::select! {
            biased;
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() { return Ok(()) }
            },
            item = rx.recv() => {
                let Some(item) = item else { return Ok(()) };
                let outcome = process(
                    &db, clock.as_ref(), &client, commander.as_ref(), &cache,
                    &metrics, &mut limiter, &item,
                ).await;
                match outcome {
                    Ok(()) => acknowledge(&client, &item),
                    Err(error) => {
                        let at = clock.now().timestamp_millis();
                        let classification = error.classify();
                        apply_classification(&db, &mut limiter, &item, at, &error).await?;
                        if classification == FailureKind::Permanent {
                            acknowledge(&client, &item);
                        }
                    }
                }
            }
        }
    }
}

/// Sends the PUBACK for one processed message.
///
/// A failure here is not fatal: the broker will redeliver, and redelivery is
/// deduplicated on `message_id`. Losing an acknowledgement costs a repeat;
/// sending one early costs history.
fn acknowledge(client: &rumqttc::AsyncClient, item: &Inbound) {
    if let Some(publish) = item.publish.as_ref()
        && let Err(error) = client.try_ack(publish)
    {
        tracing::warn!(topic = %item.topic, %error, "could not acknowledge a committed message");
    }
}

/// Applies ADR-014's classification at the pipeline's one message failure site.
///
/// The classification is what decides the outcome, not the call site: `Fatal`
/// propagates and the supervisor exits the process, `Permanent` quarantines the
/// message and carries on — a permanently failing message that stopped the
/// process would take every other device down with it — and a `Transient`
/// failure that has already exhausted its bounded retries leaves the message
/// unprocessed.
async fn apply_classification(
    db: &rhizo_storage::EdgeDb,
    limiter: &mut quarantine::Limiter,
    item: &Inbound,
    at: i64,
    error: &EdgeError,
) -> Result<(), String> {
    let classification = error.classify();
    match classification {
        FailureKind::Fatal => Err(error.to_string()),
        FailureKind::Permanent => {
            tracing::error!(topic=%item.topic,%classification,error=%error,"quarantining a permanently failing message");
            let device = Topic::parse(&item.topic)
                .ok()
                .map(|topic| topic.device_id().to_string());
            if limiter.allow(device.as_deref().unwrap_or_default(), at)
                && let Err(failure) = rhizo_storage::repo::quarantine::insert(
                    db,
                    device.as_deref(),
                    &item.topic,
                    &item.payload,
                    &error.to_string(),
                    at,
                )
                .await
            {
                // A failure while quarantining is judged on its own terms, so a
                // full or broken database still stops the process.
                if failure.classify().is_fatal() {
                    return Err(failure.to_string());
                }
                tracing::error!(error=%failure,"could not quarantine the failing message");
            }
            Ok(())
        }
        FailureKind::Transient => {
            tracing::warn!(topic=%item.topic,%classification,error=%error,"leaving a message unprocessed after exhausting its bounded retries");
            Ok(())
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn process(
    db: &rhizo_storage::EdgeDb,
    clock: &dyn Clock,
    client: &rumqttc::AsyncClient,
    commander: Option<&crate::control::command::Commander>,
    cache: &LatestSampleCache,
    m: &Metrics,
    limiter: &mut quarantine::Limiter,
    item: &Inbound,
) -> Result<(), EdgeError> {
    let topic = match Topic::parse(&item.topic) {
        Ok(t) => t,
        Err(_) => {
            m.decode.with_label_values(&["topic"]).inc();
            return Ok(());
        }
    };
    if !matches!(
        topic,
        Topic::Telemetry(_)
            | Topic::Actuator(_)
            | Topic::Events(_)
            | Topic::Status(_)
            | Topic::CommandResult(_)
    ) {
        m.decode.with_label_values(&["topic_direction"]).inc();
        return Ok(());
    }
    let at = clock.now().timestamp_millis();
    let started = std::time::Instant::now();
    let result = match &topic {
        Topic::Telemetry(_) => decode::<TelemetryBatch>(&topic, &item.payload).map(Msg::Telemetry),
        Topic::Actuator(_) => decode::<ActuatorState>(&topic, &item.payload).map(Msg::Actuator),
        Topic::Events(_) => decode::<DeviceEventBatch>(&topic, &item.payload).map(Msg::Events),
        Topic::Status(_) => decode_status(&topic, &item.payload).map(Msg::Status),
        Topic::CommandResult(_) => decode::<CommandResult>(&topic, &item.payload).map(Msg::Result),
        _ => unreachable!(),
    };
    let msg = match result {
        Ok(v) => v,
        Err(e) => {
            let reason = e.metric_reason();
            m.decode.with_label_values(&[reason]).inc();
            let dev = topic.device_id().to_string();
            if limiter.allow(&dev, at) {
                rhizo_storage::repo::quarantine::insert(
                    db,
                    Some(&dev),
                    &item.topic,
                    &item.payload,
                    &e.to_string(),
                    at,
                )
                .await?;
            }
            return Ok(());
        }
    };
    let kind = msg.kind();
    m.received.with_label_values(&[kind]).inc();
    let dedup = match msg {
        Msg::Telemetry(e) => {
            let (d, n) = retry_transient(m, || ingest::persist_telemetry(db, &e, at)).await?;
            if d == Dedup::New {
                for sample in rhizo_storage::repo::query::latest_samples(db).await? {
                    cache.update(sample);
                }
            }
            for sample in &e.data.samples {
                if sample.validate().is_valid() {
                    m.measurements.with_label_values(&["sample"]).inc();
                } else {
                    m.sensor_errors
                        .with_label_values(&["measurement", "validation"])
                        .inc();
                }
            }
            debug_assert!(n <= e.data.samples.len());
            d
        }
        Msg::Actuator(e) => retry_transient(m, || ingest::persist_actuator(db, &e, at)).await?,
        Msg::Status(e) => {
            let result =
                retry_transient(m, || ingest::persist_status_with_transitions(db, &e, at)).await?;
            publish_edge_time(client, &e.device_id, at).await?;
            if result.dedup == Dedup::New {
                let device_id = e.device_id.to_string();
                if result
                    .transitions
                    .iter()
                    .any(|transition| transition.kind == "device_restart")
                {
                    m.device_restarts.with_label_values(&[&device_id]).inc();
                }
                log_device_transitions(&device_id, &result.transitions);
            }
            result.dedup
        }
        Msg::Result(e) => {
            // The settlement commits **first**, and is idempotent on the
            // command's terminal status. A crash between the two commits is
            // therefore harmless: the redelivered result re-applies a settlement
            // that is already terminal, which writes nothing, and then records
            // the transport row. The reverse order would let a crash lose the
            // ledger entry while remembering that the message was seen.
            if let Some(commander) = commander
                && commander.apply_result(&e.data).await?
                    == crate::control::command::Settled::UnknownCommand
            {
                m.watering_failures
                    .with_label_values(&["unknown_command"])
                    .inc();
            }
            retry_transient(m, || ingest::persist_command_result(db, &e, at)).await?
        }
        Msg::Events(e) => {
            for event in &e.data.events {
                if let rhizo_mqtt_contract::payload::EventDetail::Gap { lost_tier, .. } =
                    event.detail
                {
                    let tier = serde_json::to_value(lost_tier)
                        .ok()
                        .and_then(|v| v.as_str().map(ToOwned::to_owned))
                        .unwrap_or_else(|| "unknown".to_owned());
                    m.history_gaps.with_label_values(&[&tier]).inc();
                }
            }
            let boot = e.boot_id;
            let dev = e.device_id.clone();
            let replay = e.data.replay;
            if replay {
                // The hold goes on before the batch is committed, so a plant is
                // never issued a dose in the window between the first replayed
                // event arriving and the edge having read the rest (SAFETY-016).
                crate::control::reconcile::begin(db, dev.as_ref(), clock.now()).await?;
            }
            let commit = retry_transient(m, || ingest::persist_replay(db, &e, at)).await?;
            // No contiguous prefix means there is nothing truthful to say, so
            // the edge stays silent and the device replays again. `Some(0)` is
            // a real acknowledgement of sequence 0 and is published normally.
            if replay
                && let Some(boot_id) = boot
                && let Some(through_device_seq) = commit.through_device_seq
            {
                let ack = Envelope {
                    v: 1,
                    kind: rhizo_mqtt_contract::MessageKind::EventAck,
                    message_id: rhizo_mqtt_contract::MessageId::from_uuid(uuid::Uuid::new_v4()),
                    device_id: dev.clone(),
                    boot_id: Some(boot_id),
                    sequence: None,
                    device_time_ms: None,
                    clock_synced: None,
                    data: rhizo_mqtt_contract::payload::EventAck {
                        boot_id,
                        through_device_seq,
                    },
                };
                client
                    .publish(
                        Topic::EventsAck(dev.clone()).as_string(),
                        rumqttc::QoS::AtLeastOnce,
                        false,
                        ack.to_json()
                            .map_err(|x| EdgeError::Decode(x.to_string()))?,
                    )
                    .await
                    .map_err(|x| EdgeError::Mqtt(x.to_string()))?;
                // The release happens only on a *committed* contiguous prefix
                // through the sender's final batch. `complete` alone is the
                // sender's framing and never releases a plant on its own.
                if commit.sender_reports_complete {
                    crate::control::reconcile::complete(
                        db,
                        dev.as_ref(),
                        &boot_id.to_string(),
                        clock.now(),
                    )
                    .await?;
                }
            } else if replay && commit.through_device_seq.is_none() {
                tracing::debug!(device=%e.device_id,"replay committed with no contiguous prefix; publishing no acknowledgement");
            }
            commit.dedup
        }
    };
    if dedup == Dedup::Duplicate {
        m.duplicate.with_label_values(&[kind]).inc()
    }
    m.duration
        .with_label_values(&[kind])
        .observe(started.elapsed().as_secs_f64());
    Ok(())
}

fn log_device_transitions(
    device_id: &str,
    transitions: &[rhizo_storage::repo::ingest::StatusTransition],
) {
    for effect in transitions {
        if effect.severity == "warning" {
            tracing::warn!(
                device_id = %device_id,
                transition = %effect.kind,
                detail = ?effect.detail,
                "device state changed"
            );
        } else {
            tracing::info!(
                device_id = %device_id,
                transition = %effect.kind,
                detail = ?effect.detail,
                "device state changed"
            );
        }
    }
}

async fn publish_edge_time(
    client: &rumqttc::AsyncClient,
    device: &rhizo_mqtt_contract::DeviceId,
    at: i64,
) -> Result<(), EdgeError> {
    let envelope = Envelope {
        v: 1,
        kind: rhizo_mqtt_contract::MessageKind::EdgeTime,
        message_id: rhizo_mqtt_contract::MessageId::from_uuid(uuid::Uuid::new_v4()),
        device_id: device.clone(),
        boot_id: None,
        sequence: None,
        device_time_ms: None,
        clock_synced: None,
        data: rhizo_mqtt_contract::payload::EdgeTime {
            edge_time_ms: rhizo_mqtt_contract::UtcMillis(at),
        },
    };
    client
        .publish(
            Topic::Time(device.clone()).as_string(),
            rumqttc::QoS::AtLeastOnce,
            false,
            envelope
                .to_json()
                .map_err(|e| EdgeError::Decode(e.to_string()))?,
        )
        .await
        .map_err(|e| EdgeError::Mqtt(e.to_string()))
}

/// Retries only what ADR-014 classifies as `Transient`, with its documented
/// 50 ms base, 500 ms cap, and three attempts.
///
/// The retry decision comes from [`Classify`] rather than from a `match` arm
/// here. A new `StorageError` variant therefore cannot be silently retried
/// forever or silently dropped — it has to be classified where it is defined.
async fn retry_transient<T, F, Fut>(m: &Metrics, mut operation: F) -> Result<T, EdgeError>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, StorageError>>,
{
    let mut backoff =
        rhizo_telemetry::Backoff::new(Duration::from_millis(50), Duration::from_millis(500));
    for attempt in 0..=3 {
        match operation().await {
            Ok(value) => return Ok(value),
            Err(error) if error.classify().is_retryable() && attempt < 3 => {
                if matches!(error, StorageError::Busy(_)) {
                    m.sqlite_busy.inc();
                }
                tokio::time::sleep(backoff.next_delay()).await;
                tracing::warn!(
                    attempt = attempt + 1,
                    error = %error,
                    "retrying a transient SQLite failure"
                );
            }
            Err(error) => return Err(error.into()),
        }
    }
    unreachable!("the bounded retry loop always returns")
}
fn decode<T: serde::de::DeserializeOwned>(
    topic: &Topic,
    payload: &[u8],
) -> Result<Envelope<T>, rhizo_mqtt_contract::DecodeError> {
    let e = Envelope::from_json(payload)?;
    e.check_topic(topic)?;
    Ok(e)
}
fn decode_status(
    topic: &Topic,
    payload: &[u8],
) -> Result<Envelope<DeviceStatus>, rhizo_mqtt_contract::DecodeError> {
    let envelope = decode::<DeviceStatus>(topic, payload)?;
    if envelope.data.boot_generation == 0 {
        return Err(rhizo_mqtt_contract::DecodeError::Payload);
    }
    if envelope.data.validate().is_err() {
        return Err(rhizo_mqtt_contract::DecodeError::Payload);
    }
    Ok(envelope)
}
enum Msg {
    Telemetry(Envelope<TelemetryBatch>),
    Actuator(Envelope<ActuatorState>),
    Events(Envelope<DeviceEventBatch>),
    Status(Envelope<DeviceStatus>),
    Result(Envelope<CommandResult>),
}
impl Msg {
    fn kind(&self) -> &'static str {
        match self {
            Self::Telemetry(_) => "telemetry.batch",
            Self::Actuator(_) => "actuator.state",
            Self::Events(_) => "device.events",
            Self::Status(_) => "device.status",
            Self::Result(_) => "command.result",
        }
    }
}

#[cfg(test)]
mod decode {
    use super::*;
    use chrono::TimeZone;
    use rhizo_testkit::TestClock;

    async fn fixture() -> (rhizo_storage::EdgeDb, rumqttc::AsyncClient, Metrics) {
        let db = rhizo_storage::EdgeDb::in_memory().await.unwrap();
        db.migrate().await.unwrap();
        let options = rumqttc::MqttOptions::new("unused-test-client", "127.0.0.1", 9);
        let (client, _eventloop) = rumqttc::AsyncClient::new(options, 4);
        (db, client, Metrics::new().unwrap())
    }

    #[test]
    fn zero_status_boot_generation_is_not_semantically_valid() {
        let topic = Topic::parse("rhizo/v1/devices/plant-node-01/status").unwrap();
        let mut value: serde_json::Value = serde_json::from_slice(include_bytes!(
            "../../../../test/fixtures/protocol/valid/status-with-capabilities.json"
        ))
        .unwrap();
        value["data"]["boot_generation"] = 0.into();
        let payload = serde_json::to_vec(&value).unwrap();
        assert!(matches!(
            decode_status(&topic, &payload),
            Err(rhizo_mqtt_contract::DecodeError::Payload)
        ));
    }

    #[tokio::test]
    async fn received_at_comes_only_from_injected_edge_clock() {
        let (db, client, metrics) = fixture().await;
        let clock = TestClock::new(chrono::Utc.timestamp_millis_opt(42_000).single().unwrap());
        let payload =
            include_bytes!("../../../../test/fixtures/protocol/valid/telemetry-batch.json");
        let mut limiter = quarantine::Limiter::default();
        process(
            &db,
            &clock,
            &client,
            None,
            &LatestSampleCache::default(),
            &metrics,
            &mut limiter,
            &Inbound {
                topic: "rhizo/v1/devices/plant-node-01/telemetry".into(),
                payload: payload.to_vec(),
                publish: None,
            },
        )
        .await
        .unwrap();
        let received: i64 = sqlx::query_scalar("SELECT min(received_at) FROM measurements")
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert_eq!(received, 42_000);
    }

    #[tokio::test]
    async fn malformed_payload_is_quarantined_and_next_message_processes() {
        let (db, client, metrics) = fixture().await;
        let clock = TestClock::new(chrono::Utc.timestamp_millis_opt(7_000).single().unwrap());
        let mut limiter = quarantine::Limiter::default();
        let topic = "rhizo/v1/devices/plant-node-01/telemetry".to_owned();
        process(
            &db,
            &clock,
            &client,
            None,
            &LatestSampleCache::default(),
            &metrics,
            &mut limiter,
            &Inbound {
                topic: topic.clone(),
                payload: b"not-json".to_vec(),
                publish: None,
            },
        )
        .await
        .unwrap();
        process(
            &db,
            &clock,
            &client,
            None,
            &LatestSampleCache::default(),
            &metrics,
            &mut limiter,
            &Inbound {
                topic,
                payload: include_bytes!(
                    "../../../../test/fixtures/protocol/valid/telemetry-batch.json"
                )
                .to_vec(),
                publish: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM quarantined_messages")
                .fetch_one(db.pool())
                .await
                .unwrap(),
            1
        );
        assert!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM measurements")
                .fetch_one(db.pool())
                .await
                .unwrap()
                > 0
        );
    }
}

#[cfg(test)]
mod persist {
    #[test]
    fn typed_samples_are_the_only_telemetry_shape() {
        let e=rhizo_mqtt_contract::Envelope::<rhizo_mqtt_contract::payload::TelemetryBatch>::from_json(include_bytes!("../../../../test/fixtures/protocol/valid/telemetry-batch.json")).unwrap();
        assert!(e.data.samples.len() > 1);
    }

    /// Application logs are sourced from the exact status transaction result,
    /// never by discovering events that another task may have inserted.
    #[tokio::test]
    async fn status_persistence_returns_only_its_committed_transitions() {
        let db = rhizo_storage::EdgeDb::in_memory().await.unwrap();
        db.migrate().await.unwrap();
        let status = rhizo_mqtt_contract::Envelope::from_json(include_bytes!(
            "../../../../test/fixtures/protocol/valid/status-with-capabilities.json"
        ))
        .unwrap();

        let result =
            rhizo_storage::repo::ingest::persist_status_with_transitions(&db, &status, 1_000)
                .await
                .unwrap();
        assert!(
            result
                .transitions
                .iter()
                .any(|effect| effect.kind == "device_registered")
        );
        assert!(
            result
                .transitions
                .iter()
                .any(|effect| effect.kind == "online")
        );

        let duplicate =
            rhizo_storage::repo::ingest::persist_status_with_transitions(&db, &status, 2_000)
                .await
                .unwrap();
        assert_eq!(
            duplicate.dedup,
            rhizo_storage::repo::ingest::Dedup::Duplicate
        );
        assert!(duplicate.transitions.is_empty());
    }
}
#[cfg(test)]
mod replay {
    #[test]
    fn replay_contract_is_typed() {
        let e=rhizo_mqtt_contract::Envelope::<rhizo_mqtt_contract::payload::DeviceEventBatch>::from_json(include_bytes!("../../../../test/fixtures/protocol/valid/events-replay-gap.json")).unwrap();
        assert!(e.data.replay && e.data.complete);
    }
}
#[cfg(test)]
mod gaps {
    #[test]
    fn gap_is_first_class_typed_detail() {
        let e=rhizo_mqtt_contract::Envelope::<rhizo_mqtt_contract::payload::DeviceEventBatch>::from_json(include_bytes!("../../../../test/fixtures/protocol/valid/events-replay-gap.json")).unwrap();
        assert!(e.data.events.iter().any(|x| matches!(
            x.detail,
            rhizo_mqtt_contract::payload::EventDetail::Gap { .. }
        )));
    }
}

#[cfg(test)]
mod classify {
    use super::*;

    fn inbound() -> Inbound {
        Inbound {
            topic: "rhizo/v1/devices/plant-node-01/telemetry".into(),
            payload: b"whatever".to_vec(),
            publish: None,
        }
    }
    async fn quarantined(db: &rhizo_storage::EdgeDb) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM quarantined_messages")
            .fetch_one(db.pool())
            .await
            .unwrap()
    }

    /// ADR-014's whole point: one unusable message must not take the process
    /// down, because every other plant's device is behind it in the queue.
    #[tokio::test]
    async fn a_permanent_failure_is_quarantined_and_the_pipeline_continues() {
        let db = rhizo_storage::EdgeDb::in_memory().await.unwrap();
        db.migrate().await.unwrap();
        let mut limiter = quarantine::Limiter::default();
        let error = EdgeError::Storage(StorageError::Constraint("bad row".into()));
        assert_eq!(error.classify(), FailureKind::Permanent);
        apply_classification(&db, &mut limiter, &inbound(), 5, &error)
            .await
            .expect("a permanent failure must not stop the pipeline");
        assert_eq!(quarantined(&db).await, 1);
    }

    #[tokio::test]
    async fn a_fatal_failure_stops_the_pipeline_without_quarantining() {
        let db = rhizo_storage::EdgeDb::in_memory().await.unwrap();
        db.migrate().await.unwrap();
        let mut limiter = quarantine::Limiter::default();
        let error = EdgeError::Storage(StorageError::Full("disk".into()));
        assert_eq!(error.classify(), FailureKind::Fatal);
        assert!(
            apply_classification(&db, &mut limiter, &inbound(), 5, &error)
                .await
                .is_err()
        );
        assert_eq!(quarantined(&db).await, 0);
    }

    /// An exhausted transient failure leaves the message unprocessed rather
    /// than quarantining it — the operation could still succeed later.
    #[tokio::test]
    async fn an_exhausted_transient_failure_is_neither_fatal_nor_quarantined() {
        let db = rhizo_storage::EdgeDb::in_memory().await.unwrap();
        db.migrate().await.unwrap();
        let mut limiter = quarantine::Limiter::default();
        let error = EdgeError::Mqtt("broker unreachable".into());
        assert_eq!(error.classify(), FailureKind::Transient);
        apply_classification(&db, &mut limiter, &inbound(), 5, &error)
            .await
            .unwrap();
        assert_eq!(quarantined(&db).await, 0);
    }

    /// The bounded retry loop honours ADR-014's three attempts and gives up on
    /// anything the classification does not call retryable.
    #[tokio::test]
    async fn only_transient_failures_are_retried() {
        let m = Metrics::new().unwrap();
        let attempts = std::cell::Cell::new(0);
        let error = retry_transient(&m, || {
            attempts.set(attempts.get() + 1);
            async { Err::<(), _>(StorageError::Busy("locked".into())) }
        })
        .await
        .unwrap_err();
        assert_eq!(error.classify(), FailureKind::Transient);
        assert_eq!(
            attempts.get(),
            4,
            "one attempt plus ADR-014's three retries"
        );

        let permanent = std::cell::Cell::new(0);
        let error = retry_transient(&m, || {
            permanent.set(permanent.get() + 1);
            async { Err::<(), _>(StorageError::Constraint("bad row".into())) }
        })
        .await
        .unwrap_err();
        assert_eq!(error.classify(), FailureKind::Permanent);
        assert_eq!(permanent.get(), 1, "a permanent failure is never retried");
    }
}
