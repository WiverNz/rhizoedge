//! The wall clock and the monotonic timer (M9-009, ADR-013).
//!
//! Two types rather than one, because they answer different questions and
//! confusing them is how SAFETY-015 gets broken. The wall clock comes from the
//! Edge over MQTT and may be absent; the monotonic timer is
//! `esp_timer_get_time` and is always available, cannot jump, and is what every
//! offline duration is measured against.
//!
//! The `edge.time` rules themselves are **not** reimplemented here. The
//! strictly-increasing acceptance rule and the age-based validity live in
//! `rhizo_mqtt_contract::payload::TimeSyncState`, which the simulator uses too.
//! A second copy on the device is exactly how a device comes to claim
//! synchronisation it does not have — and there is no way to detect that from
//! the edge.

use rhizo_mqtt_contract::payload::TimeSyncState;
use rhizo_mqtt_contract::UtcMillis;
use rhizo_node_app::ports::{Clock, Monotonic};

/// Milliseconds since boot, from the ESP-IDF high-resolution timer.
///
/// 64-bit, so it wraps after roughly 292 000 years. The saturating arithmetic
/// downstream is not for this counter; it is for a corrupted RTC word, which
/// can present any value at all.
#[must_use]
pub fn monotonic_ms() -> u64 {
    // SAFETY: `esp_timer_get_time` reads a monotonic counter and has no
    // preconditions. It is `unsafe` only because it is `extern "C"`.
    let micros = unsafe { esp_idf_sys::esp_timer_get_time() };
    (micros.max(0) as u64) / 1000
}

/// The monotonic timer.
///
/// The run loop calls [`monotonic_ms`] directly; this is the `Monotonic` impl
/// the application layer takes when it is handed a timer rather than a reading,
/// which is how the offline evaluator is driven on hardware.
#[allow(
    dead_code,
    reason = "the Monotonic impl the offline path takes; the loop uses monotonic_ms directly"
)]
#[derive(Clone, Copy, Debug, Default)]
pub struct EspMonotonic;

impl Monotonic for EspMonotonic {
    fn monotonic_ms(&self) -> u64 {
        monotonic_ms()
    }
}

/// The Edge-synchronised wall clock.
#[derive(Clone, Copy, Debug, Default)]
pub struct EdgeClock {
    sync: TimeSyncState,
    applied_at_monotonic_ms: u64,
}

impl EdgeClock {
    /// A clock that has never been synchronised.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Applies an `edge.time`, returning whether it was accepted.
    ///
    /// An ignored message updates **nothing** — not the clock, not the last
    /// applied value, and above all not the monotonic instant the validity
    /// window is measured from.
    pub fn apply(&mut self, edge_time_ms: UtcMillis, now_monotonic_ms: u64) -> bool {
        if self.sync.apply(edge_time_ms, now_monotonic_ms) {
            self.applied_at_monotonic_ms = now_monotonic_ms;
            true
        } else {
            false
        }
    }

    /// Whether synchronisation is current, by age.
    #[must_use]
    pub fn is_synced(&self, now_monotonic_ms: u64) -> bool {
        self.sync.is_synced(now_monotonic_ms)
    }

    /// The wall time now, or `None` when unsynchronised or aged out.
    #[must_use]
    pub fn now_ms_at(&self, now_monotonic_ms: u64) -> Option<i64> {
        if !self.sync.is_synced(now_monotonic_ms) {
            return None;
        }
        let applied = self.sync.last_applied()?;
        let elapsed = now_monotonic_ms.saturating_sub(self.applied_at_monotonic_ms);
        Some(applied.0 + elapsed as i64)
    }
}

/// A [`Clock`] view of an [`EdgeClock`] at one instant.
///
/// The instant is captured rather than read inside `now_ms`, so every consumer
/// in one pass sees the same time. A gate that read the clock twice could
/// evaluate a TTL against two different instants.
#[derive(Clone, Copy, Debug)]
pub struct ClockAt {
    now_ms: Option<i64>,
}

impl ClockAt {
    /// Captures the wall time at a monotonic instant.
    #[must_use]
    pub fn capture(clock: &EdgeClock, now_monotonic_ms: u64) -> Self {
        Self {
            now_ms: clock.now_ms_at(now_monotonic_ms),
        }
    }
}

impl Clock for ClockAt {
    fn now_ms(&self) -> Option<i64> {
        self.now_ms
    }
}
