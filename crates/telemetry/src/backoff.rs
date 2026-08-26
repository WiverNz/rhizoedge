//! Exponential backoff with full jitter.
//!
//! The single retry-timing implementation in the project. All five retry sites
//! — MQTT connect, MQTT publish, SQLite `BUSY`, cloud sync, and device Wi-Fi
//! association — use this type
//! ([ADR-014](../../../../docs/adr/014-failure-and-retry-policy.md)).
//!
//! # Why full jitter
//!
//! ```text
//! delay(attempt) = random_uniform(0, min(cap, base * 2^attempt))
//! ```
//!
//! Not "exponential plus a small random addition". With a fleet of devices and
//! an edge all reconnecting after a broker restart, the naive form *preserves*
//! the synchronised retry pattern that caused the outage — every client waits
//! roughly the same time and hits the recovering broker together. Full jitter
//! spreads attempts uniformly across the whole window.
//!
//! # The short delays are correct
//!
//! Because the draw is uniform over the whole interval, `next_delay` can
//! legitimately return a few microseconds even at attempt 20. This looks like
//! a bug and is not one: it is the property that de-synchronises clients, and
//! the occasional early retry costs nothing at this scale. **Do not add a
//! minimum delay.** Doing so re-introduces exactly the synchronisation the
//! jitter exists to remove.
//!
//! # Parameters
//!
//! ADR-014 §Backoff is normative. Each site supplies its own:
//!
//! | Site | base | cap | max attempts |
//! |---|---|---|---|
//! | MQTT connection (edge) | 1 s | 60 s | unlimited |
//! | MQTT connection (device) | 2 s | 300 s | unlimited |
//! | MQTT publish (command) | 200 ms | 2 s | 3, then fail the command |
//! | SQLite transaction on `BUSY` | 50 ms | 500 ms | 3 |
//! | Cloud sync batch | 1 s | 300 s | unlimited |
//! | Device Wi-Fi association | 2 s | 300 s | unlimited |
//!
//! This type does **not** enforce a maximum attempt count. Giving up is a
//! decision about the operation, not about its timing — and for command
//! publication it is the decision ADR-014 cares most about, since a fresh
//! `command_id` after a failed publish is the most plausible route to
//! duplicate watering in the whole design.

use std::time::Duration;

use rand::Rng;

/// The source of jitter.
///
/// Injectable so tests can pin the sequence: [`Backoff::next_delay`] is
/// otherwise unassertable beyond its bounds. Production uses [`OsJitter`];
/// tests use [`SeededJitter`].
pub trait Jitter: Send {
    /// Returns a uniformly distributed value in the inclusive range
    /// `[0, upper]`.
    ///
    /// Implementations must handle `upper == 0` by returning `0`.
    fn uniform_upto(&mut self, upper: u64) -> u64;
}

/// Jitter drawn from the operating system's generator.
///
/// The default for production. Quality beyond "uniform" is irrelevant here —
/// this de-synchronises clients, it does not protect anything.
#[derive(Debug, Default)]
pub struct OsJitter;

impl Jitter for OsJitter {
    fn uniform_upto(&mut self, upper: u64) -> u64 {
        if upper == 0 {
            return 0;
        }
        rand::rng().random_range(0..=upper)
    }
}

/// Deterministic jitter for tests.
///
/// A SplitMix64 generator, written out here rather than taken from a
/// dependency so that the "same seed, same sequence" guarantee holds across
/// upgrades of every crate in the tree. A test that pins a delay sequence
/// should fail when the *backoff* changes, not when an unrelated RNG library
/// changes its internals.
#[derive(Debug, Clone)]
pub struct SeededJitter {
    state: u64,
}

