//! Rhizo Edge domain logic.
//!
//! Pure by construction: no I/O, and no direct clock access — time arrives
//! through the `Clock` trait so the safety property tests are deterministic
//! ([ADR-013](../../../docs/adr/013-clock-and-time-semantics.md)).
//!
//! M1 defined the stable type vocabulary. M5 added the analysis and
//! configuration logic the recommendation engine needs. M6 adds
//! [`irrigation`]: the safety gate, the pure total state machine, the rolling
//! budget, and no-delivery detection.
//!
//! **This crate still cannot water a plant.** It decides; it constructs no
//! command, holds no MQTT client, and touches no database. `evaluate` is the
//! only public decision function and the safety gate is its first statement, so
//! there is no second path a refusal would never be consulted for.
//!
//! **SAFETY-009, structurally.** There is no dependency on
//! `rhizo-cloud-client` and no field of `IrrigationInputs` is derived from cloud
//! state, so a cloud outage cannot change a watering answer: there is nowhere
//! for a cloud fact to enter.

#![forbid(unsafe_code)]
#![deny(clippy::disallowed_methods)]
#![allow(missing_docs)]
// Stable enum vocabulary is defined in PRD-010.
// Tests may `unwrap()`: a panic in a test is a failed assertion, not an
// unhandled failure (workspace lint policy, root Cargo.toml).
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod binding;
pub mod clock;
pub mod detect;
pub mod dry_duration;
pub mod ec;
pub mod ids;
pub mod irrigation;
pub mod measurement_policy;
pub mod offline_policy;
pub mod plant;
pub mod plant_state;
pub mod preset;
pub mod profile;
pub mod recommend;
pub mod state;
pub mod stuck;
pub mod threshold;
pub mod trend;

pub use clock::{Clock, SystemClock};
pub use ids::{PlantId, ProfileId, WateringEventId};
pub use irrigation::{IrrigationDecision, IrrigationInputs, evaluate, safety_gate};
pub use plant::*;
pub use profile::{PlantProfile, ProfileError, SoilSample};
pub use recommend::{Decision, Reason, Recommendation, RecommendationInputs, recommend};
pub use state::*;
