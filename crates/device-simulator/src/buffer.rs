//! The bounded device-side history buffer, and its replay.
//!
//! [ADR-014](../../../../docs/adr/014-failure-and-retry-policy.md) and
//! [offline-autonomy.md](../../../../docs/architecture/offline-autonomy.md) §6
//! specify a bounded ring with tiered retention and explicit gap reporting. An
//! ESP32 cannot retain unbounded history, and pretending otherwise would be a
//! design that fails quietly in the field.
//!
//! # The mechanism this module exists for
//!
//! **`event_id` is generated once, at buffering time, and never regenerated on
//! replay.** That is the whole of it. A device that regenerated ids would defeat
//! the edge's deduplication and create duplicate watering history — SAFETY-016's
//! central failure, and the one that turns "the plant was watered once" into
//! "the plant was watered three times" in the record the operator trusts.
//!
//! # Audit outranks telemetry
//!
//! The record of what the machine did to a living plant is not optional; a
//! missing moisture sample is a missing pixel in a chart. Audit events are never
//! evicted to make room for telemetry, and the two tiers have separate
//! capacities so a telemetry flood cannot crowd out an autonomous dose.
//!
//! # A gap is data
//!
//! When eviction loses events the range and count are recorded and replayed as
//! a `history.gap` event — reported, stored, and visible in the plant's history,
//! never silently absorbed (SAFETY-020).

use rhizo_mqtt_contract::payload::{
    BufferedEvent, DeviceEventBatch, EventDetail, EventKind, EventTier,
};
use rhizo_mqtt_contract::{EventId, UtcMillis};

use crate::state::{EventBufferState, GapMetadata};

/// Audit-tier capacity. Never reduced to make room for telemetry.
pub const AUDIT_CAPACITY: usize = 64;
/// Telemetry-tier capacity.
pub const TELEMETRY_CAPACITY: usize = 256;
/// How many events go in one replay batch.
///
/// Small enough that a full buffer is several batches — so `complete: true` on
/// the last one is a property with something to be last *of*, rather than a flag
/// on the only message ever sent.
pub const REPLAY_BATCH: usize = 32;

/// What a `push` did.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Buffered {
    /// Stored with room to spare.
    Stored {
        /// The sequence it was given.
        device_seq: u64,
    },
    /// Stored, and an older event of the same tier was lost to make room.
    Evicted {
        /// The sequence it was given.
        device_seq: u64,
        /// The sequence that was lost.
        lost_seq: u64,
        /// Which tier lost it.
        lost_tier: EventTier,
    },
}

impl Buffered {
    /// The sequence the new event was given.
    #[must_use]
    pub const fn device_seq(self) -> u64 {
        match self {
            Self::Stored { device_seq } | Self::Evicted { device_seq, .. } => device_seq,
        }
    }
}

impl EventBufferState {
    /// How many events of a tier are held.
    #[must_use]
    pub fn count(&self, tier: EventTier) -> usize {
        self.events.iter().filter(|e| e.tier == tier).count()
    }

