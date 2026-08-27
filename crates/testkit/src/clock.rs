//! A deterministic clock for tests.
//!
//! [ADR-013](../../../../docs/adr/013-clock-and-time-semantics.md) requires
//! that domain logic never read the system clock, and that no test sleeps to
//! advance logical time. `TestClock` is the mechanism.

use std::sync::{Arc, Mutex, MutexGuard};

use chrono::{DateTime, Duration, Utc};
use rhizo_domain::Clock;

/// A clock whose time only moves when a test moves it.
///
/// Cheap to clone and share: a test holds one and hands clones to several
/// components, all of which observe the same instant.
///
/// ```
/// use chrono::{Duration, TimeZone, Utc};
/// use rhizo_testkit::TestClock;
///
/// let clock = TestClock::new(Utc.with_ymd_and_hms(2026, 8, 26, 12, 0, 0).unwrap());
/// let shared = clock.clone();
///
/// clock.advance(Duration::hours(25));
///
/// // The clone sees it too — this is what makes a shared fixture possible.
/// assert_eq!(shared.now(), clock.now());
/// ```
///
/// # Why tests must not sleep
///
/// Three safety invariants are time-derived: SAFETY-002 (an expired command
/// must not execute), SAFETY-005 (a stale sample must not drive automatic
/// watering), and SAFETY-006 (the rolling 24-hour cap). Testing SAFETY-006 by
/// waiting would take a day per case; advancing this clock takes microseconds,
/// which is the difference between a property test that exists and one that
/// does not.
///
/// A test that sleeps is also a test that is flaky on a loaded CI machine, and
/// a flaky safety test is quickly a disabled safety test.
///
/// # Relationship to the `Clock` trait
///
/// `rhizo-domain` does not exist yet, so the trait it will define
/// (`fn now(&self) -> DateTime<Utc>`) is not implementable here. M1-012 adds
/// `impl Clock for TestClock`. This type deliberately does **not** invent a
/// second trait in the meantime — two clock abstractions would be worse than
/// none.
#[derive(Clone, Debug)]
pub struct TestClock {
    now: Arc<Mutex<DateTime<Utc>>>,
}

impl TestClock {
    /// Creates a clock reading exactly `at`.
    #[must_use]
    pub fn new(at: DateTime<Utc>) -> Self {
        Self {
            now: Arc::new(Mutex::new(at)),
        }
    }

    /// The current instant.
    #[must_use]
    pub fn now(&self) -> DateTime<Utc> {
        *self.lock()
    }

    /// Sets the clock to `at`.
    ///
    /// Accepts instants in the past as readily as the future: a clock that
    /// steps *backwards* is a real device condition (an NTP correction after a
    /// boot with an unsynchronised RTC), and it is one of the conditions the
    /// system has to behave sanely under, so the fixture has to be able to
    /// produce it.
    pub fn set(&self, at: DateTime<Utc>) {
        *self.lock() = at;
    }

    /// Moves the clock by `by`.
    ///
    /// A negative `chrono::Duration` moves it backwards, for the same reason
    /// [`set`](Self::set) accepts a past instant.
    pub fn advance(&self, by: Duration) {
        let mut guard = self.lock();
        *guard += by;
    }

