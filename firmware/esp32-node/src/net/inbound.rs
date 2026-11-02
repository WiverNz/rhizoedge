//! Owned MQTT events shared by the session and sleep handshake.
use rhizo_mqtt_contract::Topic;

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
