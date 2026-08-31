//! Bounded-cardinality ingestion metrics.
#![allow(missing_docs)]
use prometheus::{
    HistogramOpts, HistogramVec, IntCounter, IntCounterVec, IntGauge, IntGaugeVec, Opts,
};
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
    pub devices_sleeping: IntGauge,
    pub device_wake_missed: IntCounter,
    pub device_restarts: IntCounterVec,
    pub http_duration: HistogramVec,
    pub plants_total: IntGauge,
    pub plant_state: IntGaugeVec,
    pub recommendations: IntCounterVec,
    pub manual_watering_detected: IntCounter,
    pub threshold_crossings: IntCounterVec,
    // ------------------------------------------------------------------ M6
    pub watering_commands: IntCounterVec,
    pub watering_delivered_ml: prometheus::CounterVec,
    pub watering_failures: IntCounterVec,
    pub irrigation_transitions: IntCounterVec,
    pub plants_locked_out: IntGauge,
    pub lockouts: IntCounterVec,
    pub control_tick_duration: prometheus::Histogram,
    pub command_intents_pending: IntGauge,
    pub command_intents_expired: IntCounter,
    pub clock_steps: IntCounterVec,
    pub pending_cloud_events: IntGauge,
    pub cloud_sync_attempts: IntCounterVec,
    pub cloud_sync_duration: prometheus::Histogram,
    pub cloud_events_quarantined: IntCounter,
    pub cloud_events_dropped: IntCounter,
    pub cloud_last_success: IntGauge,
    pub cloud_batch_size: IntGauge,
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
            devices_sleeping: reg!(IntGauge::new(
                DEVICES_SLEEPING,
                "Expected sleeping devices"
            )?),
            device_wake_missed: reg!(IntCounter::new(
                DEVICE_WAKE_MISSED_TOTAL,
                "Expected wake windows missed"
            )?),
            device_restarts: reg!(IntCounterVec::new(
                Opts::new(DEVICE_RESTARTS_TOTAL, "Device boot identity changes"),
                &["device_id"]
            )?),
            http_duration: reg!(HistogramVec::new(
                HistogramOpts::new(HTTP_REQUEST_DURATION_SECONDS, "HTTP request latency"),
                &["route", "status"]
            )?),
            plants_total: reg!(IntGauge::new(PLANTS_TOTAL, "Configured plants")?),
            // Labelled by state, which is a closed set of six, not by plant id.
            plant_state: reg!(IntGaugeVec::new(
                Opts::new(PLANT_STATE, "Plants in each operator-facing state"),
                &["state"]
            )?),
            recommendations: reg!(IntCounterVec::new(
                Opts::new(RECOMMENDATIONS_TOTAL, "Recommendations recorded"),
                &["decision"]
            )?),
            manual_watering_detected: reg!(IntCounter::new(
                MANUAL_WATERING_DETECTED_TOTAL,
                "Waterings the system did not perform"
            )?),
            // Kind is bounded by the contract's measurement kinds and severity
            // by three values; neither is fleet-sized (ADR-010).
            threshold_crossings: reg!(IntCounterVec::new(
                Opts::new(THRESHOLD_CROSSINGS_TOTAL, "Threshold crossings"),
                &["kind", "severity"]
            )?),
            // Mode is one of three and outcome one of six; neither is
            // fleet-sized, and no label carries a plant or device id (ADR-010).
            watering_commands: reg!(IntCounterVec::new(
                Opts::new(WATERING_COMMANDS_TOTAL, "Water commands by outcome"),
                &["mode", "outcome"]
            )?),
            watering_delivered_ml: reg!(prometheus::CounterVec::new(
                Opts::new(WATERING_DELIVERED_ML_TOTAL, "Millilitres delivered"),
                &["mode"]
            )?),
            watering_failures: reg!(IntCounterVec::new(
                Opts::new(WATERING_FAILURES_TOTAL, "Watering failures by reason"),
                &["reason"]
            )?),
            irrigation_transitions: reg!(IntCounterVec::new(
                Opts::new(
                    IRRIGATION_STATE_TRANSITIONS_TOTAL,
                    "Irrigation state transitions"
                ),
                &["from", "to"]
            )?),
            plants_locked_out: reg!(IntGauge::new(
                PLANTS_LOCKED_OUT,
                "Plants currently locked out"
            )?),
            lockouts: reg!(IntCounterVec::new(
                Opts::new(LOCKOUTS_TOTAL, "Lockouts raised by reason"),
                &["reason"]
            )?),
            control_tick_duration: reg!(prometheus::Histogram::with_opts(
                HistogramOpts::new(CONTROL_TICK_DURATION_SECONDS, "Control pass duration")
                    .buckets(vec![0.001, 0.01, 0.1, 1.0, 5.0, 30.0])
            )?),
            command_intents_pending: reg!(IntGauge::new(
                COMMAND_INTENTS_PENDING,
                "Doses held for a sleeping device"
            )?),
            command_intents_expired: reg!(IntCounter::new(
                COMMAND_INTENTS_EXPIRED_TOTAL,
                "Intents that expired before a wake"
            )?),
            clock_steps: reg!(IntCounterVec::new(
                Opts::new(CLOCK_STEPS_TOTAL, "Edge wall-clock steps"),
                &["direction"]
            )?),
            pending_cloud_events: reg!(IntGauge::new(
                "pending_cloud_events",
                "Pending cloud history events"
            )?),
            cloud_sync_attempts: reg!(IntCounterVec::new(
                Opts::new("cloud_sync_attempts_total", "Cloud sync attempts"),
                &["outcome"]
            )?),
            cloud_sync_duration: reg!(prometheus::Histogram::with_opts(HistogramOpts::new(
                "cloud_sync_duration_seconds",
                "Cloud request duration"
            ))?),
            cloud_events_quarantined: reg!(IntCounter::new(
                "cloud_events_quarantined_total",
                "Cloud-rejected events"
            )?),
            cloud_events_dropped: reg!(IntCounter::new(
                "cloud_events_dropped_total",
                "Low-tier events pruned at cap"
            )?),
            cloud_last_success: reg!(IntGauge::new(
                "cloud_last_success_timestamp_seconds",
                "Last successful cloud sync"
            )?),
            cloud_batch_size: reg!(IntGauge::new(
                "cloud_sync_batch_size",
                "Adaptive cloud batch size"
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
        for state in [
            "healthy",
            "drying",
            "water_recommended",
            "sensor_fault",
            "watering_locked",
            "other",
        ] {
            metrics.plant_state.with_label_values(&[state]);
        }
        for decision in ["water", "no_water", "blocked"] {
            metrics.recommendations.with_label_values(&[decision]);
        }
        metrics
            .threshold_crossings
            .with_label_values(&["unknown", "unknown"]);
        // Pre-create the closed label sets so a scrape before the first dose
        // still shows the series at zero, which is what an operator needs from
        // a dashboard on a quiet system.
        for mode in ["automatic", "manual", "recommended"] {
            metrics.watering_delivered_ml.with_label_values(&[mode]);
            for outcome in [
                "completed",
                "rejected",
                "interrupted",
                "failed",
                "expired",
                "publish_failed",
            ] {
                metrics
                    .watering_commands
                    .with_label_values(&[mode, outcome]);
            }
        }
        metrics.watering_failures.with_label_values(&["unknown"]);
        metrics
            .irrigation_transitions
            .with_label_values(&["unknown", "unknown"]);
        metrics.lockouts.with_label_values(&["unknown"]);
        for direction in ["forward", "backward"] {
            metrics.clock_steps.with_label_values(&[direction]);
        }
        for outcome in ["success", "failure"] {
            metrics.cloud_sync_attempts.with_label_values(&[outcome]);
        }
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
        // The ceiling is the sum of the **closed** label sets, not a round
        // number. M6's largest is `irrigation_state_transitions{from,to}`: seven
        // states crossed with seven is forty-nine series in the worst case, and
        // it is bounded because the state vocabulary is. Add eighteen command
        // outcomes (three modes x six), twelve lockout reasons, nine refusal
        // reasons, and the M3-M5 catalogue, and the honest headroom is about two
        // hundred.
        //
        // What this actually guards against is a *fleet-sized* label — a plant
        // or device id — which would produce thousands rather than tens. The
        // companion assertion below is the one that names that failure directly.
        assert!(
            series < 220,
            "exported series count {series} exceeded 220; a new label was probably added; check ADR-010's cardinality rules"
        );
        assert!(
            series > 60,
            "exported series count {series} is implausibly low; the catalogue did not register"
        );
        let text = rhizo_telemetry::render_prometheus();
        assert!(
            text.lines()
                .filter(|line| line.contains("plant_id=") || line.contains("device_id="))
                .all(|line| line.starts_with(DEVICE_RESTARTS_TOTAL)),
            "no metric but the device-restart counter may carry an entity id"
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
        metrics.devices_sleeping.set(1);
        metrics.device_wake_missed.inc();
        metrics
            .device_restarts
            .with_label_values(&["plant-node-01"])
            .inc();
        assert_eq!(metrics.devices_online.get(), 2);
        assert_eq!(metrics.devices_offline.get(), 1);
        assert_eq!(metrics.devices_isolated.get(), 1);
        assert_eq!(metrics.devices_sleeping.get(), 1);
    }
}

/// The control metric set (M6-014), named so `cargo test -p edge-controller
/// metrics::control` selects it.
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod control {
    use crate::api::testsupport::TestApi;
    use rhizo_telemetry::names::*;

    /// Every metric ADR-010's catalogue lists for M6 is exported.
    #[test]
    fn the_whole_control_set_is_exported() {
        let _ = super::Metrics::new().unwrap();
        let text = rhizo_telemetry::render_prometheus();
        for name in [
            WATERING_COMMANDS_TOTAL,
            WATERING_DELIVERED_ML_TOTAL,
            WATERING_FAILURES_TOTAL,
            IRRIGATION_STATE_TRANSITIONS_TOTAL,
            PLANTS_LOCKED_OUT,
            LOCKOUTS_TOTAL,
            CONTROL_TICK_DURATION_SECONDS,
            COMMAND_INTENTS_PENDING,
            COMMAND_INTENTS_EXPIRED_TOTAL,
            CLOCK_STEPS_TOTAL,
        ] {
            assert!(text.contains(name), "{name} is missing from /metrics");
        }
    }

    /// No control metric is labelled by a plant or device id: those are
    /// fleet-sized and the cardinality guard exists because of them (ADR-010).
    #[test]
    fn no_control_metric_is_labelled_by_an_entity_id() {
        let _ = super::Metrics::new().unwrap();
        let text = rhizo_telemetry::render_prometheus();
        for line in text.lines().filter(|line| {
            line.starts_with(WATERING_COMMANDS_TOTAL)
                || line.starts_with(LOCKOUTS_TOTAL)
                || line.starts_with(IRRIGATION_STATE_TRANSITIONS_TOTAL)
        }) {
            assert!(!line.contains("plant_id="), "{line}");
            assert!(!line.contains("device_id="), "{line}");
        }
    }

    /// A completed dose counts its outcome, its volume, and its transitions —
    /// and every transition is persisted as a plant event, which is what makes
    /// "what did the system think, and when" reconstructable months later.
    #[tokio::test]
    async fn a_dose_counts_its_outcome_and_persists_its_transitions() {
        let api = TestApi::start().await;
        api.waterable("monstera-01").await;
        api.device_connected().await;
        let before = api
            .state
            .metrics
            .watering_commands
            .with_label_values(&["manual", "completed"])
            .get();

        let (_, body) = api
            .json(
                "POST",
                "/api/v1/plants/monstera-01/water",
                serde_json::json!({ "ml": 40.0 }),
            )
            .await;
        let command_id = body["command_id"].as_str().unwrap();
        api.commander
            .apply_result(&rhizo_mqtt_contract::payload::CommandResult {
                command_id: rhizo_mqtt_contract::CommandId::from_uuid(command_id.parse().unwrap()),
                status: rhizo_mqtt_contract::payload::CommandStatus::Completed,
                requested_ml: 40.0,
                delivered_ml: Some(40.0),
                duration_ms: Some(4_000),
                clamped: false,
                reason: None,
                delivered_today_ml: 40.0,
                origin: rhizo_mqtt_contract::payload::CommandOrigin::EdgeCommand,
                detail: None,
            })
            .await
            .unwrap();

        // Strictly greater, for the same reason as the histogram below: the
        // registry is a process-wide singleton and another test's completed
        // dose lands in the same counter.
        assert!(
            api.state
                .metrics
                .watering_commands
                .with_label_values(&["manual", "completed"])
                .get()
                > before,
            "a completed dose is counted under its mode and outcome"
        );
        assert!(
            api.state
                .metrics
                .watering_delivered_ml
                .with_label_values(&["manual"])
                .get()
                >= 40.0
        );

        let events = rhizo_storage::repo::plant::plant_events(&api.db, "monstera-01", 100)
            .await
            .unwrap();
        assert!(
            events
                .iter()
                .any(|(kind, ..)| kind == "irrigation_state_changed"),
            "every irrigation transition is persisted"
        );
    }

    /// `plants_locked_out` reflects actual lockouts, which is the series that
    /// answers "is anything stuck?".
    #[tokio::test]
    async fn the_lockout_gauge_reflects_actual_lockouts() {
        let api = TestApi::start().await;
        api.waterable("monstera-01").await;
        api.device_connected().await;
        crate::control::tick::irrigation_pass(&api.commander, &api.state.metrics, api.clock.now())
            .await
            .unwrap();
        assert_eq!(api.state.metrics.plants_locked_out.get(), 0);

        api.sample_bool(api.clock.now(), "leak-0", "tray", "leak_state", true)
            .await;
        crate::control::tick::irrigation_pass(&api.commander, &api.state.metrics, api.clock.now())
            .await
            .unwrap();
        assert_eq!(api.state.metrics.plants_locked_out.get(), 1);
        assert!(
            api.state
                .metrics
                .lockouts
                .with_label_values(&["leak"])
                .get()
                >= 1
        );

        // The condition clears, and so does the gauge.
        api.clock.advance(chrono::Duration::minutes(1));
        api.sample_bool(api.clock.now(), "leak-0", "tray", "leak_state", false)
            .await;
        api.sample(api.clock.now(), 20.0).await;
        api.sample_from(
            api.clock.now(),
            "tank-0",
            "reservoir",
            "tank_level",
            "percent",
            70.0,
        )
        .await;
        // A leak is explicit-clear, so it does **not** lift on its own.
        crate::control::tick::irrigation_pass(&api.commander, &api.state.metrics, api.clock.now())
            .await
            .unwrap();
        assert_eq!(
            api.state.metrics.plants_locked_out.get(),
            1,
            "a leak that dried out is not a leak that was fixed"
        );
    }

    /// The control pass is timed, which is the signal that the single-loop
    /// design needs revisiting at scale (M13).
    #[tokio::test]
    async fn the_control_pass_is_timed() {
        let api = TestApi::start().await;
        api.waterable("monstera-01").await;
        api.device_connected().await;
        let before = api.state.metrics.control_tick_duration.get_sample_count();
        crate::control::tick::irrigation_pass(&api.commander, &api.state.metrics, api.clock.now())
            .await
            .unwrap();
        // Strictly greater, not `before + 1`. The registry is a process-wide
        // singleton, so any other test running concurrently observes the same
        // histogram — asserting an exact delta would make this test fail on
        // timing rather than on behaviour.
        assert!(
            api.state.metrics.control_tick_duration.get_sample_count() > before,
            "the control pass must be timed"
        );
    }
}
