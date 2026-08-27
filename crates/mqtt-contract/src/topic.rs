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
/// Every MQTT v1 topic (twelve concrete topic forms).
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
    EventsAck(DeviceId),
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
    /// Narrow device-to-edge wildcard filters.
    ///
    /// These are equivalent to the normative subtree subscription for all v1
    /// device-originated forms, while excluding every edge-to-device topic.
    pub const EDGE_SUBSCRIPTIONS: [&'static str; 5] = [
        "rhizo/v1/devices/+/telemetry",
        "rhizo/v1/devices/+/actuator",
        "rhizo/v1/devices/+/events",
        "rhizo/v1/devices/+/status",
        "rhizo/v1/devices/+/commands/result",
    ];
    /// The seven subscriptions a device MUST establish, in protocol §3 order.
    ///
    /// **Exact topics, never a wildcard.** `commands/+` would also match
    /// `commands/result`, which the device itself publishes, and MQTT 3.1.1 has
    /// no subscription-level "no local" option to suppress that. A device would
    /// then be delivered every result it had just sent, and the rule "MUST NOT
    /// subscribe to `commands/result`" would be unenforceable rather than merely
    /// unenforced. Naming the three command topics costs two extra SUBSCRIBE
    /// entries and removes the seam entirely.
    ///
    /// `telemetry`, `actuator`, `events`, `status`, and `commands/result` are
    /// absent deliberately: a device publishes those. Returning the complete set
    /// as one value is what lets a reconnect restore *exactly* these, rather
    /// than whichever subset a call site remembered.
    ///
    /// The cost of the exact form is that a command kind added in a later v1
    /// revision is not received until the device names it. That is the safer
    /// failure: an unreceived command is a command not executed, whereas a
    /// wildcard that silently delivered the device its own output is a live
    /// seam in the one topic tree that carries actuation.
    pub fn device_subscriptions(device_id: &DeviceId) -> [String; 7] {
        [
            Self::Config(device_id.clone()).as_string(),
            Self::Policy(device_id.clone()).as_string(),
            Self::Time(device_id.clone()).as_string(),
            Self::CommandWater(device_id.clone()).as_string(),
            Self::CommandTare(device_id.clone()).as_string(),
            Self::CommandCalibrate(device_id.clone()).as_string(),
            Self::EventsAck(device_id.clone()).as_string(),
        ]
    }
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
            Self::EventsAck(i) => (i, "events/ack"),
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
            ["events", "ack"] => Ok(Self::EventsAck(id)),
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
            | Self::CommandResult(i)
            | Self::EventsAck(i) => i,
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
            (Topic::CommandResult(id.clone()), "commands/result", false),
            (Topic::EventsAck(id), "events/ack", false),
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
    /// An acknowledgement is a statement about one moment, and retaining it
    /// would make the broker repeat that statement to a device that reconnects
    /// much later — after which the device would delete history the edge may
    /// since have lost. Protocol §5.13: `event.ack` is never retained.
    #[test]
    fn an_acknowledgement_is_never_retained() {
        let id = DeviceId::parse("node-01").unwrap();
        assert!(
            !Topic::EventsAck(id).metadata().retained,
            "a retained acknowledgement would be replayed at every reconnect"
        );
    }

    /// Protocol §3 "Subscriptions": exactly seven **exact** topics, and no
    /// wildcard that could reach the device's own output.
    #[test]
    fn device_subscribes_to_exactly_the_normative_topics() {
        let id = DeviceId::parse("node-01").unwrap();
        let subs = Topic::device_subscriptions(&id);
        assert_eq!(
            subs,
            [
                "rhizo/v1/devices/node-01/config",
                "rhizo/v1/devices/node-01/policy",
                "rhizo/v1/devices/node-01/time",
                "rhizo/v1/devices/node-01/commands/water",
                "rhizo/v1/devices/node-01/commands/tare",
                "rhizo/v1/devices/node-01/commands/calibrate",
                "rhizo/v1/devices/node-01/events/ack",
            ]
            .map(String::from)
        );
        for published_by_the_device in [
            Topic::CommandResult(id.clone()),
            Topic::Telemetry(id.clone()),
            Topic::Actuator(id.clone()),
            Topic::Events(id.clone()),
            Topic::Status(id),
        ] {
            assert!(
                !subs.contains(&published_by_the_device.as_string()),
                "a device must not subscribe to {published_by_the_device}"
            );
        }
    }

    /// The property the exact form exists to guarantee: no subscription can
    /// match anything the device publishes.
    ///
    /// Checked as "contains no wildcard" rather than only as string inequality,
    /// because the failure being removed was a *wildcard that matched* — which
    /// an equality check would not have caught.
    #[test]
    fn no_device_subscription_can_match_a_topic_the_device_publishes() {
        let id = DeviceId::parse("node-01").unwrap();
        let published = [
            Topic::Telemetry(id.clone()),
            Topic::Actuator(id.clone()),
            Topic::Events(id.clone()),
            Topic::Status(id.clone()),
            Topic::CommandResult(id.clone()),
        ];
        for filter in Topic::device_subscriptions(&id) {
            assert!(
                !filter.contains('+') && !filter.contains('#'),
                "{filter} is a wildcard; only an exact topic cannot over-match"
            );
            for topic in &published {
                assert_ne!(
                    filter,
                    topic.as_string(),
                    "a device subscription reaches its own {topic}"
                );
            }
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
