//! Outbound message assembly.
//!
//! Every device→edge publication is stamped here, and only here: one place that
//! owns `boot_id`, the per-boot `sequence`, `message_id` generation, and the
//! advisory `device_time_ms` / `clock_synced` pair.
//!
//! # Retention is derived, never chosen
//!
//! [`Publication::new`] takes its retain flag from
//! [`Topic::metadata`] rather than from the caller. Protocol §3 makes retention
//! a property of the topic — `status`, `config`, and `policy` retained,
//! everything else never — and a retained command topic is the single most
//! damaging mistake available in this protocol (ADR-002). Deriving the flag
//! means a call site cannot get it wrong even by accident; there is no
//! parameter to pass incorrectly.

use rhizo_mqtt_contract::{
    BootId, DeviceId, Envelope, MessageId, MessageKind, PROTOCOL_VERSION, Qos, Topic, UtcMillis,
};
use serde::Serialize;
use uuid::Uuid;

use crate::rng::SplitMix64;

/// One message ready for the broker.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Publication {
    /// Destination topic.
    pub topic: Topic,
    /// Serialised envelope.
    pub payload: String,
    /// Required QoS. QoS 1 everywhere in v1.
    pub qos: Qos,
    /// Required retain flag, derived from the topic.
    pub retain: bool,
}

impl Publication {
    /// Builds a publication whose delivery flags come from the topic.
    #[must_use]
    pub fn new(topic: Topic, payload: String) -> Self {
        let metadata = topic.metadata();
        Self {
            topic,
            payload,
            qos: metadata.qos,
            retain: metadata.retained,
        }
    }

    /// The topic as a wire string.
    #[must_use]
    pub fn topic_string(&self) -> String {
        self.topic.as_string()
    }
}

/// The device's per-boot publishing identity.
#[derive(Debug)]
pub struct Identity {
    device_id: DeviceId,
    boot_id: BootId,
    sequence: u64,
    ids: SplitMix64,
}

impl Identity {
    /// Creates an identity with a fresh `boot_id`.
    ///
    /// The identity generator is seeded from the operating system, never from
    /// `--seed`: reproducible `message_id` values would be deduplicated away by
    /// the edge after a restart.
    #[must_use]
    pub fn new(device_id: DeviceId) -> Self {
        let mut ids = SplitMix64::from_os();
        let boot_id = BootId::from_uuid(uuid_v4(&mut ids));
        Self {
            device_id,
            boot_id,
            sequence: 0,
            ids,
        }
    }

    /// The device identity.
    #[must_use]
    pub const fn device_id(&self) -> &DeviceId {
        &self.device_id
    }

    /// This boot's identifier.
    #[must_use]
    pub const fn boot_id(&self) -> BootId {
        self.boot_id
    }

    /// The last sequence number issued.
    #[must_use]
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }

    /// A fresh identifier for a message or a batch.
    ///
    /// UUIDv7 when the wall clock is synchronised, so ids sort by time and give
    /// the edge a cheap consistency check; UUIDv4 otherwise, because a v7 built
    /// from a meaningless timestamp would claim an ordering the device cannot
    /// support (protocol §4).
    pub fn next_uuid(&mut self, wall_ms: Option<UtcMillis>) -> Uuid {
        match wall_ms {
            Some(now) => MessageId::new_v7(now, &mut self.ids).as_uuid(),
            None => uuid_v4(&mut self.ids),
        }
    }

    /// Stamps a payload into a complete envelope and serialises it.
    ///
    /// `sequence` advances on every call, so a message that is built is a
    /// message that was sent — gaps in the sequence mean loss, which is exactly
    /// what the edge's gap detection is looking for.
    ///
    /// # Errors
    ///
    /// Returns the serialisation failure, which for these types can only mean a
    /// non-finite float reached a payload — a protocol violation (§4) rather
    /// than a transient condition.
    pub fn seal<T: Serialize>(
        &mut self,
        topic: Topic,
        data: T,
        wall_ms: Option<UtcMillis>,
        clock_synced: bool,
    ) -> Result<Publication, serde_json::Error> {
        self.sequence = self.sequence.saturating_add(1);
        let envelope = Envelope {
            v: PROTOCOL_VERSION,
            kind: MessageKind::for_topic(&topic),
            message_id: MessageId::from_uuid(self.next_uuid(wall_ms)),
            device_id: self.device_id.clone(),
            boot_id: Some(self.boot_id),
            sequence: Some(self.sequence),
            device_time_ms: wall_ms,
            clock_synced: Some(clock_synced),
            data,
        };
        Ok(Publication::new(topic, serde_json::to_string(&envelope)?))
    }

    /// Stamps the Last Will and Testament.
    ///
    /// Fixed at connect time, so it carries `sequence: 0` and
    /// `clock_synced: false`: the will is written before the device knows
    /// anything about the Edge's clock, and it will be published at an unknown
    /// future instant (protocol §5.6).
    ///
    /// # Errors
    ///
    /// As [`seal`](Self::seal).
    pub fn seal_will<T: Serialize>(
        &mut self,
        topic: Topic,
        data: T,
    ) -> Result<Publication, serde_json::Error> {
        let envelope = Envelope {
            v: PROTOCOL_VERSION,
            kind: MessageKind::for_topic(&topic),
            message_id: MessageId::from_uuid(self.next_uuid(None)),
            device_id: self.device_id.clone(),
            boot_id: Some(self.boot_id),
            sequence: Some(0),
            device_time_ms: None,
            clock_synced: Some(false),
            data,
        };
        Ok(Publication::new(topic, serde_json::to_string(&envelope)?))
    }
}

