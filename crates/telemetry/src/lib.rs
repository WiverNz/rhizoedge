//! Shared observability and failure-handling primitives for Rhizo Edge.
//!
//! Every host binary — `edge-controller`, `device-simulator`, `cloud-api` —
//! wires its logging, metrics, error classification, and retry timing through
//! this crate. It exists in M0 precisely so that no later milestone invents its
//! own ([ADR-010](../../../docs/adr/010-observability-strategy.md)).
//!
//! This crate depends on nothing else in the workspace.
//!
//! # Error-handling conventions
//!
//! These are project-wide rules, recorded here because this is the crate every
//! other one depends on
//! ([ADR-014](../../../docs/adr/014-failure-and-retry-policy.md)).
//!
//! ## `thiserror` in libraries, `anyhow` only at the top of a binary
//!
//! A library returns a typed error so its caller can *match* on it — and, in
//! particular, so it can implement [`Classify`] and be routed to the right
//! retry behaviour. An opaque `anyhow::Error` cannot be classified, which is
//! why it belongs only where the error is about to be logged and the process
//! is about to exit.
//!
//! ```text
//! // library crate
//! #[derive(Debug, thiserror::Error)]
//! pub enum StorageError {
//!     #[error("database is busy")]
//!     Busy,
//!     #[error("disk is full")]
//!     DiskFull,
//! }
//!
//! // binary top level, and nowhere else
//! fn main() -> anyhow::Result<()> { … }
//! ```
//!
//! ## Every error type implements [`Classify`], exhaustively
//!
//! [`Classify::classify`] maps an error onto one [`FailureKind`], and every
//! implementation matches without a catch-all arm so that a newly added
//! variant fails to compile until someone decides whether it is retryable. See
//! the [`Classify`] documentation for why that rule carries the weight here.
//!
//! ## No `unwrap()` or `expect()` in long-running paths
//!
//! `clippy::unwrap_used` and `clippy::expect_used` are denied workspace-wide
//! and allowed in tests. Where an invariant genuinely cannot be violated,
//! `expect()` is permitted with a message stating *why* it cannot fail — that
//! message is the documentation.
//!
//! ## Fatal means exit
//!
//! On [`FailureKind::Fatal`] the process logs at ERROR with full context and
//! exits non-zero. A process that is up but not evaluating safety is worse
//! than one that is down, because monitoring reports "healthy" while nothing
//! is watching the plant.

#![forbid(unsafe_code)]
// Tests may `unwrap()`: a panic in a test is a failed assertion, not an
// unhandled failure (workspace lint policy, root Cargo.toml).
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod backoff;
pub mod error;
pub mod failure;
pub mod metrics;
pub mod names;
pub mod tracing_setup;

pub use backoff::Backoff;
pub use error::TelemetryError;
pub use failure::{Classify, FailureKind};
pub use metrics::{counter, gauge, histogram, registry, render_prometheus};
pub use tracing_setup::{LogFormat, init_tracing, init_tracing_with_writer, validate_filter};
