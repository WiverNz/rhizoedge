//! Metric name constants.
//!
//! Every metric name in the system is a `const` here, so a typo at a call site
//! is a compile error rather than a silently missing series that nobody
//! notices until they need it.
//!
//! # The catalogue is deliberately empty in M0
//!
//! [ADR-010](../../../../docs/adr/010-observability-strategy.md) is explicit
//! that metrics are added when a real question cannot be answered without
//! them: a large catalogue nobody reads costs cardinality and maintenance and
//! is worse than a small one that answers the operational questions. M0
//! delivers no behaviour to measure, so it declares no metric.
//!
//! ADR-010 §Metrics carries the normative catalogue and the milestone each
//! entry arrives in — ingestion in M3, control and lockouts in M6, cloud sync
//! in M7. A name added here should match that document.
//!
//! # Naming
//!
//! Prometheus conventions, which are not merely cosmetic — `promtool` and most
//! dashboards assume them:
//!
//! - counters end in `_total`
//! - durations end in `_seconds`, byte counts in `_bytes`
//! - gauges are named for the quantity, not the act of measuring it
//!   (`devices_online`, not `devices_online_count`)
/// MQTT messages received.
pub const MQTT_MESSAGES_RECEIVED_TOTAL: &str = "mqtt_messages_received_total";
/// MQTT decoding failures.
pub const MQTT_DECODE_ERRORS_TOTAL: &str = "mqtt_decode_errors_total";
/// MQTT duplicate messages.
pub const MQTT_DUPLICATE_MESSAGES_TOTAL: &str = "mqtt_duplicate_messages_total";
/// Successful reconnections.
pub const MQTT_RECONNECTS_TOTAL: &str = "mqtt_reconnects_total";
/// MQTT lifecycle state gauge.
pub const MQTT_CONNECTION_STATE: &str = "mqtt_connection_state";
/// Measurement samples processed.
pub const MEASUREMENTS_PROCESSED_TOTAL: &str = "measurements_processed_total";
/// Sensor validation failures.
pub const SENSOR_ERRORS_TOTAL: &str = "sensor_errors_total";
/// SQLite busy responses.
pub const SQLITE_BUSY_TOTAL: &str = "sqlite_busy_total";
/// Database storage bytes.
pub const STORAGE_BYTES: &str = "storage_bytes";
/// Supervised task panics.
pub const TASK_PANICS_TOTAL: &str = "task_panics_total";
/// MQTT pipeline latency.
pub const MQTT_PROCESSING_DURATION_SECONDS: &str = "mqtt_processing_duration_seconds";
/// Retention rows pruned.
pub const ROWS_PRUNED_TOTAL: &str = "rows_pruned_total";
/// Durable device-reported gaps.
pub const HISTORY_GAPS_TOTAL: &str = "history_gaps_total";
/// Currently online devices.
pub const DEVICES_ONLINE: &str = "devices_online";
/// Currently offline devices.
pub const DEVICES_OFFLINE: &str = "devices_offline";
/// Currently isolated devices.
pub const DEVICES_ISOLATED: &str = "devices_isolated";
/// Device restarts, deliberately fleet-cardinality labelled.
pub const DEVICE_RESTARTS_TOTAL: &str = "device_restarts_total";
/// HTTP request latency.
pub const HTTP_REQUEST_DURATION_SECONDS: &str = "http_request_duration_seconds";
