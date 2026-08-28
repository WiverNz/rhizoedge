//! Local operator REST API.
pub mod devices;
pub mod health;
pub mod server;

use crate::metrics::Metrics;

/// Shared handler state.
#[derive(Clone)]
pub struct ApiState {
    /// Authoritative registry database.
    pub db: rhizo_storage::EdgeDb,
    /// Process metrics and MQTT lifecycle.
    pub metrics: Metrics,
}
