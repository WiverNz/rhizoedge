//! The durable pending-`command.result` ledger (F-090-16 to F-090-19, M9-011).
//!
//! # Why this exists at all
//!
//! Since the post-M6 correction a result is retired only by
//! `command.result.ack` (protocol §5.14), never by the broker's PUBACK — QoS 1
//! is hop by hop and says nothing about whether the edge committed. A device
//! that waters while the edge is down therefore accumulates *several*
//! unacknowledged results, so this is a bounded durable ledger and not a single
//! slot.
//!
//! # The decision this module records
//!
//! Every bounded structure saturates, and ADR-014 §Device-side pending-result
//! ledger requires M9 to decide what happens then rather than default into it.
//! The decision, against ADR-014's six points:
//!
//! 1. **New actuation is refused while saturated**, with
//!    [`RejectReason::ResultLedgerFull`]. The check is a device-local veto that
//!    runs *after* `validate_water_command` has already accepted, so the shared
//!    gate stays the only gate and this can only ever stop a dose, never permit
//!    one. A device that cannot record what it delivered does not deliver more.
//! 2. **Already-delivered water stays attributable.** Nothing is evicted, so
//!    every entry is still here. Independently, `delivered_today_ml` is carried
//!    in every `command.result` and in `device.status`, giving the edge a
//!    running total to reconcile against even while individual results are
//!    in flight — the aggregated form ADR-014 point 2 asks about.
//! 3. **Saturation is visible.** [`LedgerState::Saturated`] raises a
//!    `pump_fault`-tier audit event in the device event buffer (durable, never
//!    evicted by telemetry) and is reported in `device.status`. It is never an
//!    invisible steady state.
//! 4. **Recovery** is per `command_id`: an acknowledgement removes exactly the
//!    named entry, an acknowledgement for an entry that is not held is a no-op
//!    (§5.14), and freeing a slot below the threshold clears the fault. No
//!    entry is re-keyed or renumbered, so nothing is double-counted.
//! 5. **Reboot** carries the whole ledger, because it is part of
//!    [`crate::persist::PersistedState`] and is written before the publish.
//!    Re-publishing after a reboot is expected and is deduplicated by the edge
//!    on `command_id`.
//! 6. **No eviction of an unacknowledged result is adopted.** ADR-014 point 6
//!    permits one only with an explicit safety-equivalence argument; no such
//!    argument exists, because an evicted result removes a quantity the edge's
//!    rolling 24-hour budget is derived from and the edge learns nothing at all.
//!    That is why the event buffer's "evict oldest, record a gap" does not
//!    transfer: a gap reports a lost *record*, which the edge can see and reason
//!    about; a dropped result leaves the edge's *arithmetic* wrong with nothing
//!    to notice, and under-counting is the direction that waters again too soon.
//!
//! # The reserved slot
//!
//! Capacity is [`PENDING_RESULT_CAPACITY`] and actuation stops at
//! [`ACTUATION_THRESHOLD`], one below it. The reserve is not tidiness: a
//! refusal is itself a `command.result` and needs somewhere to live, so without
//! it the device could reach a state where it cannot even record the refusal it
//! just issued.
//!
//! If the ledger is nonetheless completely full — several commands arriving
//! while saturated — a *rejection* result is published once, un-ledgered and
//! unretried. That is sound for exactly one class of result and no other: a
//! rejection reports zero delivered water, so losing it cannot under-count
//! anything. The saturation audit event is what carries the fact to the edge
//! durably.

use serde::{Deserialize, Serialize};

use rhizo_mqtt_contract::{CommandId, payload::CommandResult};

/// How many unacknowledged results the device keeps.
///
/// Sixteen, matched to `rhizo_mqtt_contract::safety::COMMAND_DEDUP_RING`.
/// The dedup ring already bounds how many distinct commands the device can
/// remember at all, so a deeper ledger could hold a result for a command the
/// ring has forgotten — the two structures answer for the same commands and are
/// sized together deliberately.
///
/// This is **not** the simulator's `PENDING_RESULT_LIMIT = 32`, and not the
/// simulator's policy. See the module documentation.
pub const PENDING_RESULT_CAPACITY: usize = 16;

