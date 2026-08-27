//! Pure, `no_std` shared offline-policy types.
#![no_std]
#![forbid(unsafe_code)]
#![allow(missing_docs)] // Refusal variants mirror ADR-015's named gate steps.
extern crate alloc;
pub mod types;
pub use types::*;
