//! Host-clock adapters owned by the executable edge boundary.

use chrono::{DateTime, TimeDelta, Utc};
use rhizo_domain::Clock;
use std::time::Instant;

/// A wall clock whose elapsed duration is multiplied by a fixed scale.
///
/// The anchor is read once, so changing the host clock after startup cannot
/// create a scale-dependent discontinuity. A scale of 1.0 behaves as ordinary
/// elapsed wall time; M8 supplies the same larger value to the simulator.
pub struct AcceleratedClock {
    base_utc: DateTime<Utc>,
    base_monotonic: Instant,
    scale: f64,
}

impl AcceleratedClock {
    /// Creates a clock anchored at process startup.
    #[must_use]
    pub fn new(base_utc: DateTime<Utc>, scale: f64) -> Self {
        Self {
            base_utc,
            base_monotonic: Instant::now(),
            scale,
        }
    }
}

impl Clock for AcceleratedClock {
    fn now(&self) -> DateTime<Utc> {
        let elapsed_ms = self.base_monotonic.elapsed().as_secs_f64() * 1_000.0 * self.scale;
        let elapsed_ms = elapsed_ms.min(i64::MAX as f64) as i64;
        self.base_utc + TimeDelta::milliseconds(elapsed_ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale_is_applied_to_monotonic_elapsed_time() {
        let base = DateTime::from_timestamp_millis(1_000).unwrap();
        let clock = AcceleratedClock::new(base, 600.0);
        std::thread::sleep(std::time::Duration::from_millis(10));
        assert!(clock.now().timestamp_millis() >= 7_000);
    }
}
