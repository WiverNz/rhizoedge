//! Command and result payloads.
use crate::{CommandId, UtcMillis};
use alloc::string::String;
use serde::{Deserialize, Serialize};
/// Water command.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct WaterCommand {
    /** Idempotency key. */
    pub command_id: CommandId,
    /** Requested volume. */
    pub requested_ml: f32,
    /** Edge issue time. */
    pub issued_at_ms: UtcMillis,
    /** Expiry. */
    pub expires_at_ms: UtcMillis,
}
/// Tare command.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct TareCommand {
    /** Idempotency key. */
    pub command_id: CommandId,
    /** Edge issue time. */
    pub issued_at_ms: UtcMillis,
    /** Expiry. */
    pub expires_at_ms: UtcMillis,
}
/// Calibration command.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CalibrateCommand {
    /** Idempotency key. */
    pub command_id: CommandId,
    /** Fixed duration. */
    pub run_seconds: f32,
    /** Edge issue time. */
    pub issued_at_ms: UtcMillis,
    /** Expiry. */
    pub expires_at_ms: UtcMillis,
}
/// Structural command validation failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandError {
    RequestedVolume,
    Expiry,
    RunDuration,
}
impl WaterCommand {
    /** Validates wire-level invariants. */
    pub fn validate(&self) -> Result<(), CommandError> {
        if !self.requested_ml.is_finite() || self.requested_ml <= 0.0 {
            return Err(CommandError::RequestedVolume);
        }
        validate_times(self.issued_at_ms, self.expires_at_ms)
    }
}
impl TareCommand {
    /** Validates expiry. */
    pub fn validate(&self) -> Result<(), CommandError> {
        validate_times(self.issued_at_ms, self.expires_at_ms)
    }
}
impl CalibrateCommand {
    /** Validates duration and expiry. */
    pub fn validate(&self) -> Result<(), CommandError> {
        if !self.run_seconds.is_finite() || self.run_seconds <= 0.0 {
            return Err(CommandError::RunDuration);
        }
        validate_times(self.issued_at_ms, self.expires_at_ms)
    }
}
fn validate_times(issue: UtcMillis, expiry: UtcMillis) -> Result<(), CommandError> {
    if expiry <= issue {
        Err(CommandError::Expiry)
    } else {
        Ok(())
    }
}
/// Result lifecycle status.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandStatus {
    Completed,
    Rejected,
    Interrupted,
    Failed,
    #[serde(other)]
    Unknown,
}
/// Conservative rejection reason.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RejectReason {
    ClockUnsynced,
    Expired,
    MalformedCommand,
    LeakDetected,
    LeakUnknown,
    TankUnknown,
    TankLow,
    PumpUnavailable,
    OverDailyMax,
    #[serde(other)]
    Unknown,
}
/// Command origin.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandOrigin {
    EdgeCommand,
    OfflineAutonomous,
    #[serde(other)]
    Unknown,
}
/// Device command result.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CommandResult {
    /** Correlation. */
    pub command_id: CommandId,
    /** Outcome. */
    pub status: CommandStatus,
    /** Requested volume. */
    pub requested_ml: f32,
    /** Actual volume; unknown after interruption. */
    pub delivered_ml: Option<f32>,
    /** Run duration. */
    pub duration_ms: Option<u32>,
    /** Whether hard limits reduced it. */
    pub clamped: bool,
    /** Rejection reason. */
    pub reason: Option<RejectReason>,
    /** Rolling device volume. */
    pub delivered_today_ml: f32,
    /** Authority. */
    pub origin: CommandOrigin,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    /** Future diagnostic detail. */
    pub detail: Option<String>,
}

/// Durable acknowledgement of one `command.result` (edge → device).
///
/// # Why QoS 1 is not this
///
/// MQTT QoS 1 is **hop by hop**. The PUBACK a device receives for its
/// `command.result` is written by the *broker*, on receipt, and says nothing
/// about the edge — which may not have read the message yet, may crash before
/// it commits, and (with a clean session) will never be offered it again. A
/// device that stopped retrying on that PUBACK would therefore drop a result
/// the edge never recorded.
///
/// A lost result is not a lost sample. It is ledger data: the edge's rolling
/// 24-hour budget (SAFETY-006) is derived from the rows results produce, so a
/// silently dropped `completed` under-counts delivered volume, and under-counting
/// is the direction that waters again too soon.
///
/// This is the same argument protocol §5.4 already makes for `event.ack`, and
/// the same shape of answer: an application-level acknowledgement published
/// **after** the edge's transaction commits.
///
/// # Per `command_id`, not cumulative
///
/// `event.ack` can be cumulative because replayed events carry a total order —
/// `device_seq`. Results have no such order: a `command_id` is a UUID minted by
/// whoever issued the command. So the acknowledgement names exactly the result
/// it covers, and a device clears that one entry.
///
/// Idempotent in both directions: acknowledging a `command_id` the device is no
/// longer holding is a no-op, and an edge that has already committed a result
/// re-acknowledges it on every redelivery — which is what lets a device whose
/// acknowledgement was lost make progress on its next retry.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CommandResultAck {
    /// The result the edge has durably committed.
    pub command_id: CommandId,
}
#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;
    fn cmd() -> WaterCommand {
        WaterCommand {
            command_id: CommandId::from_uuid(Uuid::nil()),
            requested_ml: 1.,
            issued_at_ms: UtcMillis(1),
            expires_at_ms: UtcMillis(2),
        }
    }
    #[test]
    fn validation_boundaries() {
        let mut c = cmd();
        for v in [0., -1., f32::NAN] {
            c.requested_ml = v;
            assert_eq!(c.validate(), Err(CommandError::RequestedVolume));
        }
        let mut c = cmd();
        c.expires_at_ms = c.issued_at_ms;
        assert_eq!(c.validate(), Err(CommandError::Expiry));
    }
    #[test]
    fn unknown_and_null_are_conservative() {
        let r: RejectReason = serde_json::from_str("\"future\"").unwrap();
        assert_eq!(r, RejectReason::Unknown);
        let mut value = serde_json::to_value(CommandResult {
            command_id: cmd().command_id,
            status: CommandStatus::Interrupted,
            requested_ml: 1.,
            delivered_ml: None,
            duration_ms: None,
            clamped: false,
            reason: None,
            delivered_today_ml: 1.,
            origin: CommandOrigin::EdgeCommand,
            detail: None,
        })
        .unwrap();
        assert!(value.get_mut("delivered_ml").unwrap().is_null());
    }
    #[test]
    fn a_result_acknowledgement_round_trips() {
        let ack = CommandResultAck {
            command_id: CommandId::from_uuid(Uuid::nil()),
        };
        let json = serde_json::to_string(&ack).unwrap();
        assert_eq!(
            json,
            r#"{"command_id":"00000000-0000-0000-0000-000000000000"}"#
        );
        assert_eq!(
            serde_json::from_str::<CommandResultAck>(&json).unwrap(),
            ack
        );
    }
}