    /// How many events are held in total, excluding any pending gap marker.
    #[must_use]
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// Whether nothing is buffered and no gap is pending.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.events.is_empty() && self.gap.is_none()
    }

    /// The pending gap, if history has been lost since the last acknowledgement.
    #[must_use]
    pub const fn gap(&self) -> Option<&GapMetadata> {
        self.gap.as_ref()
    }

    /// The capacity of a tier.
    #[must_use]
    pub const fn capacity(tier: EventTier) -> usize {
        match tier {
            EventTier::Audit => AUDIT_CAPACITY,
            // An unrecognised tier is treated as the cheaper one: a future
            // tier this build does not understand must not be able to claim
            // the audit reservation.
            EventTier::Telemetry | EventTier::Unknown => TELEMETRY_CAPACITY,
        }
    }

    /// Buffers an event, evicting within its own tier if that tier is full.
    ///
    /// `event_id` is supplied by the caller and stored as given. It is generated
    /// **once**, here at buffering time, and every replay uses this same value.
    ///
    /// Eviction is always within the event's own tier. A telemetry event can
    /// never displace an audit event, which is what makes "audit survives a
    /// telemetry flood" a property of the data structure rather than of the
    /// order things happened to arrive.
    pub fn push(
        &mut self,
        event_id: EventId,
        tier: EventTier,
        kind: EventKind,
        monotonic_ms: u64,
        device_time_ms: Option<UtcMillis>,
        detail: EventDetail,
    ) -> Buffered {
        let device_seq = self.next_seq;
        self.next_seq = self.next_seq.saturating_add(1);

        let evicted = if self.count(tier) >= Self::capacity(tier) {
            self.evict_oldest(tier)
        } else {
            None
        };

        self.events.push(BufferedEvent {
            event_id,
            device_seq,
            tier,
            kind,
            monotonic_ms,
            device_time_ms,
            detail,
        });

        match evicted {
            Some(lost_seq) => {
                self.record_gap(lost_seq, tier, monotonic_ms, device_time_ms);
                Buffered::Evicted {
                    device_seq,
                    lost_seq,
                    lost_tier: tier,
                }
            }
            None => Buffered::Stored { device_seq },
        }
    }

    /// Removes the oldest event of a tier, returning its sequence.
    fn evict_oldest(&mut self, tier: EventTier) -> Option<u64> {
        let index = self.events.iter().position(|e| e.tier == tier)?;
        Some(self.events.remove(index).device_seq)
    }

    /// Records or extends the pending gap.
    ///
    /// One gap marker spans a run of losses, and it keeps the `event_id` it was
    /// created with. Emitting a fresh marker per lost event would flood the very
    /// buffer that is already full; keeping one stable id means a replayed gap
    /// deduplicates like any other event.
    fn record_gap(
        &mut self,
        lost_seq: u64,
        lost_tier: EventTier,
        monotonic_ms: u64,
        device_time_ms: Option<UtcMillis>,
    ) {
        match self.gap.as_mut() {
            Some(gap) => {
                gap.to_seq = gap.to_seq.max(lost_seq);
                gap.from_seq = gap.from_seq.min(lost_seq);
                gap.lost_count = gap.lost_count.saturating_add(1);
                // Audit loss outranks telemetry loss in the report: losing the
                // record of a dose matters more than losing a sample, and the
                // marker should say the worse thing that happened.
                if lost_tier == EventTier::Audit {
                    gap.lost_tier = EventTier::Audit;
                }
            }
            None => {
                self.gap = Some(GapMetadata {
                    event_id: EventId::from_uuid(gap_id(lost_seq, monotonic_ms)),
                    device_seq: self.next_seq,
                    monotonic_ms,
                    device_time_ms,
                    from_seq: lost_seq,
                    to_seq: lost_seq,
                    lost_count: 1,
                    lost_tier,
                });
                self.next_seq = self.next_seq.saturating_add(1);
            }
        }
    }

    /// Everything to replay, oldest first, with any gap marker in sequence.
    #[must_use]
    pub fn replay_events(&self) -> Vec<BufferedEvent> {
        let mut events = self.events.clone();
        if let Some(gap) = self.gap.as_ref() {
            events.push(BufferedEvent {
                event_id: gap.event_id,
                device_seq: gap.device_seq,
                tier: EventTier::Audit,
                kind: EventKind::HistoryGap,
                monotonic_ms: gap.monotonic_ms,
                device_time_ms: gap.device_time_ms,
                detail: EventDetail::Gap {
                    from_seq: gap.from_seq,
                    to_seq: gap.to_seq,
                    lost_count: gap.lost_count,
                    lost_tier: gap.lost_tier,
                },
            });
        }
        events.sort_by_key(|e| e.device_seq);
        events
    }

    /// Splits the buffered history into replay batches.
    ///
    /// Always returns at least one batch, and the last always sets
    /// `complete: true`. An empty buffer still produces one complete batch,
    /// because the edge holds every plant on a reconnecting device in
    /// `Uncertain` until it sees that flag — a device with nothing to say still
    /// has to say so, or the plant waits forever (protocol §5.4, SAFETY-016).
    #[must_use]
    pub fn replay_batches(&self, batch_size: usize) -> Vec<DeviceEventBatch> {
        let events = self.replay_events();
        let size = batch_size.max(1);
        if events.is_empty() {
            return vec![DeviceEventBatch {
                replay: true,
                complete: true,
                events: Vec::new(),
            }];
        }
        let chunks: Vec<_> = events.chunks(size).collect();
        let last = chunks.len().saturating_sub(1);
        chunks
            .into_iter()
            .enumerate()
            .map(|(index, chunk)| DeviceEventBatch {
                replay: true,
                complete: index == last,
                events: chunk.to_vec(),
            })
            .collect()
    }

    /// The highest sequence a replay would cover.
    #[must_use]
    pub fn highest_seq(&self) -> Option<u64> {
        self.replay_events().last().map(|e| e.device_seq)
    }

    /// Discards everything the edge has acknowledged.
    ///
    /// Until this is called, events are retained and replayed again — so an
    /// edge that crashes mid-reconciliation loses nothing; it simply replays.
    pub fn acknowledge(&mut self, through_seq: u64) {
        self.events.retain(|e| e.device_seq > through_seq);
        if self
            .gap
            .as_ref()
            .is_some_and(|gap| gap.device_seq <= through_seq)
        {
            self.gap = None;
        }
        self.pending_ack_through_seq = Some(
            self.pending_ack_through_seq
                .map_or(through_seq, |previous| previous.max(through_seq)),
        );
    }
}

