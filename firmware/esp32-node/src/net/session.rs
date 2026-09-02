//! The MQTT session as a channel of decoded events.
//!
//! `EspMqttClient`'s callback runs on the ESP-IDF MQTT task, so it cannot hold
//! the device state — everything it touches would need a lock, and a lock held
//! across a publish is a way to stall the broker task. Instead the callback
//! does the least possible work: it copies the event into an owned value and
//! sends it down a channel, and the main loop owns the state.
//!
//! The copy is deliberate. `EventPayload::Received` borrows a buffer the MQTT
//! task reuses as soon as the callback returns, so anything kept must be owned
//! before then.

use std::sync::mpsc::{channel, Receiver, TryRecvError};

use esp_idf_svc::mqtt::client::{EspMqttClient, EventPayload};

use rhizo_mqtt_contract::{DeviceId, Topic};

use crate::net::mqtt::{connect_cb, last_will_payload, subscribe, BrokerSettings};

/// One thing that happened on the connection.
#[derive(Debug)]
pub enum Inbound {
    /// The broker accepted the connection.
    Connected,
    /// The session dropped.
    Disconnected,
    /// A publish arrived on a topic the device subscribed to.
    Message {
        /// The parsed topic, or `None` when it is not a v1 topic at all.
        topic: Option<Topic>,
        /// The payload, owned.
        payload: Vec<u8>,
    },
    /// The broker acknowledged one of our publishes.
    ///
    /// **Not delivery.** QoS 1 is hop by hop; this says the broker has the
    /// message, not that the edge committed it. Only `command.result.ack`
    /// retires a result, and only `event.ack` retires buffered history.
    PublishAcknowledged(u32),
}

/// A connected session.
pub struct Session {
    client: EspMqttClient<'static>,
    events: Receiver<Inbound>,
}

impl Session {
    /// Connects, subscribes to the eight exact topics, and returns the session.
    ///
    /// The Last Will is composed here and passed to the constructor, so there
    /// is no window in which a connected client has no will.
    ///
    /// # Errors
    ///
    /// If the client cannot be created or a subscription is refused.
    pub fn open(
        settings: &BrokerSettings<'_>,
        boot_generation: u64,
    ) -> Result<Self, esp_idf_sys::EspError> {
        let will_topic = Topic::Status(settings.device_id.clone()).as_string();
        let will_payload = last_will_payload(settings.device_id, boot_generation);

        let (tx, events) = channel();
        let own = settings.device_id.clone();
        let mut client = connect_cb(
            settings,
            &will_topic,
            will_payload.as_bytes(),
            move |event| {
                // The least possible work on the MQTT task: decode, copy, send.
                // `EventPayload::Received` borrows a buffer the task reuses the
                // moment this returns, so anything kept is owned first.
                if let Some(inbound) = to_inbound(&event.payload(), &own) {
                    let _ = tx.send(inbound);
                }
            },
        )?;
        subscribe(&mut client, settings.device_id)?;
        Ok(Self { client, events })
    }

    /// The client, for publishing.
    pub fn client(&mut self) -> &mut EspMqttClient<'static> {
        &mut self.client
    }

    /// Everything that has happened since the last call.
    ///
    /// Non-blocking: the wake loop has telemetry to sample and a watchdog to
    /// feed, and must not park on the broker.
    pub fn drain(&self) -> Vec<Inbound> {
        let mut out = Vec::new();
        loop {
            match self.events.try_recv() {
                Ok(event) => out.push(event),
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => return out,
            }
        }
    }
}

/// Parses a received topic, keeping the failure visible.
///
/// A topic the device cannot parse is logged and ignored rather than guessed
/// at: the eight subscriptions are exact, so anything else arriving is either a
/// broker misconfiguration or a protocol version this build does not speak, and
/// neither is a reason to act.
#[must_use]
pub fn parse_topic(raw: Option<&str>, own: &DeviceId) -> Option<Topic> {
    let raw = raw?;
    match Topic::parse(raw) {
        Ok(topic) if topic.device_id() == own => Some(topic),
        Ok(topic) => {
            log::warn!("ignoring {raw}: addressed to {}", topic.device_id());
            None
        }
        Err(error) => {
            log::warn!("ignoring unparseable topic {raw}: {error:?}");
            None
        }
    }
}

/// Turns one callback event into an [`Inbound`], copying anything borrowed.
#[must_use]
pub fn to_inbound(
    payload: &EventPayload<'_, esp_idf_sys::EspError>,
    own: &DeviceId,
) -> Option<Inbound> {
    match payload {
        EventPayload::Connected(_) => Some(Inbound::Connected),
        EventPayload::Disconnected => Some(Inbound::Disconnected),
        EventPayload::Received { topic, data, .. } => Some(Inbound::Message {
            topic: parse_topic(*topic, own),
            payload: data.to_vec(),
        }),
        EventPayload::Published(id) => Some(Inbound::PublishAcknowledged(*id)),
        EventPayload::BeforeConnect
        | EventPayload::Subscribed(_)
        | EventPayload::Unsubscribed(_)
        | EventPayload::Deleted(_)
        | EventPayload::Error(_) => None,
    }
}
