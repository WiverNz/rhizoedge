//! Central MQTT v1 topic grammar and delivery metadata.
use crate::DeviceId;
use alloc::{string::String, vec::Vec};
use core::{fmt, str::FromStr};

/// MQTT QoS supported by the v1 contract.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Qos {
    /// At-least-once delivery.
    One,
}
/// Required publication flags for a topic.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TopicMetadata {
    /** Required QoS. */
    pub qos: Qos,
    /** Required retain flag. */
    pub retained: bool,
}
/// Every MQTT v1 topic (eleven concrete topic forms).
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Topic {
    Telemetry(DeviceId),
    Actuator(DeviceId),
    Events(DeviceId),
    Status(DeviceId),
    Config(DeviceId),
    Policy(DeviceId),
    Time(DeviceId),
    CommandWater(DeviceId),
    CommandTare(DeviceId),
    CommandCalibrate(DeviceId),
    CommandResult(DeviceId),
}
/// Topic parse failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TopicError {
    Malformed,
    UnsupportedVersion,
    InvalidDeviceId,
    UnknownSuffix,
}
impl Topic {
    /// Edge wildcard subscription.
    pub const EDGE_SUBSCRIPTION: &'static str = "rhizo/v1/devices/+/#";
    /// Builds the exact wire topic.
    pub fn as_string(&self) -> String {
        let (id, suffix) = match self {
            Self::Telemetry(i) => (i, "telemetry"),
            Self::Actuator(i) => (i, "actuator"),
            Self::Events(i) => (i, "events"),
            Self::Status(i) => (i, "status"),
            Self::Config(i) => (i, "config"),
            Self::Policy(i) => (i, "policy"),
            Self::Time(i) => (i, "time"),
            Self::CommandWater(i) => (i, "commands/water"),
            Self::CommandTare(i) => (i, "commands/tare"),
            Self::CommandCalibrate(i) => (i, "commands/calibrate"),
            Self::CommandResult(i) => (i, "commands/result"),
        };
        alloc::format!("rhizo/v1/devices/{id}/{suffix}")
    }
    /// Parses and validates a topic.
    pub fn parse(value: &str) -> Result<Self, TopicError> {
        let parts: Vec<_> = value.split('/').collect();
        if parts.len() < 5 || parts[0] != "rhizo" || parts[2] != "devices" {
            return Err(TopicError::Malformed);
        }
        if parts[1] != "v1" {
            return Err(TopicError::UnsupportedVersion);
        }
        let id = DeviceId::parse(parts[3]).map_err(|_| TopicError::InvalidDeviceId)?;
        match parts[4..] {
            ["telemetry"] => Ok(Self::Telemetry(id)),
            ["actuator"] => Ok(Self::Actuator(id)),
            ["events"] => Ok(Self::Events(id)),
            ["status"] => Ok(Self::Status(id)),
            ["config"] => Ok(Self::Config(id)),
            ["policy"] => Ok(Self::Policy(id)),
            ["time"] => Ok(Self::Time(id)),
            ["commands", "water"] => Ok(Self::CommandWater(id)),
            ["commands", "tare"] => Ok(Self::CommandTare(id)),
            ["commands", "calibrate"] => Ok(Self::CommandCalibrate(id)),
            ["commands", "result"] => Ok(Self::CommandResult(id)),
            _ => Err(TopicError::UnknownSuffix),
        }
    }
    /// Returns the device scoped by this topic.
    pub fn device_id(&self) -> &DeviceId {
        match self {
            Self::Telemetry(i)
            | Self::Actuator(i)
            | Self::Events(i)
            | Self::Status(i)
            | Self::Config(i)
            | Self::Policy(i)
            | Self::Time(i)
            | Self::CommandWater(i)
            | Self::CommandTare(i)
            | Self::CommandCalibrate(i)
            | Self::CommandResult(i) => i,
        }
    }
    /// Returns required QoS and retention.
    pub const fn metadata(&self) -> TopicMetadata {
        TopicMetadata {
            qos: Qos::One,
            retained: matches!(self, Self::Status(_) | Self::Config(_) | Self::Policy(_)),
        }
    }
}
impl fmt::Display for Topic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.as_string())
    }
}
impl FromStr for Topic {
    type Err = TopicError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}
impl fmt::Display for TopicError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid MQTT topic: {self:?}")
    }
}
#[cfg(feature = "std")]
impl std::error::Error for TopicError {}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::format;
    #[test]
    fn exact_round_trips_and_flags() {
        let id = DeviceId::parse("node-01").unwrap();
        let cases = [
            (Topic::Telemetry(id.clone()), "telemetry", false),
            (Topic::Actuator(id.clone()), "actuator", false),
            (Topic::Events(id.clone()), "events", false),
            (Topic::Status(id.clone()), "status", true),
            (Topic::Config(id.clone()), "config", true),
            (Topic::Policy(id.clone()), "policy", true),
            (Topic::Time(id.clone()), "time", false),
            (Topic::CommandWater(id.clone()), "commands/water", false),
            (Topic::CommandTare(id.clone()), "commands/tare", false),
            (
                Topic::CommandCalibrate(id.clone()),
                "commands/calibrate",
                false,
            ),
            (Topic::CommandResult(id), "commands/result", false),
        ];
        for (topic, suffix, retained) in cases {
            assert_eq!(
                topic.as_string(),
                format!("rhizo/v1/devices/node-01/{suffix}")
            );
            assert_eq!(Topic::parse(&topic.as_string()).unwrap(), topic);
            assert_eq!(
                topic.metadata(),
                TopicMetadata {
                    qos: Qos::One,
                    retained
                }
            );
        }
    }
    #[test]
    fn malformed_rejected() {
        for s in [
            "",
            "rhizo/v2/devices/abc/status",
            "rhizo/v1/devices/x%23/status",
            "rhizo/v1/devices/abc/nope",
            "rhizo/v1/devices",
        ] {
            assert!(Topic::parse(s).is_err(), "{s}");
        }
    }
}
