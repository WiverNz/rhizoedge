//! Local operator REST API.
pub mod bindings;
pub mod device_config;
pub mod devices;
pub mod health;
pub mod intents;
pub mod measurement_policies;
pub mod offline_policy;
pub mod overview;
pub mod plants;
pub mod presets;
pub mod profiles;
pub mod recommendation;
pub mod server;
pub mod support;
pub mod sync;
#[cfg(test)]
pub mod testsupport;
pub mod watering;

use crate::metrics::Metrics;
use std::sync::Arc;

/// Shared handler state.
#[derive(Clone)]
pub struct ApiState {
    /// Authoritative registry database.
    pub db: rhizo_storage::EdgeDb,
    /// Process metrics and MQTT lifecycle.
    pub metrics: Metrics,
    /// The edge clock. Injected rather than read directly, so a test can put a
    /// plant three hours in the past without waiting three hours (ADR-013).
    pub clock: Arc<dyn rhizo_domain::Clock>,
    /// The one path from a decision to the wire.
    ///
    /// Held by the API so `POST /water` runs the *same* gate and the *same*
    /// persist-before-publish order as the control loop. An HTTP handler with
    /// its own MQTT client would be a second actuation path, and SAFETY-003 and
    /// SAFETY-004 would hold only for one of them.
    pub commander: crate::control::command::Commander,
    /// Stable edge identity reported by the composite overview.
    pub edge_id: String,
    /// Logical seconds per real second, shared with the simulator in M8.
    pub time_scale: f64,
}
