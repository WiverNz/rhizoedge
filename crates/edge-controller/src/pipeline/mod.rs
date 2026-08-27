//! Decode, validate, and transactionally persist inbound device messages.
mod quarantine;
use crate::{metrics::Metrics, mqtt::ingress::Inbound, state::cache::LatestSampleCache};
use rhizo_domain::Clock;
use rhizo_mqtt_contract::payload::{
    ActuatorState, CommandResult, DeviceEventBatch, DeviceStatus, TelemetryBatch,
};
use rhizo_mqtt_contract::{Envelope, Topic};
use rhizo_storage::StorageError;
use rhizo_storage::repo::ingest::{self, Dedup};
use std::{future::Future, sync::Arc, time::Duration};
use tokio::sync::{mpsc, watch};

/// Owns the single logical SQLite writer path.
pub async fn run(
    mut rx: mpsc::Receiver<Inbound>,
    db: rhizo_storage::EdgeDb,
    clock: Arc<dyn Clock>,
    client: rumqttc::AsyncClient,
    cache: LatestSampleCache,
    mut shutdown: watch::Receiver<bool>,
    metrics: Metrics,
) -> Result<(), String> {
    let mut limiter = quarantine::Limiter::default();
    loop {
        tokio::select! {biased;changed=shutdown.changed()=>{if changed.is_err()||*shutdown.borrow(){return Ok(())}},item=rx.recv()=>{let Some(item)=item else{return Ok(())};process(&db,clock.as_ref(),&client,&cache,&metrics,&mut limiter,item).await?}}
    }
}

async fn process(
    db: &rhizo_storage::EdgeDb,
    clock: &dyn Clock,
    client: &rumqttc::AsyncClient,
    cache: &LatestSampleCache,
    m: &Metrics,
    limiter: &mut quarantine::Limiter,
    item: Inbound,
) -> Result<(), String> {
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
        Topic::Status(_) => decode::<DeviceStatus>(&topic, &item.payload).map(Msg::Status),
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
                .await
                .map_err(|x| x.to_string())?;
            }
            return Ok(());
        }
    };
    let kind = msg.kind();
    m.received.with_label_values(&[kind]).inc();
    let dedup = match msg {
        Msg::Telemetry(e) => {
            let (d, n) = retry_busy(m, || ingest::persist_telemetry(db, &e, at)).await?;
            if d == Dedup::New {
                for sample in rhizo_storage::repo::query::latest_samples(db)
                    .await
                    .map_err(|x| x.to_string())?
                {
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
        Msg::Actuator(e) => retry_busy(m, || ingest::persist_actuator(db, &e, at)).await?,
        Msg::Status(e) => {
            retry_busy(m, || {
                ingest::persist_raw(db, &e, rhizo_mqtt_contract::MessageKind::DeviceStatus, at)
            })
            .await?
        }
        Msg::Result(e) => retry_busy(m, || ingest::persist_command_result(db, &e, at)).await?,
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
            let commit = retry_busy(m, || ingest::persist_replay(db, &e, at)).await?;
            if replay && let Some(boot_id) = boot {
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
                        through_device_seq: commit.through_device_seq,
                    },
                };
                client
                    .publish(
                        Topic::EventsAck(dev).as_string(),
                        rumqttc::QoS::AtLeastOnce,
                        false,
                        ack.to_json().map_err(|x| x.to_string())?,
                    )
                    .await
                    .map_err(|x| x.to_string())?;
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

async fn retry_busy<T, F, Fut>(m: &Metrics, mut operation: F) -> Result<T, String>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, StorageError>>,
{
    let mut backoff =
        rhizo_telemetry::Backoff::new(Duration::from_millis(50), Duration::from_millis(200));
    for attempt in 0..=3 {
        match operation().await {
            Ok(value) => return Ok(value),
            Err(StorageError::Busy(error)) if attempt < 3 => {
                m.sqlite_busy.inc();
                tokio::time::sleep(backoff.next_delay()).await;
                tracing::warn!(
                    attempt = attempt + 1,
                    error,
                    "retrying busy SQLite transaction"
                );
            }
            Err(error) => return Err(error.to_string()),
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
            &LatestSampleCache::default(),
            &metrics,
            &mut limiter,
            Inbound {
                topic: "rhizo/v1/devices/plant-node-01/telemetry".into(),
                payload: payload.to_vec(),
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
            &LatestSampleCache::default(),
            &metrics,
            &mut limiter,
            Inbound {
                topic: topic.clone(),
                payload: b"not-json".to_vec(),
            },
        )
        .await
        .unwrap();
        process(
            &db,
            &clock,
            &client,
            &LatestSampleCache::default(),
            &metrics,
            &mut limiter,
            Inbound {
                topic,
                payload: include_bytes!(
                    "../../../../test/fixtures/protocol/valid/telemetry-batch.json"
                )
                .to_vec(),
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