/// A deterministic identifier for a gap marker.
///
/// Derived from the loss it describes rather than drawn at random, so the same
/// gap regenerates the same id if it is ever rebuilt from the same facts — and
/// so a test can assert on it. Uniqueness comes from the sequence, which is
/// monotonic within a boot.
fn gap_id(lost_seq: u64, monotonic_ms: u64) -> uuid::Uuid {
    let mut bytes = [0u8; 16];
    bytes[..8].copy_from_slice(&lost_seq.to_be_bytes());
    bytes[8..].copy_from_slice(&monotonic_ms.to_be_bytes());
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    uuid::Uuid::from_bytes(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn event_id(n: u128) -> EventId {
        EventId::from_uuid(Uuid::from_u128(n))
    }

    fn push(state: &mut EventBufferState, n: u128, tier: EventTier) -> Buffered {
        state.push(
            event_id(n),
            tier,
            match tier {
                EventTier::Audit => EventKind::PolicyActivated,
                _ => EventKind::Unknown(String::from("telemetry.sample")),
            },
            n as u64 * 1_000,
            None,
            match tier {
                EventTier::Audit => EventDetail::PolicyActivated { policy_version: 7 },
                _ => EventDetail::Unknown,
            },
        )
    }

    #[test]
    fn events_buffer_in_sequence_order_within_a_boot() {
        let mut state = EventBufferState::default();
        assert!(state.is_empty());
        for n in 0..5 {
            assert_eq!(push(&mut state, n, EventTier::Audit).device_seq(), n as u64);
        }
        let replayed = state.replay_events();
        assert_eq!(replayed.len(), 5);
        assert!(
            replayed
                .windows(2)
                .all(|w| w[0].device_seq < w[1].device_seq),
            "device_seq is monotonic within a boot"
        );
    }

    /// SAFETY-016's central mechanism.
    #[test]
    fn safety_016_event_id_is_stable_across_every_replay() {
        let mut state = EventBufferState::default();
        for n in 0..40 {
            push(&mut state, n, EventTier::Audit);
        }
        let ids = |state: &EventBufferState| -> Vec<EventId> {
            state
                .replay_batches(REPLAY_BATCH)
                .into_iter()
                .flat_map(|b| b.events)
                .map(|e| e.event_id)
                .collect()
        };
        let first = ids(&state);
        assert_eq!(first, ids(&state));
        assert_eq!(first, ids(&state), "and again, and again");
        assert_eq!(
            first.len(),
            40,
            "a replay covers everything, not a shrinking suffix"
        );
    }

    #[test]
    fn replaying_three_times_yields_one_logical_event_per_id() {
        let mut state = EventBufferState::default();
        for n in 0..10 {
            push(&mut state, n, EventTier::Audit);
        }
        let mut seen: std::collections::HashMap<EventId, usize> = std::collections::HashMap::new();
        for _ in 0..3 {
            for batch in state.replay_batches(4) {
                batch.validate().expect("no duplicate ids within a batch");
                for event in batch.events {
                    *seen.entry(event.event_id).or_default() += 1;
                }
            }
        }
        assert_eq!(seen.len(), 10, "ten distinct events");
        assert!(
            seen.values().all(|count| *count == 3),
            "each replayed three times, under the same id every time — which is \
             what lets the edge collapse them to one"
        );
    }

    // ----------------------------------------------------------- the tiers

    /// SAFETY-020: audit events are never evicted to make room for telemetry.
    #[test]
    fn safety_020_telemetry_never_evicts_audit() {
        let mut state = EventBufferState::default();
        for n in 0..AUDIT_CAPACITY as u128 {
            push(&mut state, n, EventTier::Audit);
        }
        let audit_ids: Vec<_> = state
            .events
            .iter()
            .filter(|e| e.tier == EventTier::Audit)
            .map(|e| e.event_id)
            .collect();

        // Ten times the telemetry capacity.
        for n in 0..(TELEMETRY_CAPACITY as u128 * 10) {
            push(&mut state, 1_000 + n, EventTier::Telemetry);
        }

        let survivors: Vec<_> = state
            .events
            .iter()
            .filter(|e| e.tier == EventTier::Audit)
            .map(|e| e.event_id)
            .collect();
        assert_eq!(
            survivors, audit_ids,
            "the record of what the machine did to a plant is not optional"
        );
        assert_eq!(state.count(EventTier::Audit), AUDIT_CAPACITY);
        assert_eq!(state.count(EventTier::Telemetry), TELEMETRY_CAPACITY);
    }

    #[test]
    fn a_full_audit_tier_evicts_its_own_oldest_and_reports_the_loss() {
        let mut state = EventBufferState::default();
        for n in 0..AUDIT_CAPACITY as u128 {
            assert!(matches!(
                push(&mut state, n, EventTier::Audit),
                Buffered::Stored { .. }
            ));
        }
        let outcome = push(&mut state, 999, EventTier::Audit);
        assert!(matches!(
            outcome,
            Buffered::Evicted {
                lost_seq: 0,
                lost_tier: EventTier::Audit,
                ..
            }
        ));
        assert_eq!(state.count(EventTier::Audit), AUDIT_CAPACITY);

        let gap = state.gap().expect("a lost audit event is a reported gap");
        assert_eq!(gap.lost_tier, EventTier::Audit);
        assert_eq!(gap.lost_count, 1);
        assert_eq!(gap.from_seq, 0);
    }

    // ------------------------------------------------------------ the gap

    #[test]
    fn overflow_emits_a_gap_with_the_correct_range_and_count() {
        let mut state = EventBufferState::default();
        let overflow = 5u128;
        for n in 0..(TELEMETRY_CAPACITY as u128 + overflow) {
            push(&mut state, n, EventTier::Telemetry);
        }
        let gap = state.gap().expect("eviction must be reported");
        assert_eq!(gap.lost_count, overflow as u32);
        assert_eq!(gap.from_seq, 0);
        assert_eq!(gap.to_seq, overflow as u64 - 1);
        assert_eq!(gap.lost_tier, EventTier::Telemetry);

        // ...and it appears in the replay as a `history.gap` audit event.
        let replayed = state.replay_events();
        let marker = replayed
            .iter()
            .find(|e| e.kind == EventKind::HistoryGap)
            .expect("the gap is replayed, not merely recorded");
        assert_eq!(marker.tier, EventTier::Audit);
        assert_eq!(
            marker.detail,
            EventDetail::Gap {
                from_seq: 0,
                to_seq: overflow as u64 - 1,
                lost_count: overflow as u32,
                lost_tier: EventTier::Telemetry,
            }
        );
    }

    #[test]
    fn a_run_of_losses_is_one_marker_with_a_stable_id_not_a_flood_of_them() {
        let mut state = EventBufferState::default();
        for n in 0..(TELEMETRY_CAPACITY as u128 + 50) {
            push(&mut state, n, EventTier::Telemetry);
        }
        let markers: Vec<_> = state
            .replay_events()
            .into_iter()
            .filter(|e| e.kind == EventKind::HistoryGap)
            .collect();
        assert_eq!(markers.len(), 1, "one marker spans the run of losses");
        assert_eq!(
            state.replay_events()[0].event_id,
            state.replay_events()[0].event_id
        );

        let id = markers[0].event_id;
        for _ in 0..3 {
            let again: Vec<_> = state
                .replay_events()
                .into_iter()
                .filter(|e| e.kind == EventKind::HistoryGap)
                .collect();
            assert_eq!(again[0].event_id, id, "the marker keeps its identity");
        }
    }

    #[test]
    fn losing_an_audit_event_upgrades_the_reported_tier() {
        let mut state = EventBufferState::default();
        for n in 0..(TELEMETRY_CAPACITY as u128 + 1) {
            push(&mut state, n, EventTier::Telemetry);
        }
        assert_eq!(state.gap().unwrap().lost_tier, EventTier::Telemetry);

        for n in 0..(AUDIT_CAPACITY as u128 + 1) {
            push(&mut state, 10_000 + n, EventTier::Audit);
        }
        assert_eq!(
            state.gap().unwrap().lost_tier,
            EventTier::Audit,
            "the marker reports the worse loss, not the most recent one"
        );
    }

    // ------------------------------------------------------------- replay

    #[test]
    fn replay_is_ordered_batched_and_complete_only_at_the_end() {
        let mut state = EventBufferState::default();
        for n in 0..70 {
            push(&mut state, n, EventTier::Audit);
        }
        let batches = state.replay_batches(REPLAY_BATCH);
        assert_eq!(batches.len(), 3, "70 events in batches of 32");
        assert!(batches.iter().all(|b| b.replay));
        assert!(
            batches[..2].iter().all(|b| !b.complete),
            "only the final batch is complete"
        );
        assert!(batches[2].complete);

        let sequences: Vec<_> = batches
            .iter()
            .flat_map(|b| b.events.iter().map(|e| e.device_seq))
            .collect();
        let mut sorted = sequences.clone();
        sorted.sort_unstable();
        assert_eq!(sequences, sorted, "replay is in device_seq order");
    }

    #[test]
    fn an_empty_buffer_still_replays_one_complete_batch() {
        let state = EventBufferState::default();
        let batches = state.replay_batches(REPLAY_BATCH);
        assert_eq!(batches.len(), 1);
        assert!(batches[0].events.is_empty());
        assert!(
            batches[0].complete,
            "the edge holds the plant in Uncertain until it sees this; a device \
             with nothing to say still has to say so"
        );
    }

    #[test]
    fn a_batch_size_of_zero_is_treated_as_one_rather_than_dividing_by_it() {
        let mut state = EventBufferState::default();
        push(&mut state, 1, EventTier::Audit);
        let batches = state.replay_batches(0);
        assert_eq!(batches.len(), 1);
        assert!(batches[0].complete);
    }

    // ------------------------------------------------------ acknowledgement

    #[test]
    fn unacknowledged_events_are_retained_and_replayed_again() {
        let mut state = EventBufferState::default();
        for n in 0..10 {
            push(&mut state, n, EventTier::Audit);
        }
        let first = state.replay_events().len();
        // No acknowledgement: an edge that crashed mid-reconciliation gets the
        // whole thing again.
        assert_eq!(state.replay_events().len(), first);
        assert_eq!(state.replay_events().len(), first);
    }

    #[test]
    fn acknowledgement_discards_only_what_was_acknowledged() {
        let mut state = EventBufferState::default();
        for n in 0..10 {
            push(&mut state, n, EventTier::Audit);
        }
        state.acknowledge(4);
        let remaining: Vec<_> = state
            .replay_events()
            .into_iter()
            .map(|e| e.device_seq)
            .collect();
        assert_eq!(remaining, vec![5, 6, 7, 8, 9]);
        assert_eq!(state.pending_ack_through_seq, Some(4));

        state.acknowledge(9);
        assert!(state.is_empty());
    }

    #[test]
    fn an_acknowledgement_never_moves_backwards() {
        let mut state = EventBufferState::default();
        for n in 0..10 {
            push(&mut state, n, EventTier::Audit);
        }
        state.acknowledge(7);
        state.acknowledge(2);
        assert_eq!(
            state.pending_ack_through_seq,
            Some(7),
            "a late, lower acknowledgement must not un-acknowledge history"
        );
    }

    #[test]
    fn a_gap_is_cleared_only_once_it_has_been_acknowledged() {
        let mut state = EventBufferState::default();
        for n in 0..(TELEMETRY_CAPACITY as u128 + 3) {
            push(&mut state, n, EventTier::Telemetry);
        }
        let gap_seq = state.gap().unwrap().device_seq;
        state.acknowledge(gap_seq - 1);
        assert!(state.gap().is_some(), "not yet");
        state.acknowledge(gap_seq);
        assert!(state.gap().is_none());
    }

    #[test]
    fn the_buffer_round_trips_through_json_with_its_ids_intact() {
        let mut state = EventBufferState::default();
        for n in 0..(TELEMETRY_CAPACITY as u128 + 2) {
            push(&mut state, n, EventTier::Telemetry);
        }
        push(&mut state, 9_999, EventTier::Audit);

        let encoded = serde_json::to_vec(&state).unwrap();
        let decoded: EventBufferState = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(decoded, state);
        assert_eq!(
            decoded.replay_events(),
            state.replay_events(),
            "ids and order survive persistence, which is what a reboot needs"
        );
    }
}
