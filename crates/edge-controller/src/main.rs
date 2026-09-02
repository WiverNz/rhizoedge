//! Rhizo Edge ingestion process.
#![forbid(unsafe_code)]
use clap::Parser;
use edge_controller::{
    config::{self, CliOverrides},
    metrics::Metrics,
    mqtt::ingress,
    state::cache::LatestSampleCache,
    supervisor::Supervisor,
};
use std::{path::PathBuf, process::ExitCode, sync::Arc, time::Duration};

#[derive(Debug, Parser)]
#[command(name = "edge-controller", version)]
struct Cli {
    #[arg(long, value_name = "PATH")]
    config: Option<PathBuf>,
    #[arg(long, value_name = "DIRECTIVE")]
    log_level: Option<String>,
}

#[tokio::main]
async fn main() -> ExitCode {
    match start().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
async fn start() -> Result<(), String> {
    let cli = Cli::parse();
    let loaded = config::load(&CliOverrides {
        config_path: cli.config,
        log_level: cli.log_level,
    })
    .map_err(|e| format!("{e} (key {})", e.key()))?;
    let format = loaded
        .config
        .log
        .parsed_format()
        .map_err(|e| e.to_string())?;
    rhizo_telemetry::init_tracing(format, &loaded.config.log.level).map_err(|e| e.to_string())?;
    loaded.emit_warnings();
    let c = loaded.config;
    let metrics = Metrics::new().map_err(|e| e.to_string())?;
    let db = rhizo_storage::EdgeDb::connect(&c.storage.path)
        .await
        .map_err(|e| e.to_string())?;
    db.migrate().await.map_err(|e| e.to_string())?;
    rhizo_storage::repo::outbox::configure(&db, c.cloud.enabled, c.cloud.outbox_max_rows)
        .await
        .map_err(|e| e.to_string())?;
    let devices = rhizo_storage::repo::query::device_count(&db)
        .await
        .map_err(|e| e.to_string())?;
    let cache = LatestSampleCache::restore(&db)
        .await
        .map_err(|e| e.to_string())?;
    tracing::info!(
        devices,
        latest_samples = cache.len(),
        "restored persistent edge state"
    );
    let opts = ingress::options(
        &c.mqtt.broker_url,
        &c.mqtt.client_id,
        &c.mqtt.username,
        c.mqtt.password.expose(),
    )?;
    let (client, eventloop) = ingress::connect(opts, 64);
    let (tx, rx) = tokio::sync::mpsc::channel(128);
    #[allow(
        clippy::disallowed_methods,
        reason = "the binary anchors the host-clock adapter once at startup"
    )]
    let host_utc = chrono::Utc::now();
    // Accelerated time can run far ahead of the host. Restarting from host time
    // would move the logical clock backwards, causing devices to reject the
    // next `edge.time` and every safety-dated command after it. Anchor beyond
    // the last durable ingress receipt instead; at production scale this is
    // simply the host clock, while E2E restarts remain monotonic.
    let durable_high_water: Option<i64> =
        sqlx::query_scalar("SELECT max(received_at) FROM processed_messages")
            .fetch_one(db.pool())
            .await
            .map_err(|e| e.to_string())?;
    let base_utc = durable_high_water
        .and_then(|millis| chrono::DateTime::from_timestamp_millis(millis.saturating_add(1)))
        .map_or(host_utc, |durable| durable.max(host_utc));
    let clock: Arc<dyn rhizo_domain::Clock> = Arc::new(
        edge_controller::clock::AcceleratedClock::new(base_utc, c.time_scale),
    );
    tracing::info!(
        time_scale = c.time_scale,
        variable = "RHIZO_TIME_SCALE",
        "edge logical clock configured"
    );
    // The M6 commander. Unlike M5's evaluation pass it holds a transport, so the
    // control plane can move water — and every path from a decision to the wire
    // goes through it, which persists before it publishes.
    let commander = edge_controller::control::command::Commander::new(
        db.clone(),
        clock.clone(),
        Arc::new(edge_controller::control::transport::MqttTransport::new(
            client.clone(),
        )),
        metrics.clone(),
    );
    // SAFETY-010's recovery procedure, before anything can issue a new command:
    // expire what has timed out, await what has not, and re-publish **nothing**.
    let recovery = commander.reconcile().await.map_err(|e| e.to_string())?;
    edge_controller::control::intents::reconcile(&commander, clock.now())
        .await
        .map_err(|e| e.to_string())?;
    tracing::info!(
        expired = recovery.expired,
        awaiting = recovery.awaiting,
        "reconciled the command ledger at startup"
    );
    let mut supervisor = Supervisor::new(metrics.clone(), Duration::from_secs(10));
    if c.cloud.enabled {
        let cloud_client = rhizo_cloud_client::CloudClient::new(
            &c.cloud.base_url,
            c.edge_id.clone(),
            Duration::from_secs(c.cloud.request_timeout_seconds),
        )
        .map_err(|e| e.to_string())?;
        supervisor.spawn(
            "cloud_outbox",
            edge_controller::cloud::drain::run(
                db.clone(),
                cloud_client,
                clock.clone(),
                metrics.clone(),
                supervisor.shutdown_receiver(),
            ),
        );
    }
    supervisor.spawn(
        "mqtt_ingress",
        ingress::run(
            client.clone(),
            eventloop,
            tx,
            supervisor.shutdown_receiver(),
            metrics.clone(),
        ),
    );
    supervisor.spawn(
        "pipeline",
        edge_controller::pipeline::run(
            rx,
            db.clone(),
            clock.clone(),
            client.clone(),
            Some(commander.clone()),
            cache,
            supervisor.shutdown_receiver(),
            metrics.clone(),
        ),
    );
    supervisor.spawn(
        "retention",
        edge_controller::retention::run(
            db.clone(),
            clock.clone(),
            supervisor.shutdown_receiver(),
            metrics.clone(),
        ),
    );
    supervisor.spawn(
        "device_health",
        edge_controller::device::health::run(
            db.clone(),
            clock.clone(),
            client.clone(),
            metrics.clone(),
            supervisor.shutdown_receiver(),
        ),
    );
    supervisor.spawn(
        "plant_control",
        edge_controller::control::tick::run_control(
            commander.clone(),
            clock.clone(),
            metrics.clone(),
            std::time::Duration::from_secs_f64(
                (c.control.tick_interval_seconds as f64 / c.time_scale).max(0.01),
            ),
            c.time_scale,
            supervisor.shutdown_receiver(),
        ),
    );
    // One sensible default profile, so a first-run system is usable without the
    // operator inventing numbers. Inserted only when the id is free, so an
    // operator who edits it keeps their edit.
    if edge_controller::api::profiles::seed_default(&db, clock.now().timestamp_millis())
        .await
        .map_err(|e| e.to_string())?
    {
        tracing::info!(
            profile = edge_controller::api::profiles::DEFAULT_PROFILE_ID,
            "seeded the default plant profile"
        );
    }
    let bind = c.api.bind;
    let cors_allowed_origins = c.api.cors_allowed_origins;
    let api_state = edge_controller::api::ApiState {
        db: db.clone(),
        metrics: metrics.clone(),
        clock: clock.clone(),
        commander: commander.clone(),
        edge_id: c.edge_id.clone(),
        time_scale: c.time_scale,
    };
    let mut shutdown = supervisor.shutdown_receiver();
    supervisor.spawn("api", async move {
        let app = edge_controller::api::server::router(api_state, cors_allowed_origins);
        let listener = tokio::net::TcpListener::bind(bind)
            .await
            .map_err(|e| e.to_string())?;
        axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                while shutdown.changed().await.is_ok() {
                    if *shutdown.borrow() {
                        break;
                    }
                }
            })
            .await
            .map_err(|e| e.to_string())
    });
    let result = supervisor.run().await;
    db.close().await;
    result
}
