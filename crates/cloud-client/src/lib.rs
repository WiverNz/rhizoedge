//! Rhizo Edge cloud HTTP client.
//!
//! # Status
//!
//! M0 creates this crate as a workspace member only. The outbox drain, the
//! idempotent batch upload, and the retry policy are M7 deliverables
//! ([ADR-005](../../../docs/adr/005-cloud-event-model-and-idempotency.md)).
//! Nothing in this crate may ever affect a local safety decision
//! (SAFETY-008).

#![forbid(unsafe_code)]
// Tests may `unwrap()`: a panic in a test is a failed assertion, not an
// unhandled failure (workspace lint policy, root Cargo.toml).
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
