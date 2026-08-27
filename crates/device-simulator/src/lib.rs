//! Rhizo Edge reference device simulator.
//!
//! A host binary that behaves like an ESP32 plant node: it speaks exactly the
//! MQTT v1 protocol, models soil and water plausibly, injects faults on demand,
//! and runs on accelerated virtual time (PRD 020).
//!
//! # The rule this crate exists to keep
//!
//! **The simulator is never more permissive than the firmware.** There is
//! exactly one code path to actuation and it calls
//! [`rhizo_mqtt_contract::validate_water_command`]. There is no bypass flag, no
//! debug shortcut, and no second implementation of the rules
//! ([ADR-008](../../../docs/adr/008-shared-code-simulator-and-firmware.md)).
//!
//! # What is not here, deliberately
//!
//! No offline-policy evaluation and no autonomous dose scheduler. M2 models the
//! device *mechanics* of offline autonomy — capability declaration, atomic
//! policy activation, monotonic runtime state, isolation, and the bounded event
//! buffer — while the single shared `rhizo_policy::evaluate_offline` and its one
//! simulator call site arrive together in M6-019. A simulator-specific
//! evaluator would be the exact divergence ADR-008 exists to prevent.
//!
//! # Shape
//!
//! [`device::Device`] is a sans-I/O state machine: connection events, inbound
//! payloads, and elapsed virtual time in; publications out. [`mqtt`] and
//! [`runner`] are the only modules that own I/O.
//!
//! A library alongside the binary so integration tests can drive the device
//! core directly (ADR-001 boundary rule 4).

#![forbid(unsafe_code)]
// Tests may `unwrap()`: a panic in a test is a failed assertion, not an
// unhandled failure (workspace lint policy, root Cargo.toml).
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod buffer;
pub mod capabilities;
pub mod capture;
pub mod cli;
pub mod clock;
pub mod command;
pub mod config;
pub mod control;
pub mod device;
pub mod envelope;
pub mod environment;
pub mod fault;
pub mod isolation;
pub mod model;
pub mod mqtt;
pub mod offline_state;
pub mod policy;
pub mod pump;
pub mod rng;
pub mod runner;
pub mod shutdown;
pub mod state;
pub mod telemetry;
pub mod time_sync;

#[cfg(test)]
pub(crate) mod testutil;

pub use cli::{
    ActuatorList, ActuatorSpec, Cli, CliError, Fault, PolicyStep, SensorGroup, SensorList,
};
pub use device::Device;
pub use envelope::Publication;
