//! Pure, `no_std` shared offline-policy types.
#![no_std]
#![forbid(unsafe_code)]
#![allow(missing_docs)]
// Refusal variants mirror ADR-015's named gate steps.
// Tests may `unwrap()`: a panic in a test is a failed assertion, not an
// unhandled failure (workspace lint policy, root Cargo.toml).
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
extern crate alloc;
pub mod evaluate;
pub mod gate;
pub mod types;
pub mod validate;
pub use evaluate::{evaluate_offline, next_offline_state};
pub use gate::offline_gate;
pub use types::*;
pub use validate::validate_authored;
