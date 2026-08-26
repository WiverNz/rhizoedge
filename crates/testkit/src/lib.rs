//! Shared test fixtures for Rhizo Edge.
//!
//! A library crate, not a test target: it is a `dev-dependency` of the crates
//! it serves, so a fixture is written once rather than copied into each
//! milestone's tests.
//!
//! # The rule this crate exists to enforce
//!
//! **Tests advance the clock; they do not sleep.**
//!
//! [ADR-013](../../../docs/adr/013-clock-and-time-semantics.md) requires that
//! domain logic never reads the system clock, and that logical time in a test
//! moves by an explicit call. A test that sleeps is slow, is flaky on a loaded
//! CI machine, and — for anything derived from the rolling 24-hour window
//! (SAFETY-006) — is simply impossible to write.
//!
//! ```
//! use chrono::{Duration, TimeZone, Utc};
//! use rhizo_testkit::TestClock;
//!
//! // right
//! let clock = TestClock::new(Utc.with_ymd_and_hms(2026, 8, 26, 12, 0, 0).unwrap());
//! clock.advance(Duration::hours(24));
//!
//! // wrong: std::thread::sleep(std::time::Duration::from_secs(86_400));
//! ```
//!
//! # Status
//!
//! M0-010 delivers [`TestClock`]. Payload builders arrive in M1, the MQTT spy
//! in M2, and database fixtures in M3 — each with the milestone whose tests
//! first need it, so nothing here is a fixture for code that does not exist.

#![forbid(unsafe_code)]
// Tests may `unwrap()`: a panic in a test is a failed assertion, not an
// unhandled failure (workspace lint policy, root Cargo.toml).
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod clock;

pub use clock::TestClock;
