//! The ESP32 plant node's application logic, host-testable with fake adapters.
//!
//! # Why this is a crate and not a module
//!
//! [ADR-007](../../../docs/adr/007-esp32-rust-framework-and-toolchain.md)
//! requires that the application layer contain **no `esp_idf_*` import**, so
//! that SAFETY-002, -007, -011, and -013…-021 are covered by `cargo test` with
//! no board attached. Its illustrative layout puts that layer in
//! `firmware/esp32-node/src/app/` and enforces the rule with a grep. This crate
//! achieves the same isolation structurally instead: it does not depend on
//! `esp-idf-sys` at all, so an ESP-IDF symbol here does not compile. The ADR
//! states plainly that "a cleaner separation that achieves the same isolation is
//! acceptable, and the isolation is the requirement".
//!
//! Two things fall out of that choice and both are improvements:
//!
//! * these tests run on the **host toolchain pin** (1.98.0), so the safety
//!   logic is compiled by the same compiler as the rest of the project rather
//!   than by the nightly `riscv32imc-esp-espidf` needs (M9-001);
//! * `cargo test` here needs no target flag, no `build-std`, and no ESP-IDF
//!   installation, so a contributor with none of that can still run the safety
//!   suite.
//!
//! `firmware/esp32-node` depends on this crate by path and supplies the ESP-IDF
//! implementations of [`ports`].
//!
//! # What lives where
//!
//! | Concern | Module |
//! |---|---|
//! | hardware abstraction and its fakes | [`ports`], [`fakes`] |
//! | boot ordering (SAFETY-011) | [`boot`] |
//! | persistent state and checksums | [`persist`] |
//! | identity and boot identity | [`identity`] |
//! | the one actuation path | [`command`] |
//! | pending-result ledger (F-090-17…19) | [`ledger`] |
//! | command dedup ring (SAFETY-001) | [`dedup`] |
//! | interrupted dose (SAFETY-011) | [`recovery`] |
//! | configuration (§5.7) | [`config`] |
//! | policy store and activation (SAFETY-019) | [`policy`] |
//! | the one offline evaluator call site | [`offline`] |
//! | monotonic budget and cooldown (SAFETY-015) | [`budget`] |
//! | event buffer and gaps (SAFETY-016, -020) | [`buffer`] |
//! | power mode and the wake cycle (ADR-018) | [`power`], [`awake_hold`] |
//! | rails and warm-up | [`sampling`] |
//! | telemetry and status | [`telemetry`] |
//! | serial provisioning | [`provision`] |
//!
//! # The two rules this crate exists to keep
//!
//! **One actuation gate.** `rhizo_mqtt_contract::validate_water_command` is
//! called from exactly one place ([`command::authorise`]) and the pump is
//! driven from exactly one place ([`command::execute_authorised`]).
//!
//! **One offline evaluator.** `rhizo_policy::evaluate_offline` is called from
//! exactly one place ([`offline::evaluate_and_act`]).
//!
//! `tests/single_actuation_path.rs` checks both against the source text, the
//! same way the simulator's does.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
// Tests may `unwrap()`: a panic in a test is a failed assertion, not an
// unhandled failure (workspace lint policy, root Cargo.toml).
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod awake_hold;
pub mod boot;
pub mod budget;
pub mod buffer;
pub mod command;
pub mod config;
pub mod dedup;
pub mod fakes;
pub mod identity;
pub mod ledger;
pub mod offline;
pub mod persist;
pub mod policy;
pub mod ports;
pub mod power;
pub mod provision;
pub mod recovery;
pub mod sampling;
pub mod telemetry;

/// The firmware version reported in `device.status`.
pub const FIRMWARE_VERSION: &str = concat!("rhizo-node-", env!("CARGO_PKG_VERSION"));

/// The compile-time limits reported read-only in `device.status`.
///
/// Read from the shared contract crate, never from configuration: no message
/// can change them (ADR-011, SAFETY-007).
#[must_use]
pub fn reported_limits() -> rhizo_mqtt_contract::payload::ReportedLimits {
    rhizo_mqtt_contract::payload::ReportedLimits {
        max_run_seconds: rhizo_mqtt_contract::safety::FIRMWARE_MAX_RUN_SECONDS,
        max_ml_per_run: rhizo_mqtt_contract::safety::FIRMWARE_MAX_ML_PER_RUN,
        max_daily_ml: rhizo_mqtt_contract::safety::FIRMWARE_MAX_DAILY_ML,
    }
}