/// Builds a random UUIDv4 from a caller-supplied generator.
///
/// The contract crate offers only `new_v7`, which needs a trustworthy
/// timestamp. An unsynchronised device has none.
fn uuid_v4(rng: &mut SplitMix64) -> Uuid {
    use rhizo_mqtt_contract::ids::RandomSource;
    let mut bytes = [0u8; 16];
    rng.fill_bytes(&mut bytes);
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rhizo_mqtt_contract::payload::{DeviceStatus, DeviceStatusValue};
    use std::collections::BTreeMap;

    fn identity() -> Identity {
        Identity::new(DeviceId::parse("plant-node-01").unwrap())
    }

    fn status() -> DeviceStatus {
        DeviceStatus {
            status: DeviceStatusValue::Online,
            reason: None,
            firmware_version: None,
            protocol_version: Some(1),
            applied_config_version: None,
            uptime_ms: None,
            free_heap_bytes: None,
            rssi_dbm: None,
            applied_policy_versions: BTreeMap::new(),
            connectivity: None,
            capabilities: Default::default(),
            limits: None,
        }
    }

    #[test]
    fn retention_is_taken_from_the_topic_not_from_the_caller() {
        let id = DeviceId::parse("plant-node-01").unwrap();
        let retained = [
            Topic::Status(id.clone()),
            Topic::Config(id.clone()),
            Topic::Policy(id.clone()),
        ];
        let never_retained = [
            Topic::Telemetry(id.clone()),
            Topic::Actuator(id.clone()),
            Topic::Events(id.clone()),
            Topic::Time(id.clone()),
            Topic::CommandWater(id.clone()),
            Topic::CommandTare(id.clone()),
            Topic::CommandCalibrate(id.clone()),
            Topic::CommandResult(id),
        ];
        for topic in retained {
            assert!(Publication::new(topic, String::new()).retain);
        }
        for topic in never_retained {
            let p = Publication::new(topic, String::new());
            assert!(!p.retain, "{} must never be retained", p.topic_string());
            assert_eq!(p.qos, Qos::One);
        }
    }

    #[test]
    fn a_sealed_envelope_decodes_as_the_protocol_requires() {
        let mut identity = identity();
        let p = identity
            .seal(
                Topic::Status(identity.device_id().clone()),
                status(),
                Some(UtcMillis(1_756_121_400_000)),
                true,
            )
            .unwrap();
        let decoded = Envelope::<DeviceStatus>::from_json(p.payload.as_bytes()).unwrap();
        decoded.check_topic(&p.topic).unwrap();
        assert_eq!(decoded.v, 1);
        assert_eq!(decoded.boot_id, Some(identity.boot_id()));
        assert_eq!(decoded.sequence, Some(1));
        assert_eq!(decoded.clock_synced, Some(true));
        assert_eq!(decoded.message_id.as_uuid().get_version_num(), 7);
    }

    #[test]
    fn an_unsynchronised_device_emits_v4_and_omits_device_time() {
        let mut identity = identity();
        let p = identity
            .seal(
                Topic::Status(identity.device_id().clone()),
                status(),
                None,
                false,
            )
            .unwrap();
        let decoded = Envelope::<DeviceStatus>::from_json(p.payload.as_bytes()).unwrap();
        assert_eq!(decoded.message_id.as_uuid().get_version_num(), 4);
        assert_eq!(decoded.device_time_ms, None);
        assert_eq!(decoded.clock_synced, Some(false));
    }

    #[test]
    fn sequence_increases_monotonically_within_a_boot() {
        let mut identity = identity();
        let topic = Topic::Status(identity.device_id().clone());
        let mut last = 0;
        for _ in 0..32 {
            let p = identity.seal(topic.clone(), status(), None, false).unwrap();
            let decoded = Envelope::<DeviceStatus>::from_json(p.payload.as_bytes()).unwrap();
            let seq = decoded.sequence.unwrap();
            assert!(seq > last, "{seq} did not advance past {last}");
            last = seq;
        }
    }

    #[test]
    fn a_fresh_identity_has_a_fresh_boot_id() {
        assert_ne!(identity().boot_id(), identity().boot_id());
    }

    #[test]
    fn the_will_is_sequence_zero_and_never_claims_a_clock() {
        let mut identity = identity();
        let p = identity
            .seal_will(
                Topic::Status(identity.device_id().clone()),
                DeviceStatus {
                    status: DeviceStatusValue::Offline,
                    reason: Some("connection_lost".into()),
                    ..status()
                },
            )
            .unwrap();
        assert!(p.retain, "the will must be retained (protocol §5.6)");
        let decoded = Envelope::<DeviceStatus>::from_json(p.payload.as_bytes()).unwrap();
        assert_eq!(decoded.sequence, Some(0));
        assert_eq!(decoded.clock_synced, Some(false));
        assert_eq!(decoded.data.status, DeviceStatusValue::Offline);
        assert_eq!(decoded.data.reason.as_deref(), Some("connection_lost"));
        assert_eq!(
            identity.sequence(),
            0,
            "the will must not consume a live sequence number"
        );
    }

    #[test]
    fn message_ids_do_not_repeat() {
        let mut identity = identity();
        let mut seen = std::collections::HashSet::new();
        for i in 0..1_000 {
            let wall = (i % 2 == 0).then_some(UtcMillis(1_756_121_400_000 + i));
            assert!(seen.insert(identity.next_uuid(wall)), "repeated id at {i}");
        }
    }
}
