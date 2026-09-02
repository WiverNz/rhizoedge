//! Randomness for identifier generation.
//!
//! `esp_random` is the hardware RNG. It is documented as producing true random
//! numbers only while the RF subsystem is enabled; before Wi-Fi is up it
//! degrades to a pseudo-random source. That is acceptable here and nowhere
//! else: the only consumer is UUID generation, where the requirement is
//! *uniqueness within a fleet*, not unpredictability. Nothing in this firmware
//! derives a key or a nonce.

use rhizo_node_app::ports::Rng;

/// The ESP32 hardware random number generator.
#[derive(Clone, Copy, Debug, Default)]
pub struct EspRng;

impl Rng for EspRng {
    fn fill(&mut self, out: &mut [u8]) {
        // SAFETY: `esp_fill_random` writes exactly `len` bytes to `buf` and has
        // no other preconditions.
        unsafe {
            esp_idf_sys::esp_fill_random(out.as_mut_ptr().cast(), out.len());
        }
    }
}
