//! Generic JSON message envelope.
use crate::{BootId, DeviceId, MessageId, PROTOCOL_VERSION, Topic, UtcMillis};
use alloc::string::String;
use core::fmt;
use serde::{Deserialize, Serialize, de::DeserializeOwned};

/// Payload discriminator.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum MessageKind {
    #[serde(rename = "telemetry.batch")]
    TelemetryBatch,
    #[serde(rename = "actuator.state")]
    ActuatorState,
    #[serde(rename = "device.events")]
    DeviceEvents,
    #[serde(rename = "device.status")]
    DeviceStatus,
    #[serde(rename = "device.config")]
    DeviceConfig,
    #[serde(rename = "device.policy")]
    DevicePolicy,
    #[serde(rename = "edge.time")]
    EdgeTime,
    #[serde(rename = "command.water")]
    CommandWater,
    #[serde(rename = "command.tare")]
    CommandTare,
    #[serde(rename = "command.calibrate")]
    CommandCalibrate,
    #[serde(rename = "command.result")]
    CommandResult,
    #[serde(rename = "command.result.ack")]
    CommandResultAck,
    #[serde(rename = "event.ack")]
    EventAck,
}
impl MessageKind {
    /// Expected discriminator for a topic.
    pub const fn for_topic(topic: &Topic) -> Self {
        match topic {
            Topic::Telemetry(_) => Self::TelemetryBatch,
            Topic::Actuator(_) => Self::ActuatorState,
            Topic::Events(_) => Self::DeviceEvents,
            Topic::Status(_) => Self::DeviceStatus,
            Topic::Config(_) => Self::DeviceConfig,
            Topic::Policy(_) => Self::DevicePolicy,
            Topic::Time(_) => Self::EdgeTime,
            Topic::CommandWater(_) => Self::CommandWater,
            Topic::CommandTare(_) => Self::CommandTare,
            Topic::CommandCalibrate(_) => Self::CommandCalibrate,
            Topic::CommandResult(_) => Self::CommandResult,
            Topic::CommandResultAck(_) => Self::CommandResultAck,
            Topic::EventsAck(_) => Self::EventAck,
        }
    }
}

/// Envelope carried on every MQTT topic.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Envelope<T> {
    /// Protocol version.
    pub v: u16,
    /// Payload discriminator.
    pub kind: MessageKind,
    /// Global deduplication key.
    pub message_id: MessageId,
    /// Device identity duplicated from the topic.
    pub device_id: DeviceId,
    /// Device boot identity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub boot_id: Option<BootId>,
    /// Per-boot sequence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sequence: Option<u64>,
    /// Advisory device wall time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_time_ms: Option<UtcMillis>,
    /// Device synchronization state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clock_synced: Option<bool>,
    /// Kind-specific payload.
    pub data: T,
}

/// Encoding failure.
#[derive(Debug)]
pub struct EncodeError(serde_json::Error);
/// Typed decode or identity failure.
#[derive(Debug)]
pub enum DecodeError {
    /// Invalid JSON or field representation.
    Json(serde_json::Error),
    /// Unsupported protocol version.
    UnsupportedVersion,
    /// Topic and payload identities differ.
    DeviceMismatch,
    /// Topic and payload kinds differ.
    KindMismatch,
    /// Required envelope field absent.
    Envelope,
    /// Kind-specific semantic validation failed.
    Payload,
}
impl DecodeError {
    /// Stable metric reason label.
    pub const fn metric_reason(&self) -> &'static str {
        match self {
            Self::Json(_) => "json",
            Self::UnsupportedVersion => "version",
            Self::DeviceMismatch => "device_mismatch",
            Self::KindMismatch => "kind_mismatch",
            Self::Envelope => "envelope",
            Self::Payload => "payload",
        }
    }
}
impl<T: Serialize> Envelope<T> {
    /// Serializes as JSON.
    pub fn to_json(&self) -> Result<String, EncodeError> {
        serde_json::to_string(self).map_err(EncodeError)
    }
}
impl<T: DeserializeOwned> Envelope<T> {
    /// Decodes JSON and checks version.
    pub fn from_json(bytes: &[u8]) -> Result<Self, DecodeError> {
        let value: serde_json::Value = serde_json::from_slice(bytes).map_err(DecodeError::Json)?;
        for required in ["v", "kind", "message_id", "device_id", "data"] {
            if value.get(required).is_none() {
                return Err(DecodeError::Envelope);
            }
        }
        let envelope: Self = serde_json::from_value(value).map_err(DecodeError::Json)?;
        if envelope.v != PROTOCOL_VERSION {
            return Err(DecodeError::UnsupportedVersion);
        }
        Ok(envelope)
    }
}
impl<T> Envelope<T> {
    /// Checks identity and kind against a parsed topic.
    pub fn check_topic(&self, topic: &Topic) -> Result<(), DecodeError> {
        if &self.device_id != topic.device_id() {
            return Err(DecodeError::DeviceMismatch);
        }
        if self.kind != MessageKind::for_topic(topic) {
            return Err(DecodeError::KindMismatch);
        }
        Ok(())
    }
    /// Checks only the duplicated device identity.
    pub fn check_identity(&self, id: &DeviceId) -> Result<(), DecodeError> {
        if &self.device_id == id {
            Ok(())
        } else {
            Err(DecodeError::DeviceMismatch)
        }
    }
}
impl fmt::Display for EncodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "MQTT decode error: {}", self.metric_reason())
    }
}
#[cfg(feature = "std")]
impl std::error::Error for EncodeError {}
#[cfg(feature = "std")]
impl std::error::Error for DecodeError {}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;
    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
    struct Data {
        value: u8,
    }
    fn envelope() -> Envelope<Data> {
        Envelope {
            v: 1,
            kind: MessageKind::TelemetryBatch,
            message_id: MessageId::from_uuid(Uuid::nil()),
            device_id: DeviceId::parse("node-01").unwrap(),
            boot_id: Some(BootId::from_uuid(Uuid::nil())),
            sequence: Some(1),
            device_time_ms: None,
            clock_synced: Some(false),
            data: Data { value: 2 },
        }
    }
    #[test]
    fn round_trip_unknown_field() {
        let json = envelope()
            .to_json()
            .unwrap()
            .replace("\"data\"", "\"future\":true,\"data\"");
        assert_eq!(
            Envelope::<Data>::from_json(json.as_bytes()).unwrap(),
            envelope()
        );
    }
    #[test]
    fn exact_rejections() {
        let mut e = envelope();
        e.v = 2;
        assert!(matches!(
            Envelope::<Data>::from_json(e.to_json().unwrap().as_bytes()),
            Err(DecodeError::UnsupportedVersion)
        ));
        assert!(matches!(
            envelope().check_topic(&Topic::Telemetry(DeviceId::parse("other-01").unwrap())),
            Err(DecodeError::DeviceMismatch)
        ));
        assert!(matches!(
            envelope().check_topic(&Topic::Status(DeviceId::parse("node-01").unwrap())),
            Err(DecodeError::KindMismatch)
        ));
        assert!(matches!(
            Envelope::<Data>::from_json(br#"{"v":1}"#),
            Err(DecodeError::Envelope)
        ));
    }
}
