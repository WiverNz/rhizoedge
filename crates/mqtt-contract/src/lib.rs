//! Rhizo Edge MQTT v1 wire contract.
//!
//! This crate is the one piece of the workspace the ESP32 firmware also
//! compiles, so it depends on nothing else here and performs no I/O
//! ([ADR-001](../../../docs/adr/001-rust-workspace-and-crate-boundaries.md)).
//!
//! # Status
//!
//! M0 creates this crate as a workspace member only. Its content — the topic
//! grammar, `DeviceId`, `UtcMillis`, the envelope, the ten payload types, and
//! `validate_water_command` — is the M1 deliverable specified by
//! `docs/protocol/mqtt-v1.md`. Nothing here is public API yet.

#![forbid(unsafe_code)]
// Tests may `unwrap()`: a panic in a test is a failed assertion, not an
// unhandled failure (workspace lint policy, root Cargo.toml).
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

#[cfg(test)]
mod tests {
    /// Proves the test harness runs for this workspace (M0-002).
    ///
    /// Deliberately trivial: M0 delivers no wire-contract behaviour, and a
    /// test that asserted otherwise would be scaffolding pretending to be
    /// coverage.
    #[test]
    fn test_harness_runs() {
        assert_eq!(1 + 1, 2);
    }
}