impl SeededJitter {
    /// Creates a generator that will produce the same sequence for the same
    /// seed, forever.
    #[must_use]
    pub const fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    /// SplitMix64.
    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

impl Jitter for SeededJitter {
    fn uniform_upto(&mut self, upper: u64) -> u64 {
        if upper == 0 {
            return 0;
        }
        // Modulo introduces a bias of at most one part in 2^64 / upper. For a
        // jitter window this is immeasurable, and the alternative (rejection
        // sampling) would make the sequence depend on how many draws were
        // rejected, which is the opposite of what a test needs.
        self.next_u64() % (upper.saturating_add(1))
    }
}

/// Full-jitter exponential backoff.
///
/// ```
/// use std::time::Duration;
/// use rhizo_telemetry::backoff::{Backoff, SeededJitter};
///
/// let mut b = Backoff::with_jitter(
///     Duration::from_millis(50),
///     Duration::from_millis(500),
///     SeededJitter::new(7),
/// );
///
/// // Each delay lies in [0, min(cap, base * 2^attempt)].
/// let first = b.next_delay();
/// assert!(first <= Duration::from_millis(50));
///
/// // On success, start over.
/// b.reset();
/// assert_eq!(b.attempt(), 0);
/// ```
pub struct Backoff<J: Jitter = OsJitter> {
    base: Duration,
    cap: Duration,
    attempt: u32,
    jitter: J,
}

impl Backoff<OsJitter> {
    /// Creates a backoff using the operating system's generator.
    ///
    /// `base` is the first attempt's window; `cap` bounds every window. A
    /// `cap` below `base` is honoured — the window is `min(cap, …)`, so the
    /// result is simply a constant-window jittered retry.
    #[must_use]
    pub fn new(base: Duration, cap: Duration) -> Self {
        Self::with_jitter(base, cap, OsJitter)
    }
}

impl<J: Jitter> Backoff<J> {
    /// Creates a backoff with an explicit jitter source.
    #[must_use]
    pub const fn with_jitter(base: Duration, cap: Duration, jitter: J) -> Self {
        Self {
            base,
            cap,
            attempt: 0,
            jitter,
        }
    }

    /// The number of delays produced since construction or the last
    /// [`reset`](Self::reset).
    #[must_use]
    pub const fn attempt(&self) -> u32 {
        self.attempt
    }

    /// The configured first-attempt window.
    #[must_use]
    pub const fn base(&self) -> Duration {
        self.base
    }

    /// The configured maximum window.
    #[must_use]
    pub const fn cap(&self) -> Duration {
        self.cap
    }

    /// The upper bound of the window the *next* call to
    /// [`next_delay`](Self::next_delay) will draw from: `min(cap, base * 2^attempt)`.
    ///
    /// Saturating throughout. At attempt 64 a naive `base << attempt` would
    /// overflow `u64` nanoseconds and — worse than panicking — would wrap to a
    /// tiny window, turning a long outage into a hot retry loop. The shift is
    /// clamped before the multiply, and the multiply saturates.
    #[must_use]
    pub fn window(&self) -> Duration {
        let base_nanos = self.base.as_nanos().min(u128::from(u64::MAX)) as u64;
        let cap_nanos = self.cap.as_nanos().min(u128::from(u64::MAX)) as u64;

        // `1u64 << 64` is undefined; clamp the exponent first. Beyond 63 the
        // multiply would saturate anyway for any non-zero base.
        let factor = if self.attempt >= 64 {
            u64::MAX
        } else {
            1u64 << self.attempt
        };

        let grown = base_nanos.saturating_mul(factor);
        Duration::from_nanos(grown.min(cap_nanos))
    }

    /// Draws the next delay and advances the attempt counter.
    ///
    /// The result is uniform over `[0, window()]` — see the module docs for why
    /// a very short delay is correct rather than a bug.
    ///
    /// The attempt counter saturates at [`u32::MAX`], so a caller that retries
    /// forever cannot wrap it back to a short window.
    pub fn next_delay(&mut self) -> Duration {
        let window = self.window();
        let upper = window.as_nanos().min(u128::from(u64::MAX)) as u64;
        let drawn = self.jitter.uniform_upto(upper);
        self.attempt = self.attempt.saturating_add(1);
        Duration::from_nanos(drawn)
    }

    /// Returns to the first attempt.
    ///
    /// Called on success. ADR-014: "the attempt counter resets on success".
    pub fn reset(&mut self) {
        self.attempt = 0;
    }
}

impl<J: Jitter + std::fmt::Debug> std::fmt::Debug for Backoff<J> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Backoff")
            .field("base", &self.base)
            .field("cap", &self.cap)
            .field("attempt", &self.attempt)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: Duration = Duration::from_millis(50);
    const CAP: Duration = Duration::from_millis(500);

    fn seeded(seed: u64) -> Backoff<SeededJitter> {
        Backoff::with_jitter(BASE, CAP, SeededJitter::new(seed))
    }

    #[test]
    fn window_doubles_until_it_reaches_the_cap() {
        let mut b = seeded(1);
        assert_eq!(b.window(), Duration::from_millis(50));
        b.next_delay();
        assert_eq!(b.window(), Duration::from_millis(100));
        b.next_delay();
        assert_eq!(b.window(), Duration::from_millis(200));
        b.next_delay();
        assert_eq!(b.window(), Duration::from_millis(400));
        b.next_delay();
        // 800 ms would exceed the 500 ms cap.
        assert_eq!(b.window(), CAP);
    }

    #[test]
    fn no_delay_ever_exceeds_the_cap() {
        let mut b = seeded(42);
        for _ in 0..1_000 {
            assert!(b.next_delay() <= CAP);
        }
    }