    /// Takes the lock, recovering from poisoning.
    ///
    /// The guarded value is a single `DateTime`, so a panic elsewhere cannot
    /// leave it torn. Propagating the poison would turn one failing test into
    /// a cascade of unrelated failures whose real cause is three tests away.
    fn lock(&self) -> MutexGuard<'_, DateTime<Utc>> {
        self.now
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl Clock for TestClock {
    fn now(&self) -> DateTime<Utc> {
        TestClock::now(self)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread;

    use chrono::TimeZone;

    use super::*;

    fn at(y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, mo, d, h, mi, s).unwrap()
    }

    #[test]
    fn a_new_clock_reads_exactly_what_it_was_given() {
        let t = at(2026, 8, 26, 12, 0, 0);
        assert_eq!(TestClock::new(t).now(), t);
    }

    #[test]
    fn advance_moves_time_forward_by_exactly_the_requested_amount() {
        let start = at(2026, 8, 26, 12, 0, 0);
        let clock = TestClock::new(start);

        clock.advance(Duration::seconds(30));
        assert_eq!(clock.now(), start + Duration::seconds(30));

        clock.advance(Duration::hours(24));
        assert_eq!(
            clock.now(),
            start + Duration::seconds(30) + Duration::hours(24)
        );
    }

    #[test]
    fn advance_accumulates_rather_than_replacing() {
        let start = at(2026, 1, 1, 0, 0, 0);
        let clock = TestClock::new(start);
        for _ in 0..1_000 {
            clock.advance(Duration::milliseconds(1));
        }
        assert_eq!(clock.now(), start + Duration::seconds(1));
    }

    #[test]
    fn set_replaces_the_instant_outright() {
        let clock = TestClock::new(at(2026, 8, 26, 12, 0, 0));
        let target = at(2027, 3, 1, 6, 30, 0);
        clock.set(target);
        assert_eq!(clock.now(), target);
    }

    #[test]
    fn set_works_backwards_for_clock_step_tests() {
        let start = at(2026, 8, 26, 12, 0, 0);
        let clock = TestClock::new(start);
        let earlier = start - Duration::hours(6);

        clock.set(earlier);

        assert_eq!(clock.now(), earlier);
        assert!(clock.now() < start);
    }

    #[test]
    fn advance_accepts_a_negative_duration() {
        let start = at(2026, 8, 26, 12, 0, 0);
        let clock = TestClock::new(start);
        clock.advance(Duration::minutes(-90));
        assert_eq!(clock.now(), start - Duration::minutes(90));
    }

    #[test]
    fn a_clone_shares_state_with_the_original() {
        let start = at(2026, 8, 26, 12, 0, 0);
        let original = TestClock::new(start);
        let handed_to_a_component = original.clone();

        original.advance(Duration::hours(3));
        assert_eq!(handed_to_a_component.now(), start + Duration::hours(3));

        // ...and the relationship is symmetric.
        handed_to_a_component.set(start);
        assert_eq!(original.now(), start);
    }

    #[test]
    fn concurrent_readers_all_observe_a_consistent_instant() {
        let start = at(2026, 8, 26, 12, 0, 0);
        let clock = TestClock::new(start);
        let target = start + Duration::hours(1);
        clock.set(target);

        let mismatches = Arc::new(AtomicUsize::new(0));
        let mut handles = Vec::new();
        for _ in 0..8 {
            let clock = clock.clone();
            let mismatches = Arc::clone(&mismatches);
            handles.push(thread::spawn(move || {
                for _ in 0..1_000 {
                    if clock.now() != target {
                        mismatches.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }

        assert_eq!(mismatches.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn concurrent_advances_are_not_lost() {
        let start = at(2026, 8, 26, 12, 0, 0);
        let clock = TestClock::new(start);

        let mut handles = Vec::new();
        for _ in 0..8 {
            let clock = clock.clone();
            handles.push(thread::spawn(move || {
                for _ in 0..500 {
                    clock.advance(Duration::milliseconds(1));
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }

        // 8 threads * 500 ms, with no lost updates.
        assert_eq!(clock.now(), start + Duration::milliseconds(4_000));
    }

    #[test]
    fn a_poisoned_lock_does_not_disable_the_clock() {
        let clock = TestClock::new(at(2026, 8, 26, 12, 0, 0));
        let poisoner = clock.clone();

        let _ = thread::spawn(move || {
            let _guard = poisoner.now.lock().unwrap();
            panic!("deliberate panic while holding the lock");
        })
        .join();

        // A panic three tests away must not cascade into this one.
        assert_eq!(clock.now(), at(2026, 8, 26, 12, 0, 0));
        clock.advance(Duration::seconds(1));
        assert_eq!(clock.now(), at(2026, 8, 26, 12, 0, 1));
    }
}
