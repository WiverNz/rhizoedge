//! The MQTT driver.
//!
//! Thin by design: it owns the socket and nothing else. Every decision about
//! *what* to publish belongs to [`Device`], which is testable without a broker;
//! this module only moves bytes and manages the connection lifecycle.
//!
//! # Three rules that are easy to get wrong and invisible when wrong
//!
//! 1. **`clean_session = true`** (ADR-002, protocol §1). A persistent session
//!    would have the broker queue water commands for an offline device and
//!    deliver the backlog on reconnect — exactly what SAFETY-002 exists to
//!    prevent.
//! 2. **The will is configured before connecting.** Setting it on the client
//!    afterwards silently does nothing.
//! 3. **Subscriptions are re-established on every reconnect**, never assumed to
//!    survive one.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use rhizo_mqtt_contract::Topic;
use rhizo_telemetry::Backoff;
use rumqttc::{AsyncClient, ConnectionError, Event, EventLoop, LastWill, MqttOptions, Packet, QoS};

use crate::capture::FixtureCapture;
use crate::cli::Cli;
use crate::device::Device;
use crate::envelope::Publication;
use crate::fault::PublicationPipeline;
use crate::rng::SplitMix64;

/// Device keepalive (protocol §1). The edge uses 30 s; a device 60 s.
pub const DEVICE_KEEPALIVE: Duration = Duration::from_secs(60);
/// Reconnect backoff base for a device (ADR-014 §Backoff).
pub const RECONNECT_BASE: Duration = Duration::from_secs(2);
/// Reconnect backoff cap for a device. Retries are unlimited.
pub const RECONNECT_CAP: Duration = Duration::from_secs(300);
/// How long a clean shutdown waits for its DISCONNECT to reach the broker.
const SHUTDOWN_DRAIN: Duration = Duration::from_secs(5);
/// Outstanding-request capacity of the client channel.
const CHANNEL_CAPACITY: usize = 32;

/// A broker host and port.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BrokerAddress {
    /// Hostname or address.
    pub host: String,
    /// TCP port.
    pub port: u16,
}

/// Why the broker could not be addressed or reached.
#[derive(Debug, thiserror::Error)]
pub enum MqttError {
    /// `--broker` was not a `mqtt://host[:port]` URL.
    #[error("`{0}` is not a broker URL; expected mqtt://host:port")]
    BrokerUrl(String),
    /// `--broker` named a scheme this build cannot speak.
    #[error("broker scheme `{0}` is not supported; V1 is plaintext MQTT (TLS is M13)")]
    BrokerScheme(String),
    /// The port was not a number in range.
    #[error("`{0}` is not a valid TCP port")]
    BrokerPort(String),
    /// A payload could not be serialised.
    #[error("could not serialise a payload: {0}")]
    Encode(#[from] serde_json::Error),
    /// The client task ended.
    #[error("the MQTT client stopped accepting requests: {0}")]
    Client(#[from] rumqttc::ClientError),
}

/// Parses `mqtt://host[:port]`.
///
/// Written out rather than pulled from a URL crate: the accepted grammar is one
/// scheme and an optional port, and a general parser would accept forms
/// (`mqtts://`, credentials in the authority) that this build cannot honour and
/// would then ignore silently.
///
/// # Errors
///
/// Returns the specific reason the URL was rejected.
pub fn parse_broker_url(url: &str) -> Result<BrokerAddress, MqttError> {
    let (scheme, authority) = url
        .split_once("://")
        .ok_or_else(|| MqttError::BrokerUrl(url.to_owned()))?;
    if scheme != "mqtt" {
        return Err(MqttError::BrokerScheme(scheme.to_owned()));
    }
    let authority = authority.trim_end_matches('/');
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) => (
            host,
            port.parse()
                .map_err(|_| MqttError::BrokerPort(port.to_owned()))?,
        ),
        None => (authority, 1883),
    };
    if host.is_empty() {
        return Err(MqttError::BrokerUrl(url.to_owned()));
    }
    Ok(BrokerAddress {
        host: host.to_owned(),
        port,
    })
}

