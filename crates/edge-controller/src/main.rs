//! Rhizo Edge control plane entry point.
//!
//! M0 delivers the startup sequence and nothing else: parse flags, load and
//! validate configuration, install the tracing subscriber, report. The control
//! loop, the broker connection, and the API arrive in M3–M6.

#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use edge_controller::config::{self, CliOverrides, ConfigError};
use rhizo_telemetry::{Classify, FailureKind};

/// The Rhizo Edge control plane.
///
/// Deliberately few flags. ADR-011 §L2 allows "a small set"; a tuning value
/// belongs in `edge.toml` or an environment variable, where it can carry a
/// comment explaining why it is what it is.
#[derive(Debug, Parser)]
#[command(name = "edge-controller", version, about, long_about = None)]
struct Cli {
    /// Path to `edge.toml`.
    ///
    /// When given, the file must exist: naming a file that is not there is an
    /// error rather than a silent fall back to defaults.
    #[arg(long, value_name = "PATH")]
    config: Option<PathBuf>,

    /// Override `log.level` with a `RUST_LOG`-style directive.
    #[arg(long, value_name = "DIRECTIVE")]
    log_level: Option<String>,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let overrides = CliOverrides {
        config_path: cli.config,
        log_level: cli.log_level,
    };

    let loaded = match config::load(&overrides) {
        Ok(loaded) => loaded,
        Err(e) => return fail(&e),
    };

    // Only now can logging be configured — its format is itself configuration.
    let format = match loaded.config.log.parsed_format() {
        Ok(f) => f,
        Err(e) => return fail(&e),
    };
    if let Err(e) = rhizo_telemetry::init_tracing(format, &loaded.config.log.level) {
        eprintln!("error: {e}");
        return ExitCode::FAILURE;
    }

    // Warnings were collected during loading, before a subscriber existed.
    loaded.emit_warnings();

    let c = &loaded.config;
    tracing::info!(
        edge_id = %c.edge_id,
        config_file = %loaded
            .config_file
            .as_ref()
            .map_or_else(|| String::from("(none)"), |p| p.display().to_string()),
        broker_url = %c.mqtt.broker_url,
        api_bind = %c.api.bind,
        cloud_enabled = c.cloud.enabled,
        tick_interval_seconds = c.control.tick_interval_seconds,
        "configuration loaded"
    );

    // Safe to print in full: every secret-shaped field is a `Secret`, whose
    // `Debug` renders `[redacted]`. At DEBUG rather than INFO because it is
    // diagnostic detail, not a change to the world (ADR-010 §Levels).
    tracing::debug!(config = ?c, "resolved configuration");

    tracing::info!(
        "M0 delivers configuration, telemetry, and the engineering baseline only; \
         the control loop is a later milestone"
    );

    ExitCode::SUCCESS
}

/// Reports a fatal configuration error and yields a non-zero exit code.
///
/// Printed to stderr rather than logged: the log subscriber's own settings are
/// part of what failed to load, so there may not be one.
fn fail(e: &ConfigError) -> ExitCode {
    eprintln!("error: {e}");
    eprintln!("  configuration key: {}", e.key());
    debug_assert_eq!(
        e.classify(),
        FailureKind::Fatal,
        "configuration errors are Fatal by ADR-014"
    );
    ExitCode::FAILURE
}
