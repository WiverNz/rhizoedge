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

/// What an acknowledgement did.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AckOutcome {
    /// Applied; the covered prefix was discarded.
    Applied {
        /// The sequence acknowledged through.
        through_seq: u64,
        /// How many buffered events were discarded.
        removed: usize,
    },
    /// At or below one already applied, so a no-op.
    NotNewer {
        /// The sequence offered.
        through_seq: u64,
    },
    /// Beyond any sequence this device ever allocated; nothing was deleted.
    BeyondKnown {
        /// The sequence offered.
        through_seq: u64,
        /// The highest sequence this device has allocated.
        highest: u64,
    },
}

impl AckOutcome {
    /// Whether the buffer changed.
    #[must_use]
    pub const fn changed(self) -> bool {
        matches!(self, Self::Applied { removed, .. } if removed > 0)
    }

    /// A stable label for logs.
    #[must_use]
    pub const fn reason(self) -> &'static str {
        match self {
            Self::Applied { .. } => "applied",
            Self::NotNewer { .. } => "not_newer",
            Self::BeyondKnown { .. } => "beyond_known",
        }
    }
}

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
                // No `device_seq` yet. A marker takes its sequence when it is
                // sealed, not when the first loss happens — see `seal_gap`.
                self.gap = Some(GapMetadata {
                    event_id: EventId::from_uuid(gap_id(lost_seq, monotonic_ms)),
                    monotonic_ms,
                    device_time_ms,
                    from_seq: lost_seq,
                    to_seq: lost_seq,
                    lost_count: 1,
                    lost_tier,
                });
            }
        }
    }

    /// Seals the pending gap into a real buffered event, ready to replay.
    ///
    /// Called immediately before a replay is built, and this is the whole reason
    /// a gap is accumulated as *metadata* first. While a run of losses is still
    /// growing, nobody has seen it, so widening its range and raising its count
    /// is free. Once it has been sent it must never change again: the edge
    /// deduplicates on `event_id`, so a marker whose range grew after it was
    /// published would leave the edge holding the *smaller* first version for
    /// ever, silently under-reporting how much history was lost. Losses after a
    /// seal therefore open a new marker rather than widening the sent one.
    ///
    /// # Why the sequence is allocated here and not at the first loss
    ///
    /// Because acknowledgement is cumulative. A marker that took its sequence
    /// at the moment of the first loss would sit *below* events buffered
    /// afterwards, so an acknowledgement covering those events would also cover
    /// a marker the edge had never been sent — and, being cumulative, no later
    /// acknowledgement could ever cover it again. The marker would be
    /// undeletable, and the loss it describes would never reach the edge.
    ///
    /// Taking the sequence at seal time makes that impossible by construction:
    /// a marker's sequence is always above every sequence the edge could have
    /// acknowledged, because it did not exist when the edge spoke. The range of
    /// the loss is carried by `from_seq`/`to_seq`, which is where it belongs —
    /// the marker's own sequence only ever meant "where it sits in the replay".
    ///
    /// The push deliberately bypasses the tier capacity check. A marker is one
    /// small event per replay, and evicting an audit event to make room for the
    /// record of an eviction would be a loop with nothing to show for it. The
    /// overshoot is corrected by the next ordinary `push`, which enforces the
    /// capacity again.
    pub fn seal_gap(&mut self) {
        let Some(gap) = self.gap.take() else {
            return;
        };
        let device_seq = self.next_seq;
        self.next_seq = self.next_seq.saturating_add(1);
        self.events.push(BufferedEvent {
            event_id: gap.event_id,
            device_seq,
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
        self.events.sort_by_key(|e| e.device_seq);
    }

    /// Everything sealed and waiting to be replayed, oldest first.
    ///
    /// A gap still accumulating is **not** included: it has no final range yet.
    /// [`seal_gap`](Self::seal_gap) is what makes it replayable.
    #[must_use]
    pub fn replay_events(&self) -> Vec<BufferedEvent> {
        let mut events = self.events.clone();
        events.sort_by_key(|e| e.device_seq);
        events
    }

    /// The highest sequence this device has ever allocated.
    ///
    /// The ceiling an acknowledgement may not exceed. `next_seq` is the sequence
    /// the *next* event will take, so the highest allocated is one below it.
    #[must_use]
    pub const fn highest_allocated_seq(&self) -> Option<u64> {
        self.next_seq.checked_sub(1)
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

    /// Whether a gap is still accumulating and has never been sent.
    #[must_use]
    pub const fn has_pending_gap(&self) -> bool {
        self.gap.is_some()
    }

    /// Applies a cumulative acknowledgement, returning what it did.
    ///
    /// Until an acknowledgement arrives, events are retained and replayed
    /// again — so an edge that crashes mid-reconciliation loses nothing; it
    /// simply replays.
    ///
    /// Three refusals, all fail-closed:
    ///
    /// - **Beyond what this device has allocated.** A sequence the device never
    ///   issued cannot have been persisted by anyone. Honouring it would delete
    ///   the entire buffer on a typo or a corrupted field, so the whole
    ///   acknowledgement is refused and nothing is deleted.
    /// - **Older than one already applied.** Cumulative acknowledgement only
    ///   moves forward; a delayed, lower one is a no-op rather than a
    ///   regression.
    /// - Anything else is idempotent by construction: applying the same
    ///   acknowledgement twice retains the same suffix.
    ///
    /// A pending, unsealed gap is never affected: it has not been sent, so it
    /// cannot have been acknowledged.
    pub fn acknowledge(&mut self, through_seq: u64) -> AckOutcome {
        match self.highest_allocated_seq() {
            Some(highest) if through_seq > highest => {
                return AckOutcome::BeyondKnown {
                    through_seq,
                    highest,
                };
            }
            // Nothing has ever been allocated, so nothing can be acknowledged.
            None => {
                return AckOutcome::BeyondKnown {
                    through_seq,
                    highest: 0,
                };
            }
            Some(_) => {}
        }
        if self
            .pending_ack_through_seq
            .is_some_and(|previous| through_seq <= previous)
        {
            return AckOutcome::NotNewer { through_seq };
        }
        let before = self.events.len();
        self.events.retain(|e| e.device_seq > through_seq);
        self.pending_ack_through_seq = Some(through_seq);
        AckOutcome::Applied {
            through_seq,
            removed: before - self.events.len(),
        }
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

        // ...and once sealed it appears in the replay as a `history.gap`
        // audit event. Sealing is what turns the accumulator into an event; a
        // gap still growing has no final range to report.
        state.seal_gap();
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
        state.seal_gap();
        let markers: Vec<_> = state
            .replay_events()
            .into_iter()
            .filter(|e| e.kind == EventKind::HistoryGap)
            .collect();
        assert_eq!(markers.len(), 1, "one marker spans the run of losses");

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

    /// Sealing is what makes a marker immutable, and it has to be, because the
    /// edge deduplicates on `event_id`.
    ///
    /// If a sent marker could keep growing, the edge would hold the version it
    /// saw first — the *smaller* one — for ever, and under-report the loss.
    /// Losses after a seal therefore open a new marker with its own id, rather
    /// than widening the one already on the wire.
    #[test]
    fn a_sealed_marker_never_widens_and_later_losses_open_a_new_one() {
        let mut state = EventBufferState::default();
        for n in 0..(TELEMETRY_CAPACITY as u128 + 5) {
            push(&mut state, n, EventTier::Telemetry);
        }
        state.seal_gap();
        let first = state
            .replay_events()
            .into_iter()
            .find(|e| e.kind == EventKind::HistoryGap)
            .expect("sealed");
        assert!(!state.has_pending_gap(), "sealing consumes the accumulator");

        for n in 0..7u128 {
            push(&mut state, 90_000 + n, EventTier::Telemetry);
        }
        assert!(
            state.has_pending_gap(),
            "further losses open a fresh marker"
        );
        state.seal_gap();

        let markers: Vec<_> = state
            .replay_events()
            .into_iter()
            .filter(|e| e.kind == EventKind::HistoryGap)
            .collect();
        assert_eq!(markers.len(), 2, "two runs of loss, two markers");
        let sealed_again = markers
            .iter()
            .find(|m| m.event_id == first.event_id)
            .expect("the first marker is still there");
        assert_eq!(
            sealed_again.detail, first.detail,
            "a marker that has been replayed must never change what it says"
        );
        assert_ne!(
            markers[0].event_id, markers[1].event_id,
            "the second run is a distinct event, not an edit of the first"
        );
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
        // 70 pushes into a 64-slot tier evict 6, so the buffer holds 64 events
        // plus, once sealed, the one marker describing the loss: 65 in all.
        state.seal_gap();
        let batches = state.replay_batches(REPLAY_BATCH);
        assert_eq!(batches.len(), 3, "65 events in batches of 32");
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

    /// A gap can only be acknowledged once the edge could actually have seen
    /// it — which means once it has been sealed and replayed.
    ///
    /// An acknowledgement is a statement about what the edge has durably
    /// persisted. While a marker is still accumulating it has never left the
    /// device, so no acknowledgement can cover it, however high the sequence:
    /// the accumulator is untouched, and it is sealed and sent as usual.
    #[test]
    fn an_unsent_gap_survives_any_acknowledgement_and_is_still_replayed() {
        let mut state = EventBufferState::default();
        for n in 0..(TELEMETRY_CAPACITY as u128 + 3) {
            push(&mut state, n, EventTier::Telemetry);
        }
        assert!(state.has_pending_gap(), "losses recorded");

        // Acknowledge everything the device has ever allocated.
        let highest = state.highest_allocated_seq().unwrap();
        assert!(state.acknowledge(highest).changed());
        assert!(
            state.has_pending_gap(),
            "an unsent marker cannot have been persisted by the edge"
        );

        state.seal_gap();
        let marker = state
            .replay_events()
            .into_iter()
            .find(|e| e.kind == EventKind::HistoryGap)
            .expect("the gap is still replayed after the sweeping ack");
        assert!(
            marker.device_seq > highest,
            "a marker sealed after an acknowledgement must sit above it, or the \
             cumulative acknowledgement that already passed would have buried it"
        );

        // And it is acknowledgeable now that the edge has actually seen it.
        assert!(state.acknowledge(marker.device_seq).changed());
        assert!(state.replay_events().is_empty());
    }

    /// Once sealed, a marker is acknowledged by sequence like any other event —
    /// there is no special case, which is the point of sealing.
    #[test]
    fn a_sealed_gap_is_cleared_by_an_acknowledgement_that_covers_its_sequence() {
        let mut state = EventBufferState::default();
        for n in 0..(TELEMETRY_CAPACITY as u128 + 3) {
            push(&mut state, n, EventTier::Telemetry);
        }
        state.seal_gap();
        let gap_seq = state
            .replay_events()
            .into_iter()
            .find(|e| e.kind == EventKind::HistoryGap)
            .expect("sealed")
            .device_seq;
        let has_marker = |s: &EventBufferState| {
            s.replay_events()
                .iter()
                .any(|e| e.kind == EventKind::HistoryGap)
        };

        state.acknowledge(gap_seq - 1);
        assert!(has_marker(&state), "not yet");
        state.acknowledge(gap_seq);
        assert!(!has_marker(&state));
    }

    // ---------------------------------------------- acknowledgement rules

    /// Nothing is discarded until an acknowledgement says the edge has it.
    ///
    /// This is the whole reason replay is idempotent: a replay that is built,
    /// sent, and lost costs a retransmission, never a hole in the history.
    #[test]
    fn replay_alone_discards_nothing() {
        let mut state = EventBufferState::default();
        for n in 0..10 {
            push(&mut state, n, EventTier::Audit);
        }
        let first = state.replay_events();
        for _ in 0..3 {
            let _ = state.replay_batches(REPLAY_BATCH);
        }
        assert_eq!(
            state.replay_events(),
            first,
            "an unacknowledged replay leaves the buffer exactly as it was"
        );
        assert_eq!(state.pending_ack_through_seq, None);
    }

    #[test]
    fn an_acknowledgement_discards_the_covered_prefix_and_only_that() {
        let mut state = EventBufferState::default();
        for n in 0..10 {
            push(&mut state, n, EventTier::Audit);
        }
        let seqs: Vec<u64> = state.replay_events().iter().map(|e| e.device_seq).collect();
        let outcome = state.acknowledge(seqs[3]);
        assert_eq!(
            outcome,
            AckOutcome::Applied {
                through_seq: seqs[3],
                removed: 4,
            }
        );
        assert_eq!(
            state
                .replay_events()
                .iter()
                .map(|e| e.device_seq)
                .collect::<Vec<_>>(),
            seqs[4..],
            "everything after the acknowledged prefix is still held"
        );
    }

    /// A retransmitted acknowledgement — the edge published it twice, or the
    /// device's own ack of it was lost — must be a no-op, not a second deletion.
    #[test]
    fn a_duplicate_acknowledgement_is_idempotent() {
        let mut state = EventBufferState::default();
        for n in 0..10 {
            push(&mut state, n, EventTier::Audit);
        }
        let seqs: Vec<u64> = state.replay_events().iter().map(|e| e.device_seq).collect();
        state.acknowledge(seqs[5]);
        let after_first = state.replay_events();

        for _ in 0..3 {
            assert_eq!(
                state.acknowledge(seqs[5]),
                AckOutcome::NotNewer {
                    through_seq: seqs[5]
                }
            );
        }
        assert_eq!(state.replay_events(), after_first);
    }

    /// The fail-closed case: an acknowledgement for a sequence the device never
    /// issued is refused whole, and deletes nothing.
    ///
    /// The alternative — clamping it to the highest known sequence — would turn
    /// a corrupted or misaddressed field into "delete the entire buffer". The
    /// device only ever discards history it can match to something it sent.
    #[test]
    fn an_acknowledgement_beyond_any_issued_sequence_deletes_nothing() {
        let mut state = EventBufferState::default();
        for n in 0..10 {
            push(&mut state, n, EventTier::Audit);
        }
        let before = state.replay_events();
        let highest = state.highest_allocated_seq().unwrap();

        for beyond in [highest + 1, highest + 1_000, u64::MAX] {
            assert_eq!(
                state.acknowledge(beyond),
                AckOutcome::BeyondKnown {
                    through_seq: beyond,
                    highest,
                }
            );
            assert_eq!(state.replay_events(), before, "nothing was deleted");
            assert_eq!(state.pending_ack_through_seq, None, "and nothing recorded");
        }

        // The boundary itself is fine: the highest issued sequence was issued.
        assert!(state.acknowledge(highest).changed());
    }

    /// An empty device has issued nothing, so it can acknowledge nothing —
    /// including sequence 0, which is a real sequence only once allocated.
    #[test]
    fn a_device_that_has_issued_nothing_refuses_every_acknowledgement() {
        let mut state = EventBufferState::default();
        assert_eq!(
            state.acknowledge(0),
            AckOutcome::BeyondKnown {
                through_seq: 0,
                highest: 0,
            }
        );
        assert_eq!(state.pending_ack_through_seq, None);
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