/// Builds the connection options, including the will.
///
/// # Errors
///
/// Returns a broker-URL failure.
pub fn options(cli: &Cli, will: &Publication) -> Result<MqttOptions, MqttError> {
    let address = parse_broker_url(&cli.broker)?;
    // Client id MUST equal the device_id (protocol §1).
    let mut options = MqttOptions::new(cli.device_id.to_string(), address.host, address.port);
    options.set_keep_alive(DEVICE_KEEPALIVE);
    // Normative, and the reason a queued command cannot ambush a device.
    options.set_clean_session(true);
    if let Some(password) = cli.resolved_password() {
        options.set_credentials(cli.resolved_username(), password);
    }
    options.set_last_will(LastWill::new(
        will.topic_string(),
        will.payload.clone(),
        QoS::AtLeastOnce,
        will.retain,
    ));
    Ok(options)
}

/// What one poll of the connection did.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Step {
    /// The broker acknowledged a connection; subscriptions were re-established
    /// and the retained online status published.
    Connected,
    /// A message arrived on one of the device's subscriptions and is the
    /// device's to act on.
    Inbound {
        /// The topic it arrived on.
        topic: String,
    },
    /// A message arrived that the device must not act on: a topic whose
    /// grammar is invalid, one addressed to another device, or — if a
    /// subscription were ever widened by mistake — one the device publishes.
    Ignored {
        /// The topic it arrived on.
        topic: String,
        /// Why it was ignored.
        reason: &'static str,
    },
    /// Protocol traffic with no device-visible effect.
    Idle,
    /// The connection failed; the caller waits before polling again.
    Disconnected {
        /// Full-jitter backoff delay.
        retry_in: Duration,
    },
}

/// Drives one device's broker connection.
pub struct Connection {
    device: Arc<Mutex<Device>>,
    client: AsyncClient,
    eventloop: EventLoop,
    backoff: Backoff,
    /// Records one example of every kind published, for `--capture-fixtures`.
    ///
    /// Here rather than in the run loop because *every* publication passes
    /// through this type — including the ones produced inside a connect or an
    /// inbound message, which the run loop never sees. A capture that missed
    /// those would silently cover only the periodic kinds.
    capture: Option<FixtureCapture>,
    /// Applies the transport faults — `duplicate` and `reorder` — to everything
    /// that goes out. Here rather than in the device core because they are
    /// properties of the link, not of the device's decisions.
    pipeline: PublicationPipeline,
    /// The generator the transport faults draw from.
    fault_rng: SplitMix64,
}

impl Connection {
    /// Creates a client with the will already configured.
    ///
    /// # Errors
    ///
    /// Returns a broker-URL or serialisation failure.
    pub fn new(cli: &Cli, device: Arc<Mutex<Device>>) -> Result<Self, MqttError> {
        let will = lock(&device).will()?;
        let (client, eventloop) = AsyncClient::new(options(cli, &will)?, CHANNEL_CAPACITY);
        Ok(Self {
            device,
            client,
            eventloop,
            backoff: Backoff::new(RECONNECT_BASE, RECONNECT_CAP),
            capture: cli.capture_fixtures.as_ref().map(|_| FixtureCapture::new()),
            pipeline: PublicationPipeline::new(),
            // A distinct stream from the model's, so enabling a transport fault
            // does not shift the sensor noise and change every reading.
            fault_rng: SplitMix64::new(cli.seed ^ 0x7A17_0000_0000_0001),
        })
    }

    /// The captured examples, if `--capture-fixtures` was given.
    #[must_use]
    pub const fn capture(&self) -> Option<&FixtureCapture> {
        self.capture.as_ref()
    }

    /// A handle for publishing outside the poll loop.
    #[must_use]
    pub fn client(&self) -> AsyncClient {
        self.client.clone()
    }

    /// Polls once, handling whatever the broker had to say.
    ///
    /// The outcome is returned rather than only logged so a test can assert on
    /// it. In particular [`Step::Inbound`] is what makes "the four
    /// subscriptions really were restored after a reconnect" an observable fact
    /// rather than an inference from the code.
    pub async fn step(&mut self) -> Step {
        match self.eventloop.poll().await {
            Ok(Event::Incoming(Packet::ConnAck(_))) => {
                self.backoff.reset();
                self.on_connected().await;
                Step::Connected
            }
            Ok(Event::Incoming(Packet::Publish(publish))) => {
                let step = self.on_inbound(publish.topic);
                if let Step::Inbound { topic } = &step {
                    // Unwrap-free: `on_inbound` only returns `Inbound` for a
                    // topic it already parsed.
                    if let Ok(parsed) = topic.parse::<Topic>() {
                        let publications = lock(&self.device).on_message(&parsed, &publish.payload);
                        self.publish_all(&publications).await;
                    }
                }
                step
            }
            Ok(_) => Step::Idle,
            Err(e) => {
                let delay = self.backoff.next_delay();
                self.on_connection_error(&e, delay);
                Step::Disconnected { retry_in: delay }
            }
        }
    }

