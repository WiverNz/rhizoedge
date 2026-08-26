//! Property tests for the shared backoff (M0-007).
//!
//! Run a large sweep with:
//!
//! ```text
//! PROPTEST_CASES=10000 cargo test -p rhizo-telemetry backoff
//! ```
//!
//! The invariant under test is the one every retry site depends on: whatever
//! the parameters and however many attempts have been made, the delay lies in
//! `[0, min(cap, base * 2^attempt)]` and the arithmetic never overflows.

// A panic in a test is a failed assertion, not an unhandled failure.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::Duration;

use proptest::prelude::*;
use rhizo_telemetry::backoff::{Backoff, SeededJitter};

proptest! {
    /// The core bound, over arbitrary parameters and attempt counts.
    #[test]
    fn backoff_delay_is_always_within_the_window(
        base_ms in 0u64..10_000,
        cap_ms in 0u64..600_000,
        attempts in 0usize..80,
        seed: u64,
    ) {
        let base = Duration::from_millis(base_ms);
        let cap = Duration::from_millis(cap_ms);
        let mut b = Backoff::with_jitter(base, cap, SeededJitter::new(seed));

        for _ in 0..attempts {
            let window = b.window();
            let expected_cap = base.min(cap).max(Duration::ZERO);
            prop_assert!(
                window <= cap,
                "window {window:?} exceeded cap {cap:?}"
            );
            prop_assert!(
                window >= expected_cap || window == cap,
                "window {window:?} shrank below min(base, cap) {expected_cap:?}"
            );

            let delay = b.next_delay();
            prop_assert!(delay <= window, "delay {delay:?} exceeded window {window:?}");
            prop_assert!(delay <= cap, "delay {delay:?} exceeded cap {cap:?}");
        }
    }

    /// The window grows monotonically until it reaches the cap, and never
    /// shrinks afterwards — the failure mode a wrapping shift would produce.
    #[test]
    fn backoff_window_is_monotonic_up_to_the_cap(
        base_ms in 1u64..5_000,
        cap_ms in 1u64..600_000,
        seed: u64,
    ) {
        let base = Duration::from_millis(base_ms);
        let cap = Duration::from_millis(cap_ms);
        let mut b = Backoff::with_jitter(base, cap, SeededJitter::new(seed));

        let mut previous = b.window();
        for _ in 0..100 {
            b.next_delay();
            let current = b.window();
            prop_assert!(
                current >= previous,
                "window shrank from {previous:?} to {current:?}"
            );
            previous = current;
        }
        prop_assert_eq!(previous, cap.min(previous), "window settled above the cap");
    }

    /// `reset()` is a true return to the initial state: the window after a
    /// reset equals the window at construction.
    #[test]
    fn backoff_reset_restores_the_initial_window(
        base_ms in 1u64..5_000,
        cap_ms in 1u64..600_000,
        attempts in 0usize..50,
        seed: u64,
    ) {
        let base = Duration::from_millis(base_ms);
        let cap = Duration::from_millis(cap_ms);
        let mut b = Backoff::with_jitter(base, cap, SeededJitter::new(seed));

        let initial = b.window();
        for _ in 0..attempts {
            b.next_delay();
        }
        b.reset();

        prop_assert_eq!(b.attempt(), 0);
        prop_assert_eq!(b.window(), initial);
    }

    /// Extreme attempt counts saturate. `base * 2^attempt` overflows `u64`
    /// nanoseconds well before attempt 1000, and a wrap would turn a long
    /// outage into a hot retry loop.
    #[test]
    fn backoff_survives_pathological_attempt_counts(
        base_ms in 1u64..100_000,
        cap_ms in 1u64..600_000,
        seed: u64,
    ) {
        let cap = Duration::from_millis(cap_ms);
        let mut b = Backoff::with_jitter(
            Duration::from_millis(base_ms),
            cap,
            SeededJitter::new(seed),
        );

        for _ in 0..1_000 {
            b.next_delay();
        }
        prop_assert_eq!(b.attempt(), 1_000);
        prop_assert_eq!(b.window(), cap, "the window must sit at the cap, not wrap");
        prop_assert!(b.next_delay() <= cap);
    }

    /// Same seed, same sequence — the guarantee every deterministic test of a
    /// retry loop will rely on from M3 onward.
    #[test]
    fn backoff_is_reproducible_for_a_given_seed(seed: u64) {
        let run = |s: u64| {
            let mut b = Backoff::with_jitter(
                Duration::from_millis(50),
                Duration::from_millis(500),
                SeededJitter::new(s),
            );
            (0..32).map(|_| b.next_delay()).collect::<Vec<_>>()
        };
        prop_assert_eq!(run(seed), run(seed));
    }
}
