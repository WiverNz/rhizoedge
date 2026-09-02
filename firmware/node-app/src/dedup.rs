//! The persisted command deduplication ring (F-090-33, SAFETY-001, SAFETY-011).
//!
//! Sixteen entries in NVS, so a `command_id` executed before a reboot is still
//! recognised after it. A repeat re-publishes the **stored** result and does
//! not actuate — the stored one, not a freshly computed one, because the point
//! is to tell the edge what actually happened rather than what would happen
//! now.
//!
//! The ring is written on every dose. That is flash wear, and it is accepted:
//! doses are infrequent, entries are small, and the alternative is a device
//! that re-waters a plant after a power cut.

use rhizo_mqtt_contract::{CommandId, payload::CommandResult, safety::COMMAND_DEDUP_RING};

use crate::persist::DedupEntry;

/// Looks up a previously executed command.
#[must_use]
pub fn previous(ring: &[DedupEntry], command_id: CommandId) -> Option<&CommandResult> {
    ring.iter()
        .find(|entry| entry.command_id == command_id)
        .map(|entry| &entry.result)
}

/// Records an executed command, evicting the oldest entry when full.
///
/// Eviction here is safe and is **not** the ledger's question. Losing the
/// oldest dedup entry can only mean a very old `command_id` is executed twice
/// if the edge re-sends it after sixteen intervening commands — and the edge
/// never re-sends a settled command, because a retry reuses the same
/// `command_id` only while the command is still in flight. Nothing about the
/// delivered-water accounting depends on this ring.
pub fn record(ring: &mut Vec<DedupEntry>, command_id: CommandId, result: CommandResult) {
    if let Some(existing) = ring.iter_mut().find(|entry| entry.command_id == command_id) {
        existing.result = result;
        return;
    }
    ring.push(DedupEntry { command_id, result });
    while ring.len() > COMMAND_DEDUP_RING {
        ring.remove(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rhizo_mqtt_contract::payload::{CommandOrigin, CommandStatus, RejectReason};
    use uuid::Uuid;

    fn id(n: u128) -> CommandId {
        CommandId::from_uuid(Uuid::from_u128(n))
    }

    fn result(n: u128) -> CommandResult {
        CommandResult {
            command_id: id(n),
            status: CommandStatus::Completed,
            requested_ml: 10.0,
            delivered_ml: Some(10.0),
            duration_ms: Some(1000),
            clamped: false,
            reason: None,
            delivered_today_ml: 10.0 * n as f32,
            origin: CommandOrigin::EdgeCommand,
            detail: None,
        }
    }

    #[test]
    fn safety_001_a_repeat_finds_the_stored_result() {
        let mut ring = Vec::new();
        record(&mut ring, id(1), result(1));
        let found = previous(&ring, id(1)).expect("stored");
        assert_eq!(found.delivered_today_ml, 10.0);
        assert!(previous(&ring, id(2)).is_none());
    }

    #[test]
    fn the_ring_is_bounded_at_the_shared_constant() {
        let mut ring = Vec::new();
        for n in 1..=(COMMAND_DEDUP_RING as u128 + 4) {
            record(&mut ring, id(n), result(n));
        }
        assert_eq!(ring.len(), COMMAND_DEDUP_RING);
        assert!(previous(&ring, id(1)).is_none(), "oldest evicted");
        assert!(previous(&ring, id(COMMAND_DEDUP_RING as u128 + 4)).is_some());
    }

    /// A rejection is a command the device executed, so it is remembered: a
    /// repeat must republish the refusal rather than re-run the gate against
    /// different inputs and possibly water.
    #[test]
    fn a_rejection_is_stored_and_republished_unchanged() {
        let mut ring = Vec::new();
        let mut rejected = result(1);
        rejected.status = CommandStatus::Rejected;
        rejected.reason = Some(RejectReason::LeakDetected);
        rejected.delivered_ml = Some(0.0);
        record(&mut ring, id(1), rejected.clone());
        assert_eq!(previous(&ring, id(1)), Some(&rejected));
    }

    #[test]
    fn re_recording_the_same_command_updates_in_place() {
        let mut ring = Vec::new();
        record(&mut ring, id(1), result(1));
        record(&mut ring, id(1), result(2));
        assert_eq!(ring.len(), 1);
        assert_eq!(
            previous(&ring, id(1)).map(|r| r.delivered_today_ml),
            Some(20.0)
        );
    }
}
