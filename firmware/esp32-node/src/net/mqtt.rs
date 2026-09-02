//! The MQTT client, its Last Will, and the eight exact subscriptions
//! (M9-009, protocol §3, §5.6).
//!
//! # The Last Will is configured before connect, and that is not a style point
//!
//! Configuring it afterwards silently does nothing, and the omission is
//! invisible until a device dies in the field. Here it is a field of the
//! configuration struct passed to the constructor, so it cannot be set late.
//!
//! # `clean_session = true` is normative
//!
//! A persistent session would have the broker queue commands for an offline
//! device and deliver them on reconnect, carrying a TTL minted before the
//! device went away — which is exactly what SAFETY-002 exists to prevent.
//! `disable_clean_session: false` is what the ESP-IDF client calls this.
//!
//! # Eight exact topics, never a wildcard
//!
//! The set comes from `Topic::device_subscriptions`, which returns all eight as
//! one value so a reconnect restores exactly these rather than whichever subset
//! a call site remembered. `commands/+` would also match `commands/result` —
//! the device's own output — and MQTT 3.1.1 has no "no local" option, so the
//! rule would have to be "receive it but never act on it", a property of the
//! dispatch code rather than of the wire.

use esp_idf_svc::mqtt::client::{
    EspMqttClient, EspMqttEvent, LwtConfiguration, MqttClientConfiguration, QoS,
};
use rhizo_mqtt_contract::{DeviceId, Topic};

/// The QoS every v1 topic uses. There is no other.
pub const QOS: QoS = QoS::AtLeastOnce;

/// Broker credentials and identity, from NVS.
pub struct BrokerSettings<'a> {
    /// `mqtt://host:1883`.
    pub url: &'a str,
    /// Client id, which is the device id.
    pub device_id: &'a DeviceId,
    /// Broker username.
    pub username: Option<&'a str>,
    /// Broker password.
    pub password: Option<&'a str>,
}

/// Builds the Last Will payload.
///
/// A Last Will declares neither a power mode nor a sleep: it is composed at
/// connect and delivered whenever the session drops, so it is evidence of an
/// absence and never a restatement of configuration. `reason:
/// "connection_lost"` is what makes an unannounced disappearance `isolated`
/// rather than `sleeping` (SAFETY-021).
#[must_use]
pub fn last_will_payload(device_id: &DeviceId, boot_generation: u64) -> String {
    let status = serde_json::json!({
        "v": 1,
        "kind": "device.status",
        "message_id": uuid::Uuid::nil(),
        "device_id": device_id.as_ref(),
        "data": {
            "boot_generation": boot_generation,
            "status": "offline",
            "reason": "connection_lost",
            "protocol_version": rhizo_mqtt_contract::PROTOCOL_VERSION,
        }
    });
    status.to_string()
}

/// Connects with the Last Will already in place, delivering events to
/// `callback`.
///
/// The callback form rather than the connection form: `EspMqttConnection`
/// requires a thread parked on `next()`, and this firmware already has a loop
/// that must stay responsive to its watchdog. The callback runs on the
/// ESP-IDF MQTT task and does the least possible work — see
/// [`crate::net::session`].
///
/// # Errors
///
/// If the client cannot be created or the URL is malformed.
pub fn connect_cb<F>(
    settings: &BrokerSettings<'_>,
    will_topic: &str,
    will_payload: &[u8],
    callback: F,
) -> Result<EspMqttClient<'static>, esp_idf_sys::EspError>
where
    F: for<'b> FnMut(EspMqttEvent<'b>) + Send + 'static,
{
    let config = MqttClientConfiguration {
        client_id: Some(settings.device_id.as_ref()),
        username: settings.username,
        password: settings.password,
        // Set here, in the constructor's configuration, so there is no moment
        // at which a connected client has no will.
        lwt: Some(LwtConfiguration {
            topic: will_topic,
            payload: will_payload,
            qos: QOS,
            retain: true,
        }),
        // Normative. Not a performance choice.
        disable_clean_session: false,
        keep_alive_interval: Some(core::time::Duration::from_secs(30)),
        ..MqttClientConfiguration::default()
    };
    EspMqttClient::new_cb(settings.url, &config, callback)
}

/// Subscribes to exactly the eight topics of protocol §3.
///
/// # Errors
///
/// If any subscription is refused. All eight or none: a device holding a
/// partial set would silently stop receiving commands, config, or time, and
/// nothing about its behaviour would say so.
pub fn subscribe(
    client: &mut EspMqttClient<'static>,
    device_id: &DeviceId,
) -> Result<(), esp_idf_sys::EspError> {
    for topic in Topic::device_subscriptions(device_id) {
        client.subscribe(&topic, QOS)?;
        log::info!("subscribed {topic}");
    }
    Ok(())
}

/// The retain flag a topic requires (protocol §3, retention rules).
///
/// Derived from the topic rather than passed in at each call site: publishing a
/// retained message on a `commands/*` topic is a protocol violation that causes
/// repeated watering, and retained telemetry is served to new subscribers as
/// though current. Neither is a decision a publisher should be able to make.
#[must_use]
pub fn retain_for(topic: &Topic) -> bool {
    matches!(
        topic,
        Topic::Status(_) | Topic::Config(_) | Topic::Policy(_)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_status_config_and_policy_are_retained() {
        let id = DeviceId::parse("plant-node-01").expect("valid");
        assert!(retain_for(&Topic::Status(id.clone())));
        assert!(!retain_for(&Topic::Telemetry(id.clone())));
        assert!(!retain_for(&Topic::CommandResult(id.clone())));
        assert!(!retain_for(&Topic::Events(id)));
    }
}