/// The occupancy at which new actuation is refused, leaving one reserved slot.
pub const ACTUATION_THRESHOLD: usize = PENDING_RESULT_CAPACITY - 1;

/// Whether the ledger currently permits actuation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LedgerState {
    /// Below the threshold; ordinary operation.
    Normal,
    /// At or above the threshold; actuation is refused and a fault is raised.
    Saturated,
}

/// One result awaiting `command.result.ack`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PendingResult {
    /// The result to republish.
    pub result: CommandResult,
    /// Monotonic milliseconds at which it was last published, if ever.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_published_ms: Option<u64>,
    /// How many times it has been published.
    #[serde(default)]
    pub publish_count: u32,
}

/// Why an insert was refused.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LedgerFull {
    /// Every slot including the reserve is occupied.
    NoCapacity,
}

/// The bounded durable ledger.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PendingLedger {
    entries: Vec<PendingResult>,
    /// Whether the saturation fault has already been emitted for this episode.
    ///
    /// Prevents one event per command while saturated: the fault is a state,
    /// so it is raised on the crossing and cleared on the crossing back.
    #[serde(default)]
    fault_raised: bool,
}

/// How long an unacknowledged result waits before it is published again.
///
/// Protocol §5.14: on a timer, not only on reconnect, because the failure this
/// covers is an edge that crashes and restarts while the device's socket never
/// drops.
pub const COMMAND_RESULT_RETRY_MS: u64 = 15_000;

impl PendingLedger {
    /// How many results are held.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether nothing is held.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Whether the ledger currently permits actuation.
    #[must_use]
    pub fn state(&self) -> LedgerState {
        if self.entries.len() >= ACTUATION_THRESHOLD {
            LedgerState::Saturated
        } else {
            LedgerState::Normal
        }
    }

    /// Whether a new dose may be authorised.
    #[must_use]
    pub fn permits_actuation(&self) -> bool {
        self.state() == LedgerState::Normal
    }

    /// Whether the saturation fault is currently raised.
    #[must_use]
    pub const fn fault_raised(&self) -> bool {
        self.fault_raised
    }

    /// The results held, oldest first.
    #[must_use]
    pub fn entries(&self) -> &[PendingResult] {
        &self.entries
    }

    /// The total volume this ledger still has to tell the edge about.
    ///
    /// Point 2 of the ADR-014 list in aggregate form: while the ledger is
    /// saturated this is the water the edge has not yet been able to count, and
    /// it is reported alongside the fault so the outage is quantified rather
    /// than merely flagged.
    #[must_use]
    pub fn unacknowledged_ml(&self) -> f32 {
        self.entries
            .iter()
            .filter_map(|entry| entry.result.delivered_ml)
            .filter(|ml| ml.is_finite())
            .sum()
    }

    /// Records a result, or refuses when there is genuinely no room.
    ///
    /// **Never evicts.** A `command_id` already held is updated in place rather
    /// than duplicated, so a republished result cannot consume a second slot.
    ///
    /// # Errors
    ///
    /// [`LedgerFull::NoCapacity`] when every slot including the reserve is
    /// taken. The caller publishes a rejection un-ledgered in that case, which
    /// is safe for that one class of result and no other.
    pub fn insert(&mut self, result: CommandResult) -> Result<(), LedgerFull> {
        if let Some(existing) = self
            .entries
            .iter_mut()
            .find(|entry| entry.result.command_id == result.command_id)
        {
            existing.result = result;
            return Ok(());
        }
        if self.entries.len() >= PENDING_RESULT_CAPACITY {
            return Err(LedgerFull::NoCapacity);
        }
        self.entries.push(PendingResult {
            result,
            last_published_ms: None,
            publish_count: 0,
        });
        Ok(())
    }

