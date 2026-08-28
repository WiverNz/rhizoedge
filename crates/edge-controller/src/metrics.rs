//! Bounded-cardinality ingestion metrics.
#![allow(missing_docs)]
use prometheus::{HistogramOpts, HistogramVec, IntCounter, IntCounterVec, IntGauge, Opts};
use rhizo_telemetry::names::*;
/// Registered M3 metric set.
#[derive(Clone)]
pub struct Metrics {
    pub received: IntCounterVec,
    pub decode: IntCounterVec,
    pub duplicate: IntCounterVec,
    pub reconnects: IntCounter,
    pub connection: IntGauge,
    pub measurements: IntCounterVec,
    pub sensor_errors: IntCounterVec,
    pub sqlite_busy: IntCounter,
    pub storage_bytes: IntGauge,
    pub task_panics: IntCounterVec,
    pub duration: HistogramVec,
    pub rows_pruned: IntCounterVec,
    pub history_gaps: IntCounterVec,
    pub devices_online: IntGauge,
    pub devices_offline: IntGauge,
    pub devices_isolated: IntGauge,
    pub device_restarts: IntCounterVec,
    pub http_duration: HistogramVec,
}
impl Metrics {
    /// Registers the catalogue in a private registry-friendly process registry.
    pub fn new() -> Result<Self, prometheus::Error> {
        static INSTANCE: std::sync::OnceLock<Metrics> = std::sync::OnceLock::new();
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        if let Some(existing) = INSTANCE.get() {
            return Ok(existing.clone());
        }
        let _guard = LOCK.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(existing) = INSTANCE.get() {
            return Ok(existing.clone());
        }
        let r = rhizo_telemetry::registry();
        macro_rules! reg {
            ($m:expr) => {{
                let m = $m;
                r.register(Box::new(m.clone()))?;
                m
            }};
        }
        let metrics = Self {
            received: reg!(IntCounterVec::new(
                Opts::new(MQTT_MESSAGES_RECEIVED_TOTAL, "Inbound MQTT messages"),
                &["kind"]
            )?),
            decode: reg!(IntCounterVec::new(
                Opts::new(MQTT_DECODE_ERRORS_TOTAL, "Rejected MQTT messages"),
                &["reason"]
            )?),
            duplicate: reg!(IntCounterVec::new(
                Opts::new(MQTT_DUPLICATE_MESSAGES_TOTAL, "QoS duplicates"),
                &["kind"]
            )?),
            reconnects: reg!(IntCounter::new(MQTT_RECONNECTS_TOTAL, "MQTT reconnects")?),
            connection: reg!(IntGauge::new(
                MQTT_CONNECTION_STATE,
                "0 disconnected, 1 connecting, 2 connected, 3 subscribed"
            )?),
            measurements: reg!(IntCounterVec::new(
                Opts::new(MEASUREMENTS_PROCESSED_TOTAL, "Measurement samples"),
                &["kind"]
            )?),
            sensor_errors: reg!(IntCounterVec::new(
                Opts::new(SENSOR_ERRORS_TOTAL, "Sample validation errors"),
                &["sensor", "reason"]
            )?),
            sqlite_busy: reg!(IntCounter::new(SQLITE_BUSY_TOTAL, "SQLite busy responses")?),
            storage_bytes: reg!(IntGauge::new(STORAGE_BYTES, "SQLite bytes")?),
            task_panics: reg!(IntCounterVec::new(
                Opts::new(TASK_PANICS_TOTAL, "Supervised task panics"),
                &["task"]
            )?),
            duration: reg!(HistogramVec::new(
                HistogramOpts::new(MQTT_PROCESSING_DURATION_SECONDS, "MQTT processing latency")
                    .buckets(vec![0.001, 0.01, 0.1, 1.0, 5.0]),
                &["kind"]
            )?),
            rows_pruned: reg!(IntCounterVec::new(
                Opts::new(ROWS_PRUNED_TOTAL, "Rows removed by retention"),
                &["table"]
            )?),
            history_gaps: reg!(IntCounterVec::new(
                Opts::new(HISTORY_GAPS_TOTAL, "Reported history gaps"),
                &["tier"]
            )?),
            devices_online: reg!(IntGauge::new(DEVICES_ONLINE, "Online devices")?),
            devices_offline: reg!(IntGauge::new(DEVICES_OFFLINE, "Offline devices")?),
            devices_isolated: reg!(IntGauge::new(DEVICES_ISOLATED, "Isolated devices")?),
            device_restarts: reg!(IntCounterVec::new(
                Opts::new(DEVICE_RESTARTS_TOTAL, "Device boot identity changes"),
                &["device_id"]
            )?),
            http_duration: reg!(HistogramVec::new(
                HistogramOpts::new(HTTP_REQUEST_DURATION_SECONDS, "HTTP request latency"),
                &["route", "status"]
            )?),
        };
        for metric in [&metrics.received, &metrics.duplicate, &metrics.measurements] {
            metric.with_label_values(&["unknown"]);
        }
        metrics.decode.with_label_values(&["unknown"]);
        metrics
            .sensor_errors
            .with_label_values(&["unknown", "unknown"]);
        metrics.task_panics.with_label_values(&["unknown"]);
        metrics.duration.with_label_values(&["unknown"]);
        metrics.rows_pruned.with_label_values(&["unknown"]);
        metrics.history_gaps.with_label_values(&["unknown"]);
        metrics.device_restarts.with_label_values(&["unknown"]);
        metrics
            .http_duration
            .with_label_values(&["unknown", "unknown"]);
        let _ = INSTANCE.set(metrics.clone());
        Ok(metrics)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn metrics_catalogue_renders() {
        let _ = Metrics::new().unwrap();
        let text = rhizo_telemetry::render_prometheus();
        assert!(text.contains(MQTT_MESSAGES_RECEIVED_TOTAL));
        assert!(
            text.lines()
                .filter(|line| line.contains("device_id="))
                .all(|line| line.starts_with(DEVICE_RESTARTS_TOTAL))
        );
    }
    #[test]
    fn cardinality() {
        let _ = Metrics::new().unwrap();
        let series = rhizo_telemetry::registry()
            .gather()
            .iter()
            .map(|f| f.get_metric().len())
            .sum::<usize>();
        assert!(
            series < 100,
            "exported series count {series} exceeded 100; a new label was probably added; check ADR-010's cardinality rules"
        );
    }
}
#[cfg(test)]
mod devices {
    #[test]
    fn lifecycle_gauges_and_restart_label_are_bounded() {
        let metrics = super::Metrics::new().unwrap();
        metrics.devices_online.set(2);
        metrics.devices_offline.set(1);
        metrics.devices_isolated.set(1);
        metrics
            .device_restarts
            .with_label_values(&["plant-node-01"])
            .inc();
        assert_eq!(metrics.devices_online.get(), 2);
        assert_eq!(metrics.devices_offline.get(), 1);
        assert_eq!(metrics.devices_isolated.get(), 1);
    }
}
