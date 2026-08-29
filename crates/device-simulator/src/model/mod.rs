//! The physical model.
//!
//! Soil, water, a pot on a scale, a reservoir, and a conductivity probe. The
//! model is **simulator-only** — it is the one thing in this crate with no
//! firmware counterpart ([ADR-008](../../../../docs/adr/008-shared-code-simulator-and-firmware.md)
//! §What is shared) — and it is deliberately not a soil-physics claim.
//!
//! Every model takes elapsed milliseconds and a generator rather than reading a
//! clock or drawing entropy itself. Identical inputs, seed, and virtual time
//! therefore produce identical readings, which is what turns "the controller
//! behaved correctly" into an assertion rather than an observation.

pub mod battery;
pub mod ec;
pub mod soil;
pub mod tank;
pub mod weight;

pub use ec::{EcModel, EcParams};
pub use soil::{Delivery, SoilModel, SoilParams};
pub use tank::{TankModel, TankParams};
pub use weight::{WeightModel, WeightParams};
