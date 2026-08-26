//! Rhizo Edge SQLite persistence.
//!
//! Holds transactions but no decisions
//! ([ADR-001](../../../docs/adr/001-rust-workspace-and-crate-boundaries.md)
//! boundary rule 3).
//!
//! # Status
//!
//! M0 creates this crate as a workspace member only. The schema, repositories,
//! and the dedup-and-persist transaction are M3 deliverables
//! ([ADR-004](../../../docs/adr/004-sqlite-edge-persistence-model.md)). M0
//! deliberately creates no database — see PRD 000 §Data model.

#![forbid(unsafe_code)]
// Tests may `unwrap()`: a panic in a test is a failed assertion, not an
// unhandled failure (workspace lint policy, root Cargo.toml).
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