    #[test]
    fn every_delay_is_within_its_own_window() {
        let mut b = seeded(9);
        for _ in 0..200 {
            let window = b.window();
            let d = b.next_delay();
            assert!(d <= window, "{d:?} exceeded window {window:?}");
        }
    }

    #[test]
    fn a_thousand_attempts_neither_panic_nor_wrap() {
        let mut b = seeded(3);
        for i in 0..1_000 {
            let d = b.next_delay();
            assert!(d <= CAP, "attempt {i} produced {d:?}, above the cap");
        }
        assert_eq!(b.attempt(), 1_000);
        // The window is still the cap, not a wrapped-around tiny value.
        assert_eq!(b.window(), CAP);
    }

    #[test]
    fn an_extreme_attempt_count_saturates_rather_than_overflowing() {
        // A base large enough that `base * 2^attempt` overflows u64 nanoseconds
        // well before attempt 64.
        let huge_base = Duration::from_secs(3600);
        let mut b = Backoff::with_jitter(huge_base, CAP, SeededJitter::new(5));
        for _ in 0..70 {
            assert!(b.next_delay() <= CAP);
        }
        assert_eq!(b.window(), CAP);
    }

    #[test]
    fn the_attempt_counter_saturates_instead_of_wrapping() {
        let mut b = seeded(11);
        b.attempt = u32::MAX;
        let d = b.next_delay();
        assert_eq!(b.attempt(), u32::MAX, "must saturate, not wrap to 0");
        assert!(d <= CAP);
    }

    #[test]
    fn reset_returns_to_the_first_attempt() {
        let mut b = seeded(13);
        for _ in 0..10 {
            b.next_delay();
        }
        assert_eq!(b.window(), CAP);

        b.reset();

        assert_eq!(b.attempt(), 0);
        assert_eq!(b.window(), BASE, "the window is back to `base`");
    }

    #[test]
    fn a_seeded_generator_reproduces_its_sequence_exactly() {
        let collect = |seed| {
            let mut b = seeded(seed);
            (0..25).map(|_| b.next_delay()).collect::<Vec<_>>()
        };
        assert_eq!(collect(2024), collect(2024));
        assert_ne!(
            collect(2024),
            collect(2025),
            "different seeds must diverge, or the tests prove nothing"
        );
    }

    #[test]
    fn a_zero_window_yields_a_zero_delay_without_dividing_by_zero() {
        let mut b = Backoff::with_jitter(Duration::ZERO, Duration::ZERO, SeededJitter::new(1));
        assert_eq!(b.next_delay(), Duration::ZERO);
        assert_eq!(b.next_delay(), Duration::ZERO);
    }

    #[test]
    fn a_cap_below_the_base_clamps_from_the_very_first_attempt() {
        let mut b = Backoff::with_jitter(
            Duration::from_secs(10),
            Duration::from_millis(5),
            SeededJitter::new(1),
        );
        assert_eq!(b.window(), Duration::from_millis(5));
        assert!(b.next_delay() <= Duration::from_millis(5));
    }

    #[test]
    fn full_jitter_actually_spreads_delays_across_the_window() {
        // The property that matters operationally: at a wide window, delays
        // must land across it rather than clustering near the top. A
        // "exponential plus a little randomness" implementation would fail
        // this — which is the point of asserting it.
        let mut b = Backoff::with_jitter(CAP, CAP, SeededJitter::new(77));
        let mut below_a_tenth = 0;
        let mut above_nine_tenths = 0;
        for _ in 0..1_000 {
            let d = b.next_delay();
            if d < CAP / 10 {
                below_a_tenth += 1;
            }
            if d > CAP * 9 / 10 {
                above_nine_tenths += 1;
            }
        }
        assert!(
            below_a_tenth > 50,
            "expected ~100 draws in the bottom tenth, saw {below_a_tenth}"
        );
        assert!(
            above_nine_tenths > 50,
            "expected ~100 draws in the top tenth, saw {above_nine_tenths}"
        );
    }

    #[test]
    fn the_os_jitter_source_also_respects_the_bounds() {
        // The seeded generator is what the other tests use; this one proves
        // the production path is wired to the same arithmetic.
        let mut b = Backoff::new(BASE, CAP);
        for _ in 0..200 {
            let window = b.window();
            let d = b.next_delay();
            assert!(d <= window);
            assert!(d <= CAP);
        }
    }

    #[test]
    fn debug_does_not_expose_the_jitter_state() {
        let b = seeded(1);
        let s = format!("{b:?}");
        assert!(s.contains("attempt"));
        assert!(s.contains("base"));
        assert!(!s.contains("state"), "generator state is not useful output");
    }
}
