//! Reconnecting MQTT 3.1.1 consumer with explicit re-subscription.
use crate::metrics::Metrics;
use rumqttc::{AsyncClient, Event, EventLoop, Incoming, MqttOptions, QoS};
use tokio::sync::{mpsc, watch};
/// Raw bounded ingress item.
pub struct Inbound {
    /// MQTT topic.
    pub topic: String,
    /// Payload bytes.
    pub payload: Vec<u8>,
    /// The publish this came from, kept so the PUBACK can follow the commit.
    ///
    /// **This is the M3 gap M6-010 closes.** With automatic acknowledgement,
    /// `rumqttc` PUBACKs while the message is still in the ingress channel,
    /// ahead of the transaction that persists it — and protocol §5.10 has the
    /// device stop retrying once the broker acknowledges. Between those two
    /// facts a `command.result` could be acknowledged to a device and then lost
    /// on a crash, and never republished, because as far as the device is
    /// concerned it was delivered. A lost delivered dose under-counts the
    /// SAFETY-006 budget, which is the direction that over-waters.
    ///
    /// Telemetry survives that (a lost sample is fail-safe) and offline events
    /// survive it (`event.ack` is application-level and published after commit).
    /// `command.result` had neither protection, and it is ledger data.
    ///
    /// `None` for a message a test injected directly.
    pub publish: Option<rumqttc::Publish>,
}
/// Builds clean-session Edge MQTT options.
pub fn options(
    url: &str,
    client_id: &str,
    user: &str,
    password: &str,
) -> Result<MqttOptions, String> {
    let rest = url
        .strip_prefix("mqtt://")
        .ok_or_else(|| "only mqtt:// is supported in M3".to_owned())?;
    let (host, port) = rest.split_once(':').map_or((rest, 1883), |(h, p)| {
        (h, p.trim_end_matches('/').parse().unwrap_or(1883))
    });
    let mut o = MqttOptions::new(client_id, host, port);
    o.set_clean_session(true);
    // The device's retry must stop on the **edge's durable commit**, not on the
    // broker's receipt. With manual acknowledgement the PUBACK is sent by the
    // pipeline after its transaction commits (M6-010, PRD 060 §Failure modes).
    o.set_manual_acks(true);
    o.set_keep_alive(std::time::Duration::from_secs(30));
    o.set_credentials(user, password);
    Ok(o)
}
/// Creates the client shared with the post-commit ACK publisher.
pub fn connect(options: MqttOptions, capacity: usize) -> (AsyncClient, EventLoop) {
    AsyncClient::new(options, capacity)
}
/// Polls forever, restoring all narrow device-originated subscriptions at every ConnAck.
pub async fn run(
    client: AsyncClient,
    mut events: EventLoop,
    tx: mpsc::Sender<Inbound>,
    mut shutdown: watch::Receiver<bool>,
    metrics: Metrics,
) -> Result<(), String> {
    metrics.connection.set(1);
    let mut connected_once = false;
    let mut backoff = rhizo_telemetry::backoff::Backoff::new(
        std::time::Duration::from_secs(1),
        std::time::Duration::from_secs(60),
    );
    loop {
        tokio::select! {changed=shutdown.changed()=>{if changed.is_err()||*shutdown.borrow(){let _=client.disconnect().await;return Ok(())}},ev=events.poll()=>match ev{Ok(Event::Incoming(Incoming::ConnAck(_)))=>{backoff.reset();metrics.connection.set(2);if connected_once{metrics.reconnects.inc()}connected_once=true;client.subscribe_many(rhizo_mqtt_contract::Topic::EDGE_SUBSCRIPTIONS.into_iter().map(|path|rumqttc::SubscribeFilter::new(path.to_owned(),QoS::AtLeastOnce))).await.map_err(|e|e.to_string())?;},Ok(Event::Incoming(Incoming::SubAck(_)))=>metrics.connection.set(3),Ok(Event::Incoming(Incoming::Publish(p)))=>{tx.send(Inbound{topic:p.topic.clone(),payload:p.payload.to_vec(),publish:Some(p)}).await.map_err(|_|"pipeline closed".to_owned())?},Ok(_)=>{},Err(e)=>{let delay=backoff.next_delay();tracing::warn!(error=%e,?delay,"MQTT disconnected; rumqttc will reconnect");tokio::time::sleep(delay).await;metrics.connection.set(0);}}}
    }
}
