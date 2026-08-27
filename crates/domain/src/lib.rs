//! Rhizo Edge domain logic.
//!
//! Pure by construction: no I/O, and no direct clock access — time arrives
//! through the `Clock` trait so the safety property tests are deterministic
//! ([ADR-013](../../../docs/adr/013-clock-and-time-semantics.md)).
//!
//! M1 defines only stable types and pure primitives. Recommendation and state
//! transition behaviour remains in later milestones.

#![forbid(unsafe_code)]
#![deny(clippy::disallowed_methods)]
#![allow(missing_docs)]
// Stable enum vocabulary is defined in PRD-010.
// Tests may `unwrap()`: a panic in a test is a failed assertion, not an
// unhandled failure (workspace lint policy, root Cargo.toml).
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod clock;
pub mod ids;
pub mod plant;
pub mod profile;
pub mod state;

pub use clock::{Clock, SystemClock};
pub use ids::{PlantId, ProfileId, WateringEventId};
pub use plant::*;
pub use profile::{PlantProfile, SoilSample};
pub use state::*;
