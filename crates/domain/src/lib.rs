//! Rhizo Edge domain logic.
//!
//! Pure by construction: no I/O, and no direct clock access — time arrives
//! through the `Clock` trait so the safety property tests are deterministic
//! ([ADR-013](../../../docs/adr/013-clock-and-time-semantics.md)).
//!
//! # Status
//!
//! M0 creates this crate as a workspace member only. The `Clock` trait lands
//! in M1-012, the plant model in M5, and the irrigation state machine and
//! safety gate in M6.

#![forbid(unsafe_code)]
// Tests may `unwrap()`: a panic in a test is a failed assertion, not an
// unhandled failure (workspace lint policy, root Cargo.toml).
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
