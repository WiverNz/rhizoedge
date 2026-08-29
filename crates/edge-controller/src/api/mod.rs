//! Local operator REST API.
pub mod bindings;
pub mod devices;
pub mod health;
pub mod measurement_policies;
pub mod offline_policy;
pub mod plants;
pub mod presets;
pub mod profiles;
pub mod recommendation;
pub mod server;
pub mod support;
#[cfg(test)]
pub mod testsupport;

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
}
