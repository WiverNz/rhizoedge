//! Rhizo Edge control plane.
//!
//! A library alongside the binary so its internals are reachable from
//! integration tests — ADR-001's boundary rule 4 makes integration tests the
//! one thing allowed to depend on `edge-controller`.
//!
//! # Status
//!
//! M0 delivers [`config`]: layered loading with fail-fast validation and
//! secret redaction. MQTT ingestion (M3), the device registry (M4), plants and
//! recommendations (M5), and irrigation control (M6) follow. Nothing here
//! speaks to a broker or a database yet.

#![forbid(unsafe_code)]
// Tests may `unwrap()`: a panic in a test is a failed assertion, not an
// unhandled failure (workspace lint policy, root Cargo.toml).
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod config;
