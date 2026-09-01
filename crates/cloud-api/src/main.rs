//! Rhizo Edge cloud replica API.
#![forbid(unsafe_code)]
use clap::{Parser, Subcommand};
use std::{net::SocketAddr, process::ExitCode};

#[derive(Parser)]
#[command(name = "cloud-api", version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
    #[arg(
        long,
        env = "RHIZO_CLOUD__DATABASE_URL",
        default_value = "postgres://rhizo:rhizo@127.0.0.1:5432/rhizo"
    )]
    database_url: String,
    #[arg(long, env = "RHIZO_CLOUD__BIND", default_value = "0.0.0.0:8081")]
    bind: SocketAddr,
}
#[derive(Subcommand)]
enum Command {
    Reproject {
        #[arg(long)]
        edge_id: String,
    },
}

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}
async fn run() -> anyhow::Result<()> {
    let cli = Cli::parse();
    rhizo_telemetry::init_tracing(rhizo_telemetry::LogFormat::Json, "info")?;
    let pool = cloud_api::connect(&cli.database_url).await?;
    if let Some(Command::Reproject { edge_id }) = cli.command {
        let count = cloud_api::reproject(&pool, &edge_id)
            .await
            .map_err(anyhow::Error::msg)?;
        println!("reprojected {count} ledger events for {edge_id}");
        let consistent = cloud_api::projections_consistent(&pool, &edge_id).await?;
        println!(
            "projection consistency: {}",
            if consistent { "ok" } else { "drift detected" }
        );
        return Ok(());
    }
    let metrics = cloud_api::CloudMetrics::new()?;
    let app = cloud_api::router(cloud_api::AppState {
        pool: pool.clone(),
        metrics,
    });
    let listener = tokio::net::TcpListener::bind(cli.bind).await?;
    tracing::info!(bind=%cli.bind,"cloud API ready");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    tracing::info!("graceful shutdown complete");
    pool.close().await;
    Ok(())
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate());
        match terminate {
            Ok(mut terminate) => {
                tokio::select! {
                    _ = terminate.recv() => {}
                    result = tokio::signal::ctrl_c() => {
                        if let Err(error) = result {
                            tracing::error!(%error, "failed to listen for shutdown signal");
                        }
                    }
                }
            }
            Err(error) => {
                tracing::error!(%error, "failed to listen for SIGTERM");
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }
    #[cfg(not(unix))]
    if let Err(error) = tokio::signal::ctrl_c().await {
        tracing::error!(%error, "failed to listen for shutdown signal");
    }
    tracing::info!("graceful shutdown requested");
}
