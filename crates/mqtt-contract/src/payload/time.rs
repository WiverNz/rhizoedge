//! Edge-over-MQTT time synchronisation.
use crate::UtcMillis;
use serde::{Deserialize, Serialize};
/// Edge push interval while online.
pub const TIME_SYNC_INTERVAL_SECONDS: u64 = 300;
/// Maximum monotonic age of an applied synchronization.
pub const TIME_SYNC_MAX_AGE_SECONDS: u64 = 1800;
/// The sole field of `edge.time`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EdgeTime {
    /** Edge wall time sampled at publish. */
    pub edge_time_ms: UtcMillis,
}
/// Pure device synchronization freshness state.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TimeSyncState {
    last_applied_edge_time_ms: Option<UtcMillis>,
    synced_at_monotonic_ms: Option<u64>,
}
impl TimeSyncState {
    /** Applies only a strictly newer timestamp and refreshes freshness only then. */
    pub fn apply(&mut self, time: UtcMillis, monotonic_now_ms: u64) -> bool {
        if self
            .last_applied_edge_time_ms
            .is_some_and(|last| time <= last)
        {
            return false;
        }
        self.last_applied_edge_time_ms = Some(time);
        self.synced_at_monotonic_ms = Some(monotonic_now_ms);
        true
    }
    /** True strictly before the maximum age boundary. */
    pub fn is_synced(&self, now_ms: u64) -> bool {
        self.synced_at_monotonic_ms
            .is_some_and(|at| now_ms.saturating_sub(at) < TIME_SYNC_MAX_AGE_SECONDS * 1000)
    }
    /** Most recent accepted timestamp. */
    pub const fn last_applied(&self) -> Option<UtcMillis> {
        self.last_applied_edge_time_ms
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn safety_002_duplicate_time_sync_does_not_extend_validity() {
        let mut s = TimeSyncState::default();
        assert!(s.apply(UtcMillis(100), 0));
        for now in [1, 1000, 100_000, 1_700_000] {
            assert!(!s.apply(UtcMillis(100), now));
        }
        assert!(!s.is_synced(1_800_000));
    }
    #[test]
    fn safety_002_stale_time_sync_never_applied() {
        let mut s = TimeSyncState::default();
        assert!(s.apply(UtcMillis(100), 5));
        assert!(!s.apply(UtcMillis(99), 1000));
        assert!(!s.apply(UtcMillis(100), 1000));
        assert!(s.apply(UtcMillis(101), 1000));
        assert!(s.is_synced(1000));
        assert_eq!(s.last_applied(), Some(UtcMillis(101)));
    }
}
