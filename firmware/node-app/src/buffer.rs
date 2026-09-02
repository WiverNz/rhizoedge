//! The bounded tiered event buffer and its replay (M9-017, SAFETY-016, SAFETY-020).
//!
//! # `event_id` is generated once
//!
//! At buffering time, stored, and never regenerated at publish time. A device
//! that re-minted ids would defeat the edge's deduplication and turn "the plant
//! was watered once" into "the plant was watered three times" in the record the
//! operator trusts. That is SAFETY-016's central failure.
//!
//! # Audit outranks telemetry
//!
//! The two tiers have separate capacities and eviction is always **within** the
//! event's own tier, so a telemetry flood cannot displace the record of an
//! autonomous dose. That is a property of the data structure, not of the order
//! things happened to arrive.
//!
//! # A gap is data, and a sealed gap is immutable
//!
//! Eviction records a marker that accumulates while unsent — range widened,
//! count raised — and is **sealed** immediately before each replay, taking its
//! `device_seq` at that moment. Both halves are load-bearing:
//!
//! * the edge deduplicates on `event_id`, so a marker that grew after
//!   publication would be discarded as a duplicate of the smaller first
//!   version, silently under-reporting the loss;
//! * acknowledgement is cumulative, so a sequence allocated at the first loss
//!   would sit *below* events buffered afterwards, where an acknowledgement
//!   covering those would bury a marker the edge had never seen.
//!
//! # This buffer's overflow policy is not the ledger's
//!
//! Evicting the oldest audit event and recording a `history.gap` is correct
//! *here*: the gap tells the edge it is missing a **record**, which the edge can
//! see and reason about. [`crate::ledger`] looks structurally similar and is
//! not — evicting an unacknowledged `command.result` removes a **quantity the
//! edge's rolling budget is derived from**, and the edge learns nothing at all.
//! The two share no overflow decision, deliberately.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use rhizo_mqtt_contract::{
    BootId, EventId, UtcMillis,
    payload::{BufferedEvent, DeviceEventBatch, EventDetail, EventKind, EventTier},
};

/// Audit-tier capacity. Never reduced to make room for telemetry.
pub const AUDIT_CAPACITY: usize = 64;
/// Telemetry-tier capacity.
pub const TELEMETRY_CAPACITY: usize = 256;
/// How many events go in one replay batch.
pub const REPLAY_BATCH: usize = 32;

/// A loss run, accumulating until it is sealed into a replayable event.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GapMetadata {
    /// The stable id the replayed marker will carry.
    pub event_id: EventId,
    /// Monotonic instant the first loss occurred.
    pub monotonic_ms: u64,
    /// Wall time, if the clock was synchronised.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_time_ms: Option<UtcMillis>,
    /// First lost sequence.
    pub from_seq: u64,
    /// Last lost sequence.
    pub to_seq: u64,
    /// How many events were lost.
    pub lost_count: u32,
    /// Which tier lost them.
    pub lost_tier: EventTier,
}

/// The persisted buffer.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BufferState {
    /// Buffered events.
    #[serde(default)]
    pub events: Vec<BufferedEvent>,
    /// The next `device_seq` to hand out.
    #[serde(default)]
    pub next_seq: u64,
    /// Everything at or below this sequence has been acknowledged by the edge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acked_through_seq: Option<u64>,
    /// Loss recorded since the last replay.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gap: Option<GapMetadata>,
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

/// What an `event.ack` did.
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
    /// Named a different boot; ignored entirely.
    WrongBoot,
}

impl BufferState {
    /// How many events of a tier are held.
    #[must_use]
    pub fn count(&self, tier: EventTier) -> usize {
        self.events.iter().filter(|e| e.tier == tier).count()
    }

