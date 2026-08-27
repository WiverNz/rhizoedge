//! Wall-clock synchronisation from `edge.time`.
//!
//! The device has no NTP client and no fallback to its host's clock: its wall
//! time comes from the Edge over the MQTT connection it already has
//! ([ADR-013](../../../../docs/adr/013-clock-and-time-semantics.md), protocol
//! §5.12).
//!
//! # The acceptance rule is not implemented here
//!
//! [`rhizo_mqtt_contract::payload::TimeSyncState`] decides whether a timestamp
//! is applied, because that decision is shared with the firmware and is
//! precisely where SAFETY-002 can be lost. This module only carries the value
//! and the resulting freshness question.
//!
//! The rule it enforces is **strictly increasing**, not merely non-decreasing.
//! QoS 1 permits redelivery, so the same `edge_time_ms` can arrive any number of
//! times; if an equal value refreshed the freshness anchor, one captured message
//! replayed forever would hold `clock_synced` true while the device learned
//! nothing new about the Edge's clock.

use rhizo_mqtt_contract::UtcMillis;
use rhizo_mqtt_contract::payload::{EdgeTime, TIME_SYNC_MAX_AGE_SECONDS, TimeSyncState};

use crate::clock::WallClock;

/// How often an unsynchronised device may republish its status as a request for
/// synchronisation (protocol §5.12 "Triggering", rule 3).
pub const UNSYNCED_STATUS_INTERVAL_MS: u64 = 60_000;

/// The device's synchronisation state.
#[derive(Clone, Copy, Debug, Default)]
pub struct TimeSync {
    acceptance: TimeSyncState,
    wall: WallClock,
}

impl TimeSync {
    /// Applies an `edge.time`, returning whether it changed anything.
    ///
    /// A stale or duplicate timestamp is ignored **entirely**: the wall clock
    /// does not move and, critically, the freshness anchor is not refreshed.
    pub fn apply(&mut self, time: EdgeTime, monotonic_now_ms: u64) -> bool {
        if !self.acceptance.apply(time.edge_time_ms, monotonic_now_ms) {
            return false;
        }
        self.wall.set(time.edge_time_ms, monotonic_now_ms);
        true
    }

    /// Whether the synchronisation is fresh enough to accept commands.
    ///
    /// Measured as monotonic **age**, not as "a sync once succeeded".
    #[must_use]
    pub fn is_synced(&self, monotonic_now_ms: u64) -> bool {
        self.acceptance.is_synced(monotonic_now_ms)
    }

    /// The last accepted Edge timestamp, whatever its age.
    #[must_use]
    pub fn last_applied(&self) -> Option<UtcMillis> {
        self.acceptance.last_applied()
    }

    /// The device's wall time, but **only while synchronised**.
    ///
    /// Returning `None` once the synchronisation ages out is deliberate: it is
    /// what makes the envelope carry a UUIDv4 and omit `device_time_ms`, which
    /// is how protocol §4 says an unsynchronised device must present itself. A
    /// device that kept publishing an extrapolated timestamp would be asserting
    /// a wall time it has no current evidence for.
    #[must_use]
    pub fn synced_now_ms(&self, monotonic_now_ms: u64) -> Option<UtcMillis> {
        self.is_synced(monotonic_now_ms)
            .then(|| self.wall.now_ms(monotonic_now_ms))
            .flatten()
    }

    /// The maximum age, in milliseconds, before synchronisation lapses.
    #[must_use]
    pub const fn max_age_ms() -> u64 {
        TIME_SYNC_MAX_AGE_SECONDS * 1000
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edge(ms: i64) -> EdgeTime {
        EdgeTime {
            edge_time_ms: UtcMillis(ms),
        }
    }

    #[test]
    fn the_first_valid_timestamp_synchronises_the_device() {
        let mut sync = TimeSync::default();
        assert!(!sync.is_synced(0), "a fresh device is never synchronised");
        assert!(sync.apply(edge(1_756_121_400_000), 5_000));
        assert!(sync.is_synced(5_000));
        assert_eq!(
            sync.synced_now_ms(5_000),
            Some(UtcMillis(1_756_121_400_000))
        );
    }

    /// SAFETY-002. The failure this prevents: one captured `edge.time` replayed
    /// indefinitely holding `clock_synced` true forever.
    #[test]
    fn a_duplicate_timestamp_is_ignored_and_never_extends_the_window() {
        let mut sync = TimeSync::default();
        assert!(sync.apply(edge(1_000), 0));
        for now in [1, 60_000, 600_000, 1_700_000] {
            assert!(
                !sync.apply(edge(1_000), now),
                "a duplicate must be ignored at {now}"
            );
        }
        assert!(
            !sync.is_synced(TimeSync::max_age_ms()),
            "the window measures synchronisation freshness, not message arrival"
        );
    }

    #[test]
    fn an_older_timestamp_never_moves_the_clock_backwards() {
        let mut sync = TimeSync::default();
        assert!(sync.apply(edge(1_000), 0));
        assert!(!sync.apply(edge(999), 10));
        assert_eq!(sync.last_applied(), Some(UtcMillis(1_000)));
        // ...and the freshness anchor stayed at 0, not 10.
        assert!(!sync.is_synced(TimeSync::max_age_ms()));
    }

    #[test]
    fn a_strictly_newer_timestamp_is_applied_and_refreshes_the_window() {
        let mut sync = TimeSync::default();
        assert!(sync.apply(edge(1_000), 0));
        assert!(sync.apply(edge(1_001), 1_000_000));
        assert!(sync.is_synced(1_000_000));
        assert!(sync.is_synced(1_000_000 + TimeSync::max_age_ms() - 1));
    }

    #[test]
    fn synchronisation_ages_out_on_the_monotonic_clock() {
        let mut sync = TimeSync::default();
        sync.apply(edge(1_756_121_400_000), 0);
        assert!(sync.is_synced(TimeSync::max_age_ms() - 1));
        assert!(!sync.is_synced(TimeSync::max_age_ms()));
        assert!(
            sync.synced_now_ms(TimeSync::max_age_ms()).is_none(),
            "an aged-out device asserts no wall time"
        );
        assert_eq!(
            sync.last_applied(),
            Some(UtcMillis(1_756_121_400_000)),
            "the value is remembered even when it is no longer fresh"
        );
    }

    #[test]
    fn resynchronising_after_an_age_out_restores_the_clock() {
        let mut sync = TimeSync::default();
        sync.apply(edge(1_756_121_400_000), 0);
        let aged = TimeSync::max_age_ms() + 1_000;
        assert!(!sync.is_synced(aged));
        assert!(sync.apply(edge(1_756_121_400_001), aged));
        assert!(sync.is_synced(aged));
    }

    #[test]
    fn the_documented_window_is_thirty_minutes() {
        assert_eq!(TimeSync::max_age_ms(), 1_800_000);
    }
}