    /// Classifies an inbound message.
    ///
    /// Since the device subscribes to exact topics rather than `commands/+`,
    /// nothing it publishes can be delivered here. Anything on such a topic
    /// therefore means a subscription was widened somewhere, which is worth
    /// saying loudly rather than quietly dropping.
    fn on_inbound(&self, topic: String) -> Step {
        let ignored = |reason| Step::Ignored {
            topic: topic.clone(),
            reason,
        };
        let Ok(parsed) = topic.parse::<Topic>() else {
            tracing::warn!(topic, "inbound message on an invalid topic");
            return ignored("invalid_topic");
        };
        if *parsed.device_id() != self.device_id() {
            // The broker's `%u` ACL already prevents this; refusing rather than
            // guessing is what protocol §4 requires if it ever does not.
            tracing::warn!(topic, "inbound message addressed to another device");
            return ignored("device_mismatch");
        }
        match parsed {
            Topic::Config(_) | Topic::Policy(_) | Topic::Time(_) => {}
            Topic::CommandWater(_) | Topic::CommandTare(_) | Topic::CommandCalibrate(_) => {}
            Topic::EventsAck(_) => {}
            Topic::CommandResult(_)
            | Topic::Telemetry(_)
            | Topic::Actuator(_)
            | Topic::Events(_)
            | Topic::Status(_) => {
                // Not subscribed to, so reaching here means an over-broad
                // subscription somewhere — worth saying so loudly.
                tracing::warn!(topic, "inbound message on a device-published topic");
                return ignored("published_by_this_device");
            }
        }
        tracing::debug!(topic, "inbound message");
        Step::Inbound { topic }
    }

    /// The device identity, for topic checks.
    fn device_id(&self) -> rhizo_mqtt_contract::DeviceId {
        lock(&self.device).device_id().clone()
    }

    /// Re-establishes subscriptions and announces presence.
    ///
    /// Subscriptions come from
    /// [`rhizo_mqtt_contract::Topic::device_subscriptions`] every time, so a
    /// reconnect restores exactly the normative four rather than whichever
    /// subset happened to be remembered.
    async fn on_connected(&mut self) {
        let (subscriptions, publications) = {
            let mut device = lock(&self.device);
            let subscriptions = device.subscriptions();
            match device.on_connected() {
                Ok(publications) => (subscriptions, publications),
                Err(e) => {
                    tracing::error!(error = %e, "could not build the online status");
                    (subscriptions, Vec::new())
                }
            }
        };
        for filter in &subscriptions {
            if let Err(e) = self.client.subscribe(filter, QoS::AtLeastOnce).await {
                tracing::error!(filter, error = %e, "subscribe failed");
            }
        }
        tracing::info!(
            subscriptions = subscriptions.len(),
            "connected; subscriptions re-established"
        );
        self.publish_all(&publications).await;
    }

    fn on_connection_error(&self, e: &ConnectionError, delay: Duration) {
        lock(&self.device).on_disconnected();
        // A refused connection is logged at ERROR because it will not fix
        // itself; an unreachable broker is a WARN because it usually does.
        match e {
            ConnectionError::ConnectionRefused(code) => {
                tracing::error!(
                    ?code,
                    retry_in_ms = delay.as_millis() as u64,
                    "broker refused the connection"
                );
            }
            other => {
                tracing::warn!(error = %other, retry_in_ms = delay.as_millis() as u64, "disconnected");
            }
        }
    }

    /// Publishes a batch, logging rather than failing on a full channel.
    pub async fn publish_all(&mut self, publications: &[Publication]) {
        if let Some(capture) = self.capture.as_mut() {
            for p in publications {
                capture.offer(p);
            }
        }
        let faults = lock(&self.device).faults().clone();
        let publications =
            self.pipeline
                .process(&faults, &mut self.fault_rng, publications.to_vec());
        for p in &publications {
            let topic = p.topic_string();
            if let Err(e) = self
                .client
                .publish(&topic, p.qos_level(), p.retain, p.payload.clone())
                .await
            {
                tracing::error!(topic, error = %e, "publish failed");
            }
        }
    }

