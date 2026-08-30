//! The one seam between a decision and the wire (M6-009).
//!
//! Everything that publishes goes through [`Transport`]. In production that is
//! an MQTT client; in a test it is [`RecordingTransport`], which is how a unit
//! test asserts that a refused dose published **nothing** rather than merely
//! that it returned 409. "No message appeared on a command topic" is the
//! property SAFETY-003 and SAFETY-016 actually claim, and it has to be checked
//! by watching the wire.
//!
//! # `retain` is a parameter, and almost always `false`
//!
//! Commands are **never** retained. ADR-002 calls retaining a command topic the
//! single most damaging mistake available in this protocol: the broker would
//! redeliver it on every reconnect, for ever. Configuration and policy are
//! retained, and are the only two things that are.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

/// A publication failure. Transient by classification: the command keeps its
/// `command_id` and is retried (ADR-014).
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("publish failed: {0}")]
pub struct TransportError(pub String);

/// One published message, as a test observes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Published {
    /// The MQTT topic.
    pub topic: String,
    /// The exact bytes.
    pub payload: Vec<u8>,
    /// Whether the broker was asked to retain it.
    pub retain: bool,
}

/// The boxed future a [`Transport`] returns.
pub type PublishFuture<'a> = Pin<Box<dyn Future<Output = Result<(), TransportError>> + Send + 'a>>;

/// Somewhere to publish. QoS 1 always; the contract admits no other level.
pub trait Transport: Send + Sync {
    /// Publishes one message.
    fn publish(&self, topic: String, payload: Vec<u8>, retain: bool) -> PublishFuture<'_>;
}

/// The production transport.
#[derive(Clone)]
pub struct MqttTransport {
    client: rumqttc::AsyncClient,
}

impl MqttTransport {
    /// Wraps the shared ingress client.
    #[must_use]
    pub const fn new(client: rumqttc::AsyncClient) -> Self {
        Self { client }
    }
}

impl Transport for MqttTransport {
    fn publish(&self, topic: String, payload: Vec<u8>, retain: bool) -> PublishFuture<'_> {
        Box::pin(async move {
            self.client
                .publish(topic, rumqttc::QoS::AtLeastOnce, retain, payload)
                .await
                .map_err(|e| TransportError(e.to_string()))
        })
    }
}

/// A transport that records what it was asked to publish, and can be told to
/// fail the next `n` attempts.
///
/// The failure counter is what makes M6-011's key assertion testable without a
/// broker: fail twice, succeed on the third, and check the device would have
/// seen **one** `command_id`.
#[derive(Clone, Default)]
pub struct RecordingTransport {
    inner: Arc<Mutex<Recorded>>,
}

#[derive(Default)]
struct Recorded {
    published: Vec<Published>,
    fail_next: usize,
    attempts: usize,
}

impl RecordingTransport {
    /// A transport that always succeeds.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Fails the next `n` publish attempts, then succeeds.
    pub fn fail_next(&self, n: usize) {
        self.lock().fail_next = n;
    }

    /// Everything published so far, in order.
    #[must_use]
    pub fn published(&self) -> Vec<Published> {
        self.lock().published.clone()
    }

    /// Everything published to a command topic.
    #[must_use]
    pub fn commands(&self) -> Vec<Published> {
        self.published()
            .into_iter()
            .filter(|m| m.topic.contains("/commands/"))
            .collect()
    }

    /// How many publish attempts were made, successful or not.
    #[must_use]
    pub fn attempts(&self) -> usize {
        self.lock().attempts
    }

    /// Forgets everything recorded so far.
    pub fn clear(&self) {
        let mut guard = self.lock();
        guard.published.clear();
        guard.attempts = 0;
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Recorded> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }
}

impl Transport for RecordingTransport {
    fn publish(&self, topic: String, payload: Vec<u8>, retain: bool) -> PublishFuture<'_> {
        Box::pin(async move {
            let mut guard = self.lock();
            guard.attempts += 1;
            if guard.fail_next > 0 {
                guard.fail_next -= 1;
                return Err(TransportError("injected publish failure".to_owned()));
            }
            guard.published.push(Published {
                topic,
                payload,
                retain,
            });
            Ok(())
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
#[allow(
    clippy::module_inception,
    reason = "the module name is the verification filter the issue quotes literally"
)]
mod transport {
    use super::*;

    #[tokio::test]
    async fn the_recorder_keeps_order_and_flags() {
        let transport = RecordingTransport::new();
        transport
            .publish("a/commands/water".into(), b"one".to_vec(), false)
            .await
            .unwrap();
        transport
            .publish("a/config".into(), b"two".to_vec(), true)
            .await
            .unwrap();
        let published = transport.published();
        assert_eq!(published.len(), 2);
        assert_eq!(published[0].payload, b"one");
        assert!(!published[0].retain);
        assert!(published[1].retain);
        assert_eq!(transport.commands().len(), 1);
    }

    #[tokio::test]
    async fn injected_failures_are_consumed_one_at_a_time() {
        let transport = RecordingTransport::new();
        transport.fail_next(2);
        assert!(
            transport
                .publish("t".into(), Vec::new(), false)
                .await
                .is_err()
        );
        assert!(
            transport
                .publish("t".into(), Vec::new(), false)
                .await
                .is_err()
        );
        assert!(
            transport
                .publish("t".into(), Vec::new(), false)
                .await
                .is_ok()
        );
        assert_eq!(transport.attempts(), 3);
        assert_eq!(transport.published().len(), 1);
    }
}