    /// How many events are held, excluding any pending gap marker.
    #[must_use]
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// Whether nothing is buffered and no gap is pending.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.events.is_empty() && self.gap.is_none()
    }

    /// The capacity of a tier.
    ///
    /// An unrecognised tier takes the cheaper reservation: a future tier this
    /// build does not understand must not be able to claim the audit budget.
    #[must_use]
    pub const fn capacity(tier: EventTier) -> usize {
        match tier {
            EventTier::Audit => AUDIT_CAPACITY,
            EventTier::Telemetry | EventTier::Unknown => TELEMETRY_CAPACITY,
        }
    }

    /// Buffers an event, evicting within its own tier if that tier is full.
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

    fn evict_oldest(&mut self, tier: EventTier) -> Option<u64> {
        let index = self.events.iter().position(|e| e.tier == tier)?;
        Some(self.events.remove(index).device_seq)
    }

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
                // The marker reports the worse of the losses it spans: losing
                // the record of a dose matters more than losing a sample.
                if lost_tier == EventTier::Audit {
                    gap.lost_tier = EventTier::Audit;
                }
            }
            None => {
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

    /// Seals the pending gap into a replayable event.
    ///
    /// Called immediately before a replay is built. The push deliberately
    /// bypasses the tier capacity check: a marker is one small event per
    /// replay, and evicting an audit event to make room for the record of an
    /// eviction would be a loop with nothing to show for it.
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

    /// Whether a gap is still accumulating and has never been sent.
    #[must_use]
    pub const fn has_pending_gap(&self) -> bool {
        self.gap.is_some()
    }

    /// Everything sealed and waiting to be replayed, oldest first.
    ///
    /// A gap still accumulating is **not** included: it has no final range yet.
    #[must_use]
    pub fn replay_events(&self) -> Vec<BufferedEvent> {
        let mut events = self.events.clone();
        events.sort_by_key(|e| e.device_seq);
        events
    }

    /// The highest sequence this device has ever allocated.
    #[must_use]
    pub const fn highest_allocated_seq(&self) -> Option<u64> {
        self.next_seq.checked_sub(1)
    }

    /// Splits the buffered history into replay batches.
    ///
    /// Always returns at least one batch and the last always sets
    /// `complete: true`. An empty buffer still produces one complete batch,
    /// because the edge holds every plant on a reconnecting device until it
    /// sees that flag — a device with nothing to say still has to say so, or
    /// the plant waits for ever (protocol §5.4, SAFETY-016).
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

    /// Applies a cumulative `event.ack` for this boot.
    ///
    /// Three refusals, all fail-closed:
    ///
    /// * a **different boot** is ignored entirely — a delayed acknowledgement
    ///   from a previous run says nothing about the history this run holds;
    /// * a sequence **beyond** anything this device allocated is refused
    ///   **whole, not clamped**, because clamping would turn one corrupt field
    ///   into "delete the entire buffer";
    /// * a sequence **not newer** than one already applied is a no-op.
    pub fn acknowledge(
        &mut self,
        ack_boot: BootId,
        own_boot: BootId,
        through_seq: u64,
    ) -> AckOutcome {
        if ack_boot != own_boot {
            return AckOutcome::WrongBoot;
        }
        match self.highest_allocated_seq() {
            Some(highest) if through_seq > highest => {
                return AckOutcome::BeyondKnown {
                    through_seq,
                    highest,
                };
            }
            None => {
                return AckOutcome::BeyondKnown {
                    through_seq,
                    highest: 0,
                };
            }
            Some(_) => {}
        }
        if self
            .acked_through_seq
            .is_some_and(|previous| through_seq <= previous)
        {
            return AckOutcome::NotNewer { through_seq };
        }
        let before = self.events.len();
        self.events.retain(|e| e.device_seq > through_seq);
        self.acked_through_seq = Some(through_seq);
        AckOutcome::Applied {
            through_seq,
            removed: before - self.events.len(),
        }
    }
}

