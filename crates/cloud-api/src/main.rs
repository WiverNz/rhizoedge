//! Rhizo Edge cloud replica API.
//!
//! # Status
//!
//! M0-002 creates the binary as a workspace member. Idempotent event ingest,
//! projections, and the read APIs are M7 deliverables (PRD 070).

#![forbid(unsafe_code)]
// Tests may `unwrap()`: a panic in a test is a failed assertion, not an
// unhandled failure (workspace lint policy, root Cargo.toml).
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

fn main() {}
