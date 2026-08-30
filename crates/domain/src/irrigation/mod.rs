//! Connected-mode irrigation: the safety gate, the state machine, the rolling
//! budget, and no-delivery detection (M6).
//!
//! The whole module is pure. Nothing here reads a clock, touches SQLite, or
//! speaks to a broker: the caller loads state, calls [`evaluate`], and persists
//! the result together with its side effect in one transaction
//! ([ADR-006](../../../../docs/adr/006-irrigation-state-machine-ownership.md),
//! F-060-13, F-060-14).
//!
//! There is exactly **one** public decision function, [`evaluate`], and the
//! safety gate is its first statement. A second decision path that skipped the
//! gate would nullify SAFETY-003, SAFETY-004, and SAFETY-005 while looking
//! perfectly reasonable in review, so the module exposes none.
//!
//! The **offline** evaluator an isolated device runs is a different, smaller
//! function in a different, `no_std` crate: [`rhizo_policy::evaluate_offline`].
//! The two are deliberately not merged — one has a database behind it and the
//! other has 320 KB of RAM — and
//! [offline-autonomy.md](../../../../docs/architecture/offline-autonomy.md) §9
//! states the cost of that plainly.

pub mod budget;
pub mod gate;
pub mod machine;
pub mod no_delivery;
pub mod types;

pub use gate::{LeakLockout, LeakResetRefused, safety_gate};
pub use machine::{absorption_until, delivery_evidence, evaluate, next_state};
pub use no_delivery::{DeliveryEvidence, no_delivery_detected};
pub use types::{
    EvaluationMode, IrrigationDecision, IrrigationInputs, LeakState, RequiredInput,
    RequiredInputState, TankState, WeightSample, is_auto_clearable, state_from_str, state_name,
};