    /// Applies a `command.result.ack` (protocol §5.14).
    ///
    /// Returns whether an entry was removed. An acknowledgement for a
    /// `command_id` not held is a no-op and MUST NOT clear any other result.
    pub fn acknowledge(&mut self, command_id: CommandId) -> bool {
        let before = self.entries.len();
        self.entries
            .retain(|entry| entry.result.command_id != command_id);
        before != self.entries.len()
    }

    /// Marks an entry as published at `monotonic_ms`.
    pub fn mark_published(&mut self, command_id: CommandId, monotonic_ms: u64) {
        if let Some(entry) = self
            .entries
            .iter_mut()
            .find(|entry| entry.result.command_id == command_id)
        {
            entry.last_published_ms = Some(monotonic_ms);
            entry.publish_count = entry.publish_count.saturating_add(1);
        }
    }

    /// The results due for republication at `monotonic_ms`.
    ///
    /// Everything never published, plus everything whose last publication is
    /// older than [`COMMAND_RESULT_RETRY_MS`].
    #[must_use]
    pub fn due(&self, monotonic_ms: u64) -> Vec<&CommandResult> {
        self.entries
            .iter()
            .filter(|entry| match entry.last_published_ms {
                None => true,
                Some(last) => monotonic_ms.saturating_sub(last) >= COMMAND_RESULT_RETRY_MS,
            })
            .map(|entry| &entry.result)
            .collect()
    }

    /// Latches the saturation fault, returning `true` on the crossing only.
    ///
    /// The fault is a *state*, so one event is emitted when the ledger becomes
    /// saturated and one when it recovers — not one per refused command, which
    /// would flood the audit tier with the same fact.
    pub fn raise_fault_if_crossed(&mut self) -> bool {
        let saturated = self.state() == LedgerState::Saturated;
        if saturated && !self.fault_raised {
            self.fault_raised = true;
            return true;
        }
        false
    }

