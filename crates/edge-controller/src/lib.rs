//! Rhizo Edge control plane.
//!
//! A library alongside the binary so its internals are reachable from
//! integration tests — ADR-001's boundary rule 4 makes integration tests the
//! one thing allowed to depend on `edge-controller`.
//!
//! # Status
//!
//! M0 delivered [`config`], M3 MQTT ingestion, M4 the device registry, M5 plants
//! and recommendations, and M6 irrigation control: the gate, the machine, the
//! command lifecycle, offline reconciliation, and durable command intents.
//!
//! **Every path from a decision to the wire goes through
//! [`control::command::Commander`]**, which persists before it publishes and
//! never mints a second `command_id` for one dose. An HTTP handler or a control
//! pass with its own MQTT client would be a second actuation path, and the
//! safety invariants would then hold for only one of them.

#![forbid(unsafe_code)]
// Tests may `unwrap()`: a panic in a test is a failed assertion, not an
// unhandled failure (workspace lint policy, root Cargo.toml).
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod api;
pub mod clock;
pub mod cloud;
pub mod config;
pub mod control;
pub mod device;
pub mod error;
pub mod metrics;
pub mod mqtt;
pub mod pipeline;
pub mod plant;
pub mod retention;
pub mod state;
pub mod supervisor;
