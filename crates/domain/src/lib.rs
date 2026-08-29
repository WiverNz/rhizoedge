//! Rhizo Edge domain logic.
//!
//! Pure by construction: no I/O, and no direct clock access — time arrives
//! through the `Clock` trait so the safety property tests are deterministic
//! ([ADR-013](../../../docs/adr/013-clock-and-time-semantics.md)).
//!
//! M1 defined the stable type vocabulary. M5 adds the analysis and
//! configuration logic the recommendation engine needs: profile validation,
//! trends, dry-duration tracking, manual-watering detection, stuck-sensor
//! detection, the rule-based recommendation engine, plant-state derivation, EC
//! trending, binding and per-measurement-policy validation, threshold
//! evaluation, offline-policy authoring, and the embedded species preset
//! catalogue.
//!
//! **M5 issues no commands.** Nothing in this crate constructs a water command,
//! and nothing in it can water a plant. That separation is what lets the
//! recommendation logic be validated against a real plant for a week before M6
//! gives it a pump.

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
pub use plant::*;
pub use profile::{PlantProfile, ProfileError, SoilSample};
pub use recommend::{Decision, Reason, Recommendation, RecommendationInputs, recommend};
pub use state::*;