    /// Clears the saturation fault, returning `true` on the crossing back only.
    pub fn clear_fault_if_crossed(&mut self) -> bool {
        let saturated = self.state() == LedgerState::Saturated;
        if !saturated && self.fault_raised {
            self.fault_raised = false;
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rhizo_mqtt_contract::payload::{CommandOrigin, CommandStatus};
    use uuid::Uuid;

    fn result(n: u128, delivered: f32) -> CommandResult {
        CommandResult {
            command_id: CommandId::from_uuid(Uuid::from_u128(n)),
            status: CommandStatus::Completed,
            requested_ml: delivered,
            delivered_ml: Some(delivered),
            duration_ms: Some(1000),
            clamped: false,
            reason: None,
            delivered_today_ml: delivered,
            origin: CommandOrigin::EdgeCommand,
            detail: None,
        }
    }

    fn fill(ledger: &mut PendingLedger, count: usize) {
        for n in 0..count {
            ledger
                .insert(result(n as u128 + 1, 10.0))
                .expect("within capacity");
        }
    }

    #[test]
    fn actuation_is_refused_one_slot_before_the_ledger_is_full() {
        let mut ledger = PendingLedger::default();
        fill(&mut ledger, ACTUATION_THRESHOLD - 1);
        assert!(ledger.permits_actuation());
        fill_one_more(&mut ledger, ACTUATION_THRESHOLD);
        assert!(!ledger.permits_actuation());
        assert_eq!(ledger.state(), LedgerState::Saturated);
    }

    fn fill_one_more(ledger: &mut PendingLedger, n: usize) {
        ledger
            .insert(result(n as u128 + 1, 10.0))
            .expect("within capacity");
    }

    /// The reserve exists so the refusal a saturated ledger issues can itself
    /// be recorded. Without it the device reaches a state where it cannot even
    /// say why it refused.
    #[test]
    fn the_reserved_slot_holds_the_refusal_the_saturation_causes() {
        let mut ledger = PendingLedger::default();
        fill(&mut ledger, ACTUATION_THRESHOLD);
        assert!(!ledger.permits_actuation());
        assert!(ledger.insert(result(9_000, 0.0)).is_ok());
        assert_eq!(ledger.len(), PENDING_RESULT_CAPACITY);
        assert_eq!(
            ledger.insert(result(9_001, 0.0)),
            Err(LedgerFull::NoCapacity)
        );
    }

    /// ADR-014 point 6. The one property most likely to be quietly "improved"
    /// into an eviction policy by someone who has read M9-017 first.
    #[test]
    fn safety_006_a_full_ledger_never_discards_an_unacknowledged_result() {
        let mut ledger = PendingLedger::default();
        fill(&mut ledger, PENDING_RESULT_CAPACITY);
        let before: Vec<_> = ledger
            .entries()
            .iter()
            .map(|entry| entry.result.command_id)
            .collect();
        assert_eq!(
            ledger.insert(result(99_999, 40.0)),
            Err(LedgerFull::NoCapacity)
        );
        let after: Vec<_> = ledger
            .entries()
            .iter()
            .map(|entry| entry.result.command_id)
            .collect();
        assert_eq!(before, after, "a refused insert must not evict anything");
        assert_eq!(ledger.unacknowledged_ml(), 160.0);
    }

    #[test]
    fn acknowledgement_frees_exactly_one_slot_and_restores_actuation() {
        let mut ledger = PendingLedger::default();
        fill(&mut ledger, ACTUATION_THRESHOLD);
        assert!(ledger.raise_fault_if_crossed());
        assert!(!ledger.raise_fault_if_crossed(), "fault latches once");
        assert!(ledger.acknowledge(CommandId::from_uuid(Uuid::from_u128(1))));
        assert_eq!(ledger.len(), ACTUATION_THRESHOLD - 1);
        assert!(ledger.permits_actuation());
        assert!(ledger.clear_fault_if_crossed());
        assert!(!ledger.clear_fault_if_crossed());
    }

    /// Protocol §5.14 point 2: an acknowledgement the device is not holding is
    /// the ordinary outcome of a duplicate delivery, and must clear nothing.
    #[test]
    fn an_unknown_acknowledgement_is_a_no_op() {
        let mut ledger = PendingLedger::default();
        fill(&mut ledger, 3);
        assert!(!ledger.acknowledge(CommandId::from_uuid(Uuid::from_u128(4242))));
        assert_eq!(ledger.len(), 3);
    }

    #[test]
    fn a_republished_result_updates_in_place_and_takes_no_second_slot() {
        let mut ledger = PendingLedger::default();
        ledger.insert(result(1, 10.0)).expect("first insert");
        ledger.insert(result(1, 20.0)).expect("update in place");
        assert_eq!(ledger.len(), 1);
        assert_eq!(ledger.unacknowledged_ml(), 20.0);
    }

    #[test]
    fn only_the_retry_interval_makes_a_result_due_again() {
        let mut ledger = PendingLedger::default();
        ledger.insert(result(1, 10.0)).expect("insert");
        assert_eq!(ledger.due(0).len(), 1, "never published is always due");
        ledger.mark_published(CommandId::from_uuid(Uuid::from_u128(1)), 1_000);
        assert!(ledger.due(1_000 + COMMAND_RESULT_RETRY_MS - 1).is_empty());
        assert_eq!(ledger.due(1_000 + COMMAND_RESULT_RETRY_MS).len(), 1);
    }

    #[test]
    fn the_ledger_survives_a_round_trip_at_the_saturation_boundary() {
        let mut ledger = PendingLedger::default();
        fill(&mut ledger, ACTUATION_THRESHOLD);
        assert!(ledger.raise_fault_if_crossed());
        let encoded = serde_json::to_vec(&ledger).expect("ledger encodes");
        let restored: PendingLedger = serde_json::from_slice(&encoded).expect("ledger decodes");
        assert_eq!(restored, ledger);
        assert!(!restored.permits_actuation());
        assert!(restored.fault_raised(), "the fault must not be re-emitted");
    }
}
