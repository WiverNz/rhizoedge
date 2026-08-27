//! Rhizo Edge MQTT v1 wire contract.
//!
//! This crate is the one piece of the workspace the ESP32 firmware also
//! compiles, so it depends on nothing else here and performs no I/O
//! ([ADR-001](../../../docs/adr/001-rust-workspace-and-crate-boundaries.md)).
//!
//! The crate is `no_std` so the host simulator and firmware compile the same
//! protocol and safety rules. The optional `std` feature adds error-trait
//! implementations only; it never changes wire behaviour.

#![no_std]
#![forbid(unsafe_code)]
#![allow(missing_docs)]
// Public wire names are defined normatively in mqtt-v1.md.
// Tests may `unwrap()`: a panic in a test is a failed assertion, not an
// unhandled failure (workspace lint policy, root Cargo.toml).
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

extern crate alloc;
#[cfg(feature = "std")]
extern crate std;

pub mod envelope;
pub mod ids;
pub mod payload;
pub mod safety;
pub mod time;
pub mod topic;
pub mod validation;

pub use envelope::{DecodeError, EncodeError, Envelope, MessageKind};
pub use ids::{BootId, CommandId, DeviceId, EventId, MessageId};
pub use safety::{CommandVerdict, DeviceGuardState, validate_water_command};
pub use time::UtcMillis;
pub use topic::{Qos, Topic, TopicMetadata};

/// Version encoded in every MQTT topic and envelope.
pub const PROTOCOL_VERSION: u16 = 1;
