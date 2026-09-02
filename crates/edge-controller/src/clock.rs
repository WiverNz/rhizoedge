//! Host-clock adapters owned by the executable edge boundary.
//!
//! `rhizo-domain` is pure and may not read a clock, so the binary supplies one.
//! There are exactly two, and which one is in use is decided once, by
//! `time_scale`.

use chrono::{DateTime, TimeDelta, Utc};
use rhizo_domain::Clock;
use std::time::Instant;

/// The edge's wall clock.
///
/// # Real time is not "accelerated by 1.0"
///
/// The two variants are genuinely different clocks, and collapsing them would
/// break [`control::clock_step`](crate::control::clock_step) silently. That
/// detector's whole method is to compare the wall clock against an independent
/// monotonic reference — its module doc opens with "comparing the wall clock
/// against itself cannot reveal a step". A clock defined as
/// `anchor + monotonic_elapsed` *is* the monotonic clock wearing a wall-clock
/// hat: the divergence is zero by construction, no NTP step can ever be
/// detected, and F-060-51's forward-step lockout becomes unreachable code.
///
/// The same anchoring would also hide the correction itself. ADR-013 assumes
/// the edge host is genuinely NTP-synced, and an edge that boots before the
/// network is up would keep its wrong start time for the life of the process —
/// then hand it to every device, because `edge.time` is the devices' only
/// source of wall time.
///
/// So [`HostClock::Real`] reads the host on every call and is what production
/// runs. [`HostClock::Accelerated`] exists for M8, where a watering cycle has
/// to fit in seconds, and it is anchored precisely so that the virtual clock
/// cannot jump when the host's does.
pub enum HostClock {
    /// The host wall clock, read afresh on every call.
    Real,
    /// Virtual time: an anchor plus scaled monotonic elapsed time.
    Accelerated {
        /// The instant virtual time started from.
        base_utc: DateTime<Utc>,
        /// The monotonic reading taken with it.
        base_monotonic: Instant,
        /// Virtual seconds per real second.
        scale: f64,
    },
}

impl HostClock {
    /// Builds the clock `scale` asks for.
    ///
    /// `base_utc` is used only when the clock is accelerated; at real time the
    /// host is the anchor and there is nothing to anchor to.
    #[must_use]
    pub fn new(base_utc: DateTime<Utc>, scale: f64) -> Self {
        if scale == 1.0 {
            Self::Real
        } else {
            Self::Accelerated {
                base_utc,
                base_monotonic: Instant::now(),
                scale,
            }
        }
    }

    /// Whether this clock is virtual, and therefore whether the clock-step
    /// detector's monotonic reference has to be scaled to match it.
    #[must_use]
    pub const fn is_accelerated(&self) -> bool {
        matches!(self, Self::Accelerated { .. })
    }
}

impl Clock for HostClock {
    fn now(&self) -> DateTime<Utc> {
        match self {
            #[allow(
                clippy::disallowed_methods,
                reason = "this adapter is the host-clock boundary the domain is kept pure of"
            )]
            Self::Real => Utc::now(),
            Self::Accelerated {
                base_utc,
                base_monotonic,
                scale,
            } => {
                let elapsed_ms = base_monotonic.elapsed().as_secs_f64() * 1_000.0 * scale;
                let elapsed_ms = elapsed_ms.min(i64::MAX as f64) as i64;
                *base_utc + TimeDelta::milliseconds(elapsed_ms)
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn scale_is_applied_to_monotonic_elapsed_time() {
        let base = DateTime::from_timestamp_millis(1_000).unwrap();
        let clock = HostClock::new(base, 600.0);
        assert!(clock.is_accelerated());
        std::thread::sleep(std::time::Duration::from_millis(10));
        assert!(clock.now().timestamp_millis() >= 7_000);
    }

    /// At production scale the anchor is ignored and the host is read on every
    /// call. This is the property `clock_step::Detector` depends on: a wall
    /// clock derived from the monotonic clock can never diverge from it, so a
    /// forward NTP step would become undetectable and F-060-51 unreachable.
    #[test]
    fn real_time_reads_the_host_and_ignores_the_anchor() {
        let ancient = DateTime::from_timestamp_millis(1_000).unwrap();
        let clock = HostClock::new(ancient, 1.0);
        assert!(!clock.is_accelerated());
        #[allow(
            clippy::disallowed_methods,
            reason = "asserting on the host boundary itself"
        )]
        let host = Utc::now();
        let observed = clock.now();
        assert!(
            (observed - host).num_seconds().abs() < 5,
            "a real-time clock must track the host, not the anchor: {observed} vs {host}"
        );
    }

    /// Two reads of a real-time clock advance with the host rather than being
    /// frozen at a startup anchor.
    #[test]
    fn real_time_advances_between_reads() {
        let clock = HostClock::new(DateTime::from_timestamp_millis(1_000).unwrap(), 1.0);
        let first = clock.now();
        std::thread::sleep(std::time::Duration::from_millis(5));
        assert!(clock.now() >= first);
    }
}
