//! Rhizo Edge SQLite persistence.
//!
//! Holds transactions but no decisions
//! ([ADR-001](../../../docs/adr/001-rust-workspace-and-crate-boundaries.md)
//! boundary rule 3).
//!
//! All writes are made through one logical ingestion writer. Repository write
//! methods require a transaction; reads use the pool. This API shape is the
//! enforcement boundary for ADR-004's single-writer rule.

#![forbid(unsafe_code)]
// Tests may `unwrap()`: a panic in a test is a failed assertion, not an
// unhandled failure (workspace lint policy, root Cargo.toml).
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod error;
mod migrate;
mod pool;
pub mod repo;

pub use error::StorageError;
pub use pool::EdgeDb;
