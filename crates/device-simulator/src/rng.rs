//! Deterministic randomness.
//!
//! Two generators, deliberately separate:
//!
//! - a **model** generator seeded from `--seed`, driving sensor noise and
//!   rate-based faults. Identical inputs, seed, and virtual time must produce
//!   identical readings, which is what makes a physical-model test an assertion
//!   rather than a hope.
//! - an **identity** generator seeded from the operating system at boot,
//!   producing `message_id`, `batch_id`, `boot_id`, and `event_id`. These must
//!   *not* be reproducible: a restart replaying the same `--seed` would re-emit
//!   `message_id` values the edge has already deduplicated, and the telemetry
//!   after a restart would be silently discarded.
//!
//! SplitMix64 is written out here rather than taken from a dependency, for the
//! same reason `rhizo_telemetry::backoff::SeededJitter` is: a test that pins a
//! noise sequence should fail when the *model* changes, not when an unrelated
//! RNG crate changes its internals.

use rhizo_mqtt_contract::ids::RandomSource;

/// A SplitMix64 generator.
#[derive(Clone, Debug)]
pub struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    /// Creates a generator that produces the same sequence for the same seed,
    /// forever.
    #[must_use]
    pub const fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    /// Creates a generator seeded from the operating system.
    #[must_use]
    pub fn from_os() -> Self {
        Self::new(rand::random())
    }

    /// The next raw 64-bit value.
    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A uniform value in `[0, 1)`.
    pub fn next_unit(&mut self) -> f64 {
        // 53 bits is the full mantissa of an f64: using all 64 would round and
        // could return exactly 1.0, which breaks `< rate` comparisons at 1.0.
        ((self.next_u64() >> 11) as f64) * (1.0 / (1u64 << 53) as f64)
    }

    /// Whether an event of the given probability occurs.
    ///
    /// Rate `0.0` never fires and rate `1.0` always does — the two values a
    /// fault-injection test relies on being exact.
    pub fn chance(&mut self, rate: f64) -> bool {
        // `rate <= 0.0` would let a NaN through; asking for "not greater" is
        // the comparison that treats an unusable rate as "never fires".
        if !matches!(rate.partial_cmp(&0.0), Some(core::cmp::Ordering::Greater)) {
            return false;
        }
        if rate >= 1.0 {
            return true;
        }
        self.next_unit() < rate
    }

    /// A sample from a standard normal distribution, scaled by `sigma`.
    ///
    /// Box-Muller, using one of the two values it produces. Discarding the
    /// second costs one extra draw and keeps the generator stateless beyond its
    /// seed, so a caller cannot observe a different sequence depending on how
    /// many samples were taken earlier in a *different* order.
    pub fn gaussian(&mut self, sigma: f64) -> f64 {
        if sigma == 0.0 {
            return 0.0;
        }
        // `u1` must be strictly positive: ln(0) is negative infinity.
        let u1 = self.next_unit().max(f64::MIN_POSITIVE);
        let u2 = self.next_unit();
        let magnitude = (-2.0 * u1.ln()).sqrt();
        magnitude * (core::f64::consts::TAU * u2).cos() * sigma
    }
}

impl RandomSource for SplitMix64 {
    fn fill_bytes(&mut self, output: &mut [u8]) {
        for chunk in output.chunks_mut(8) {
            let bytes = self.next_u64().to_le_bytes();
            let n = chunk.len();
            chunk.copy_from_slice(&bytes[..n]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_seed_reproduces_the_same_sequence_and_a_different_one_diverges() {
        let draw = |seed| {
            let mut g = SplitMix64::new(seed);
            (0..64).map(|_| g.next_u64()).collect::<Vec<_>>()
        };
        assert_eq!(draw(7), draw(7));
        assert_ne!(draw(7), draw(8));
    }

    #[test]
    fn unit_draws_stay_inside_the_half_open_interval() {
        let mut g = SplitMix64::new(99);
        for _ in 0..10_000 {
            let u = g.next_unit();
            assert!((0.0..1.0).contains(&u), "{u} escaped [0, 1)");
        }
    }

    #[test]
    fn the_extreme_rates_are_exact_rather_than_probable() {
        let mut g = SplitMix64::new(1);
        for _ in 0..1_000 {
            assert!(!g.chance(0.0));
            assert!(g.chance(1.0));
            assert!(!g.chance(f64::NAN), "an unusable rate never fires");
        }
    }

    #[test]
    fn a_rate_is_approximately_honoured() {
        let mut g = SplitMix64::new(4242);
        let hits = (0..20_000).filter(|_| g.chance(0.25)).count();
        assert!(
            (4_500..5_500).contains(&hits),
            "saw {hits} of an expected ~5000"
        );
    }

    #[test]
    fn gaussian_noise_has_the_requested_spread_and_no_offset() {
        let mut g = SplitMix64::new(31337);
        let n = 20_000;
        let samples: Vec<f64> = (0..n).map(|_| g.gaussian(0.3)).collect();
        let mean = samples.iter().sum::<f64>() / n as f64;
        let variance = samples.iter().map(|s| (s - mean).powi(2)).sum::<f64>() / n as f64;
        assert!(mean.abs() < 0.02, "mean {mean} is not centred on zero");
        assert!(
            (variance.sqrt() - 0.3).abs() < 0.02,
            "sigma {} is not 0.3",
            variance.sqrt()
        );
    }

    #[test]
    fn zero_sigma_is_exactly_zero_and_draws_nothing() {
        let mut g = SplitMix64::new(5);
        let before = g.clone().next_u64();
        assert_eq!(g.gaussian(0.0), 0.0);
        assert_eq!(g.next_u64(), before, "a no-op must not advance the stream");
    }

    #[test]
    fn filling_a_uuid_worth_of_bytes_uses_every_byte() {
        let mut g = SplitMix64::new(2026);
        let mut bytes = [0u8; 16];
        g.fill_bytes(&mut bytes);
        assert!(bytes.iter().any(|b| *b != 0));
        let mut odd = [0u8; 5];
        g.fill_bytes(&mut odd);
        assert!(
            odd.iter().any(|b| *b != 0),
            "a short buffer is still filled"
        );
    }
}
