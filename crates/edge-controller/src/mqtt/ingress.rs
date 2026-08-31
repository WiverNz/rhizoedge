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
    /// **This is the M3 gap M6-010 closes, and it is only half the story.**
    /// With automatic acknowledgement, `rumqttc` PUBACKs while the message is
    /// still in the ingress channel, ahead of the transaction that persists it.
    /// Deferring the PUBACK until after the commit is what makes the *broker*
    /// redeliver a message this edge accepted but did not finish — which is
    /// worth having, and is why manual acks stay.
    ///
    /// What it does **not** do — and what the original M6-010 note claimed it
    /// did — is make a device's retry depend on this edge's durable commit.
    /// MQTT 3.1.1 QoS 1 acknowledges **hop by hop**: the PUBACK a device
    /// receives for its `command.result` was written by the broker, on receipt,
    /// long before this edge saw the bytes. Nothing this edge does to its own
    /// PUBACK can reach back through the broker and change that.
    ///
    /// The gap that leaves is real: this session is clean
    /// (`set_clean_session(true)`), so a message unacknowledged when the edge
    /// dies is discarded by the broker rather than redelivered, and the device
    /// — already PUBACKed — never sends it again. A lost delivered dose
    /// under-counts the SAFETY-006 budget, which is the direction that
    /// over-waters.
    ///
    /// That is closed at the application level instead, by
    /// `command.result.ack` (protocol §5.14), published after the commit and
    /// retried against by the device until it arrives. Turning the session
    /// persistent would not have been a substitute: it moves durability into
    /// the broker, where a broker restart with `persistence false`, a queue
    /// limit, or a session expiry loses it again, and it still says nothing
    /// about whether *this* process committed.
    ///
    /// Telemetry is deliberately left as it is (a lost sample is fail-safe) and
    /// offline events already had `event.ack`.
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
    // Clean, on purpose. A persistent session would make the broker the keeper
    // of undelivered ledger data, and the broker is the one participant this
    // edge has no durability contract with. Device-to-edge durability is
    // carried by `event.ack` and `command.result.ack` instead, which survive a
    // broker restart, a broker replacement, and a queue overflow alike.
    o.set_clean_session(true);
    // Manual acknowledgement so the PUBACK follows the commit: a message this
    // edge accepted but did not finish is redelivered by the broker rather than
    // dropped (M6-010, PRD 060 §Failure modes). This is a guarantee about the
    // broker-to-edge hop only -- see `Inbound::publish` for why the device's own
    // retry needs an application-level acknowledgement as well.
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
