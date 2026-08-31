//! Typed buffered event replay.
use crate::payload::SensorId;
use crate::{BootId, EventId, UtcMillis};
use alloc::string::String;
use alloc::vec::Vec;
use serde::{Deserialize, Serialize};
/// Buffer priority tier.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventTier {
    Audit,
    Telemetry,
    #[serde(other)]
    Unknown,
}
/// Event kind, unknown values preserved conservatively by the detail variant.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EventKind {
    WateringOfflineAutonomous,
    OfflineRefused,
    HistoryGap,
    PolicyActivated,
    LockoutSet,
    LockoutCleared,
    Unknown(String),
}
impl Serialize for EventKind {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(match self {
            Self::WateringOfflineAutonomous => "watering.offline_autonomous",
            Self::OfflineRefused => "offline.refused",
            Self::HistoryGap => "history.gap",
            Self::PolicyActivated => "policy.activated",
            Self::LockoutSet => "lockout.set",
            Self::LockoutCleared => "lockout.cleared",
            Self::Unknown(v) => v,
        })
    }
}
impl<'de> Deserialize<'de> for EventKind {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let v = String::deserialize(d)?;
        Ok(match v.as_str() {
            "watering.offline_autonomous" => Self::WateringOfflineAutonomous,
            "offline.refused" => Self::OfflineRefused,
            "history.gap" => Self::HistoryGap,
            "policy.activated" => Self::PolicyActivated,
            "lockout.set" => Self::LockoutSet,
            "lockout.cleared" => Self::LockoutCleared,
            _ => Self::Unknown(v),
        })
    }
}
/// Typed event detail discriminated independently of the wire kind.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "detail_type", rename_all = "snake_case")]
pub enum EventDetail {
    Watering {
        /// The plant the dose was delivered to, as the **device** understood it.
        ///
        /// A replayed dose has to name its own subject. The edge otherwise has
        /// to infer ownership from the actuator bindings that exist *at replay
        /// time*, which is a different fact from the one that was true when the
        /// water went into the pot: bindings can be edited, moved, or deleted
        /// while a device is isolated, and the budget would then be charged to
        /// whichever plant happens to be bound now. That misattribution is
        /// unsafe in both directions at once — the plant that really was watered
        /// keeps a clean budget and may be watered again, and a plant that was
        /// never touched is charged for water it did not receive.
        ///
        /// The device already knows the answer: `evaluate_offline` runs against
        /// exactly one `OfflinePolicy`, and that policy names its `plant_id`.
        /// Carrying it costs one optional field.
        ///
        /// `None` only for a v1 device published before this field existed. The
        /// edge falls back to binding-based attribution for those and says so.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        plant_id: Option<SensorId>,
        /** Applied policy. */
        policy_version: u32,
        /** Delivered volume. */
        delivered_ml: f32,
        /** Trigger sample. */
        trigger_value: f64,
        /** Duration. */
        duration_ms: u32,
    },
    Refused {
        /** Conservative reason. */
        reason: String,
    },
    Gap {
        /** First lost seq. */
        from_seq: u64,
        /** Last lost seq. */
        to_seq: u64,
        /** Count. */
        lost_count: u32,
        /** Tier lost. */
        lost_tier: EventTier,
    },
    PolicyActivated {
        /** Version. */
        policy_version: u32,
    },
    Lockout {
        /** Reason. */
        reason: String,
    },
    Unknown,
}
/// Buffered event with stable id.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BufferedEvent {
    /** Stable replay id. */
    pub event_id: EventId,
    /** Device sequence. */
    pub device_seq: u64,
    /** Buffer tier. */
    pub tier: EventTier,
    /** Event kind. */
    pub kind: EventKind,
    /** Always meaningful monotonic time. */
    pub monotonic_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /** Optional synced wall time. */
    pub device_time_ms: Option<UtcMillis>,
    /** Typed details. */
    pub detail: EventDetail,
}
/// Batch of live or replayed device events.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DeviceEventBatch {
    /** Replay marker. */
    pub replay: bool,
    #[serde(default)]
    /** Only final committed batch is complete. */
    pub complete: bool,
    /** Events. */
    pub events: Vec<BufferedEvent>,
}