    /// Publishes the shutdown status and disconnects cleanly.
    pub async fn shutdown(&mut self) {
        let publications = match lock(&self.device).on_shutdown() {
            Ok(p) => p,
            Err(e) => {
                tracing::error!(error = %e, "could not build the shutdown status");
                Vec::new()
            }
        };
        self.publish_all(&publications).await;
        // Release anything `reorder` was holding: the fault reorders, it does
        // not drop, and a shutdown must not turn it into a silent loss.
        let held = self.pipeline.flush();
        self.publish_all(&held).await;
        if let Err(e) = self.client.disconnect().await {
            tracing::debug!(error = %e, "disconnect was not queued; the socket is closing anyway");
        }
        // Keep polling until the DISCONNECT has actually gone out.
        //
        // `publish` and `disconnect` only *queue* requests; the event loop
        // writes them. Returning early drops the socket, the broker sees an
        // unclean close, and it publishes the will — overwriting the retained
        // `shutdown` status with `connection_lost` and making a deliberate stop
        // indistinguishable from a crash. That is a real failure this drain
        // exists to prevent, not tidiness.
        let deadline = tokio::time::Instant::now() + SHUTDOWN_DRAIN;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                tracing::warn!("timed out draining the shutdown publish");
                return;
            }
            match tokio::time::timeout(remaining, self.eventloop.poll()).await {
                Ok(Ok(Event::Outgoing(rumqttc::Outgoing::Disconnect))) | Ok(Err(_)) => return,
                Ok(Ok(_)) => {}
                Err(_) => {
                    tracing::warn!("timed out draining the shutdown publish");
                    return;
                }
            }
        }
    }
}

impl Publication {
    /// The rumqttc spelling of the contract's QoS.
    #[must_use]
    pub const fn qos_level(&self) -> QoS {
        match self.qos {
            rhizo_mqtt_contract::Qos::One => QoS::AtLeastOnce,
        }
    }
}

/// Takes the device lock, recovering from poisoning.
///
/// A panic in one task must not disable the device for every other one; the
/// guarded value is a single struct that a panic cannot leave torn.
pub fn lock(device: &Arc<Mutex<Device>>) -> std::sync::MutexGuard<'_, Device> {
    device
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::cli as test_cli;

    #[test]
    fn broker_urls_parse_with_and_without_a_port() {
        assert_eq!(
            parse_broker_url("mqtt://localhost:1883").unwrap(),
            BrokerAddress {
                host: "localhost".into(),
                port: 1883
            }
        );
        assert_eq!(
            parse_broker_url("mqtt://mosquitto").unwrap(),
            BrokerAddress {
                host: "mosquitto".into(),
                port: 1883
            }
        );
    }

    #[test]
    fn unsupported_and_malformed_broker_urls_are_rejected_with_a_reason() {
        assert!(matches!(
            parse_broker_url("mqtts://localhost:8883"),
            Err(MqttError::BrokerScheme(_))
        ));
        assert!(matches!(
            parse_broker_url("localhost:1883"),
            Err(MqttError::BrokerUrl(_))
        ));
        assert!(matches!(
            parse_broker_url("mqtt://localhost:not-a-port"),
            Err(MqttError::BrokerPort(_))
        ));
        assert!(matches!(
            parse_broker_url("mqtt://"),
            Err(MqttError::BrokerUrl(_))
        ));
    }

    #[test]
    fn options_are_the_normative_ones() {
        let cli = test_cli(&["--password", "secret"]);
        let mut device = Device::new(&cli);
        let will = device.will().unwrap();
        let options = options(&cli, &will).unwrap();
        assert_eq!(options.client_id(), "plant-node-01");
        assert!(
            options.clean_session(),
            "a persistent session would queue water commands for an offline device"
        );
        assert_eq!(options.keep_alive(), DEVICE_KEEPALIVE);
        let last_will = options.last_will().unwrap();
        assert_eq!(last_will.topic, "rhizo/v1/devices/plant-node-01/status");
        assert_eq!(last_will.qos, QoS::AtLeastOnce);
        assert!(last_will.retain, "the will is retained (protocol §5.6)");
        assert!(String::from_utf8_lossy(&last_will.message).contains("connection_lost"));
    }

    #[test]
    fn credentials_default_to_the_device_id_as_username() {
        let cli = test_cli(&["--password", "secret"]);
        assert_eq!(cli.resolved_username(), "plant-node-01");
    }

    #[test]
    fn the_backoff_matches_the_device_row_of_the_retry_table() {
        assert_eq!(RECONNECT_BASE, Duration::from_secs(2));
        assert_eq!(RECONNECT_CAP, Duration::from_secs(300));
    }
}
