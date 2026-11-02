//! Inspection of a drained MQTT batch during the sleep handshake.
use super::inbound::Inbound;
/// Matching ACK and messages that still need the ordinary dispatcher.
pub struct AckBatch {
    /// The sleep publication was acknowledged.
    pub acknowledged: bool,
    /// Unconsumed input, in arrival order.
    pub pending: Vec<Inbound>,
}
/// Classify events without losing application input.
pub fn classify(expected: u32, events: Vec<Inbound>) -> AckBatch {
    let mut result = AckBatch {
        acknowledged: false,
        pending: Vec::new(),
    };
    for event in events {
        match event {
            Inbound::PublishAcknowledged(id) => result.acknowledged |= id == expected,
            other => result.pending.push(other),
        }
    }
    result
}