/// Cumulative acknowledgement of replayed history (edge → device).
///
/// The transport primitive behind protocol §5.4's "a device MUST retain
/// replayed events until the edge acknowledges them". Without it that rule has
/// no mechanism: QoS 1 gives the device the *broker's* acknowledgement, and the
/// broker acks a message the edge may never have committed.
///
/// # Cumulative, not a list of ids
///
/// A prefix acknowledgement rather than an array of `event_id` values because it
/// is bounded on the wire whatever the buffer holds, is naturally idempotent, is
/// cheap for a device with kilobytes of RAM to apply, and matches the replay it
/// acknowledges — which is emitted in `device_seq` order, so every batch the
/// edge can have persisted is a prefix.
///
/// # What it asserts
///
/// > The edge has **durably committed** every replayed event for this
/// > `boot_id` up to and including `through_device_seq`.
///
/// Durably: the acknowledgement is published *after* the persistence
/// transaction commits, never before. An acknowledgement sent on receipt would
/// license the device to delete history the edge is about to lose.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EventAck {
    /// The boot whose replay is being acknowledged.
    ///
    /// Echoed from the envelope of the replay batch. A device MUST ignore an
    /// acknowledgement naming any other boot: a delayed acknowledgement from a
    /// previous run says nothing about the history this run is holding, and
    /// acting on it would delete unacknowledged events.
    pub boot_id: BootId,
    /// The highest `device_seq` the edge has durably committed.
    pub through_device_seq: u64,
}

/// Event batch structural failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EventBatchError {
    DuplicateEventId,
}
impl DeviceEventBatch {
    /** Rejects duplicate ids within one batch. */
    pub fn validate(&self) -> Result<(), EventBatchError> {
        for (i, e) in self.events.iter().enumerate() {
            if self.events[..i]
                .iter()
                .any(|seen| seen.event_id == e.event_id)
            {
                return Err(EventBatchError::DuplicateEventId);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use uuid::Uuid;
    #[test]
    fn gap_detail_and_optional_wall_time_round_trip() {
        let batch = DeviceEventBatch {
            replay: true,
            complete: false,
            events: vec![BufferedEvent {
                event_id: EventId::from_uuid(Uuid::nil()),
                device_seq: 4,
                tier: EventTier::Audit,
                kind: EventKind::HistoryGap,
                monotonic_ms: 99,
                device_time_ms: None,
                detail: EventDetail::Gap {
                    from_seq: 1,
                    to_seq: 3,
                    lost_count: 3,
                    lost_tier: EventTier::Telemetry,
                },
            }],
        };
        let json = serde_json::to_string(&batch).unwrap();
        let decoded: DeviceEventBatch = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, batch);
        assert!(!decoded.complete);
        assert!(decoded.events[0].device_time_ms.is_none());
    }
    #[test]
    fn duplicate_event_id_rejected() {
        let e = BufferedEvent {
            event_id: EventId::from_uuid(Uuid::nil()),
            device_seq: 1,
            tier: EventTier::Audit,
            kind: EventKind::PolicyActivated,
            monotonic_ms: 1,
            device_time_ms: None,
            detail: EventDetail::PolicyActivated { policy_version: 1 },
        };
        assert_eq!(
            DeviceEventBatch {
                replay: true,
                complete: true,
                events: vec![e.clone(), e]
            }
            .validate(),
            Err(EventBatchError::DuplicateEventId)
        );
    }
    #[test]
    fn unknown_kind_is_preserved_conservatively() {
        let kind: EventKind = serde_json::from_str("\"future.event\"").unwrap();
        assert_eq!(kind, EventKind::Unknown("future.event".into()));
    }
}