/// A deterministic identifier for a gap marker.
///
/// Derived from the loss it describes rather than drawn at random, so a marker
/// rebuilt from the same facts regenerates the same id and a test can assert on
/// it. Uniqueness comes from the sequence, which is monotonic within a boot.
fn gap_id(lost_seq: u64, monotonic_ms: u64) -> Uuid {
    let mut bytes = [0u8; 16];
    bytes[0..8].copy_from_slice(&lost_seq.to_be_bytes());
    bytes[8..16].copy_from_slice(&monotonic_ms.to_be_bytes());
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn boot(n: u128) -> BootId {
        BootId::from_uuid(Uuid::from_u128(n))
    }

    fn event_id(n: u128) -> EventId {
        EventId::from_uuid(Uuid::from_u128(n))
    }

    fn push_telemetry(buffer: &mut BufferState, n: u128) -> Buffered {
        buffer.push(
            event_id(n),
            EventTier::Telemetry,
            EventKind::Unknown("telemetry.sample".into()),
            n as u64,
            None,
            EventDetail::Unknown,
        )
    }

    fn push_audit(buffer: &mut BufferState, n: u128) -> Buffered {
        buffer.push(
            event_id(n),
            EventTier::Audit,
            EventKind::OfflineRefused,
            n as u64,
            None,
            EventDetail::Refused {
                reason: "leak_unknown".into(),
            },
        )
    }

    #[test]
    fn safety_020_audit_events_survive_a_telemetry_flood() {
        let mut buffer = BufferState::default();
        for n in 0..AUDIT_CAPACITY {
            push_audit(&mut buffer, n as u128 + 1);
        }
        for n in 0..(TELEMETRY_CAPACITY * 2) {
            push_telemetry(&mut buffer, 100_000 + n as u128);
        }
        assert_eq!(buffer.count(EventTier::Audit), AUDIT_CAPACITY);
        assert_eq!(buffer.count(EventTier::Telemetry), TELEMETRY_CAPACITY);
    }

    #[test]
    fn safety_016_event_ids_are_stable_across_repeated_replays() {
        let mut buffer = BufferState::default();
        for n in 1..=5u128 {
            push_audit(&mut buffer, n);
        }
        let first: Vec<_> = buffer.replay_events().iter().map(|e| e.event_id).collect();
        let second: Vec<_> = buffer.replay_events().iter().map(|e| e.event_id).collect();
        assert_eq!(first, second);
        assert_eq!(first, (1..=5u128).map(event_id).collect::<Vec<_>>());
    }

    #[test]
    fn eviction_records_a_gap_with_the_correct_range_and_count() {
        let mut buffer = BufferState::default();
        for n in 0..=TELEMETRY_CAPACITY {
            push_telemetry(&mut buffer, n as u128 + 1);
        }
        let gap = buffer.gap.as_ref().expect("a gap was recorded");
        assert_eq!(gap.lost_count, 1);
        assert_eq!(gap.from_seq, 0);
        assert_eq!(gap.to_seq, 0);
        assert_eq!(gap.lost_tier, EventTier::Telemetry);
    }

    /// A marker that grew after publication would be discarded by the edge as a
    /// duplicate of the smaller first version, so a seal is final and later
    /// losses open a new marker.
    #[test]
    fn a_sealed_gap_is_immutable_and_a_later_loss_opens_a_new_one() {
        let mut buffer = BufferState::default();
        for n in 0..=TELEMETRY_CAPACITY {
            push_telemetry(&mut buffer, n as u128 + 1);
        }
        buffer.seal_gap();
        assert!(!buffer.has_pending_gap());
        let sealed = buffer
            .replay_events()
            .into_iter()
            .find(|e| e.kind == EventKind::HistoryGap)
            .expect("the marker was sealed");
        push_telemetry(&mut buffer, 999_999);
        assert!(buffer.has_pending_gap(), "a later loss opens a new marker");
        let still = buffer
            .replay_events()
            .into_iter()
            .find(|e| e.event_id == sealed.event_id)
            .expect("the sealed marker is unchanged");
        assert_eq!(still.detail, sealed.detail);
    }

    /// The reason the sequence is allocated at seal time: a marker must sit
    /// above everything a cumulative acknowledgement could already have covered.
    #[test]
    fn a_sealed_marker_outranks_every_event_buffered_before_it() {
        let mut buffer = BufferState::default();
        for n in 0..=TELEMETRY_CAPACITY {
            push_telemetry(&mut buffer, n as u128 + 1);
        }
        let highest_before = buffer
            .replay_events()
            .last()
            .map(|e| e.device_seq)
            .expect("events buffered");
        buffer.seal_gap();
        let marker = buffer
            .replay_events()
            .into_iter()
            .find(|e| e.kind == EventKind::HistoryGap)
            .expect("sealed");
        assert!(marker.device_seq > highest_before);
    }

    #[test]
    fn an_unsealed_gap_is_never_replayed() {
        let mut buffer = BufferState::default();
        for n in 0..=TELEMETRY_CAPACITY {
            push_telemetry(&mut buffer, n as u128 + 1);
        }
        assert!(buffer.has_pending_gap());
        assert!(
            buffer
                .replay_events()
                .iter()
                .all(|e| e.kind != EventKind::HistoryGap)
        );
    }

    #[test]
    fn an_empty_buffer_still_replays_one_complete_batch() {
        let buffer = BufferState::default();
        let batches = buffer.replay_batches(REPLAY_BATCH);
        assert_eq!(batches.len(), 1);
        assert!(batches[0].complete);
        assert!(batches[0].events.is_empty());
    }

    #[test]
    fn only_the_last_replay_batch_is_complete() {
        let mut buffer = BufferState::default();
        // Twenty-five audit events, batched ten at a time: three batches, and
        // comfortably inside `AUDIT_CAPACITY` so nothing is evicted and the
        // test is about batching rather than about eviction.
        for n in 1..=25u128 {
            push_audit(&mut buffer, n);
        }
        let batches = buffer.replay_batches(10);
        assert_eq!(batches.len(), 3);
        assert_eq!(
            batches.iter().map(|b| b.complete).collect::<Vec<_>>(),
            [false, false, true]
        );
    }

    /// Refused **whole, not clamped**: clamping would turn one corrupt field
    /// into "delete the entire buffer".
    #[test]
    fn an_acknowledgement_beyond_the_highest_allocated_sequence_is_refused_whole() {
        let mut buffer = BufferState::default();
        for n in 1..=3u128 {
            push_audit(&mut buffer, n);
        }
        let outcome = buffer.acknowledge(boot(1), boot(1), 99);
        assert_eq!(
            outcome,
            AckOutcome::BeyondKnown {
                through_seq: 99,
                highest: 2
            }
        );
        assert_eq!(buffer.len(), 3, "nothing was deleted");
    }

    #[test]
    fn an_acknowledgement_for_another_boot_is_ignored() {
        let mut buffer = BufferState::default();
        push_audit(&mut buffer, 1);
        assert_eq!(
            buffer.acknowledge(boot(2), boot(1), 0),
            AckOutcome::WrongBoot
        );
        assert_eq!(buffer.len(), 1);
    }

    #[test]
    fn acknowledgement_is_idempotent_and_never_regresses() {
        let mut buffer = BufferState::default();
        for n in 1..=4u128 {
            push_audit(&mut buffer, n);
        }
        assert_eq!(
            buffer.acknowledge(boot(1), boot(1), 1),
            AckOutcome::Applied {
                through_seq: 1,
                removed: 2
            }
        );
        assert_eq!(
            buffer.acknowledge(boot(1), boot(1), 1),
            AckOutcome::NotNewer { through_seq: 1 }
        );
        assert_eq!(
            buffer.acknowledge(boot(1), boot(1), 0),
            AckOutcome::NotNewer { through_seq: 0 }
        );
        assert_eq!(buffer.len(), 2);
    }

    #[test]
    fn the_buffer_survives_a_round_trip_through_storage() {
        let mut buffer = BufferState::default();
        for n in 1..=3u128 {
            push_audit(&mut buffer, n);
        }
        for n in 0..=TELEMETRY_CAPACITY {
            push_telemetry(&mut buffer, 5_000 + n as u128);
        }
        let encoded = serde_json::to_vec(&buffer).expect("encodes");
        let restored: BufferState = serde_json::from_slice(&encoded).expect("decodes");
        assert_eq!(restored, buffer);
        assert!(restored.has_pending_gap());
    }
}
