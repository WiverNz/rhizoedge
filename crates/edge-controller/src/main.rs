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
        reason = "the binary is the host-clock adapter injected into domain Clock"
    )]
    fn host_now() -> chrono::DateTime<chrono::Utc> {
        chrono::Utc::now()
    }
    let clock: Arc<dyn rhizo_domain::Clock> = Arc::new(rhizo_domain::SystemClock::new(host_now));
    let mut supervisor = Supervisor::new(metrics.clone(), Duration::from_secs(10));
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
    // The plant evaluation loop (M5-012). It takes no MQTT client, which is the
    // strongest available form of "M5 issues no commands": the loop could not
    // publish one if it wanted to.
    supervisor.spawn(
        "plant_control",
        edge_controller::control::tick::run(
            db.clone(),
            clock.clone(),
            metrics.clone(),
            std::time::Duration::from_secs(c.control.tick_interval_seconds),
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
