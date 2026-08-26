//! Rhizo Edge reference device simulator.
//!
//! # Status
//!
//! M0-002 creates the binary as a workspace member. All simulator behaviour —
//! protocol conformance, the soil model, fault injection, and accelerated
//! virtual time — is the M2 deliverable (PRD 020).

#![forbid(unsafe_code)]
// Tests may `unwrap()`: a panic in a test is a failed assertion, not an
// unhandled failure (workspace lint policy, root Cargo.toml).
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

fn main() {}
