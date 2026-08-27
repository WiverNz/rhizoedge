//! Typed buffered event replay.
use crate::{EventId, UtcMillis};
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
