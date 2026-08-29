//! Rhizo Edge reference device simulator — process entry point.
//!
//! Parses the CLI, installs logging with `device_id` on every event, runs the
//! device until a signal or `--duration`, and reports how it stopped.

#![forbid(unsafe_code)]

use std::process::ExitCode;

use clap::Parser;
use device_simulator::cli::Cli;
use device_simulator::runner;

fn main() -> ExitCode {
    // `clap` exits 2 with a usage message on a parse failure; only the
    // cross-flag checks are ours to report.
    let cli = Cli::parse();
    if let Err(e) = cli.validate() {
        eprintln!("error: {e}");
        return ExitCode::from(2);
    }

    let format = match cli.log_format.parse() {
        Ok(f) => f,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(2);
        }
    };
    if let Err(e) = rhizo_telemetry::init_tracing(format, &cli.log_level) {
        eprintln!("error: {e}");
        return ExitCode::from(2);
    }

    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: could not start the async runtime: {e}");
            return ExitCode::FAILURE;
        }
    };
    runtime.block_on(run(cli))
}

async fn run(cli: Cli) -> ExitCode {
    // Every event carries `device_id`: with several simulators running,
    // unlabelled logs are unusable (ADR-010 §Structured fields).
    let span = tracing::info_span!("device", device_id = %cli.device_id);
    let _entered = span.enter();

    tracing::info!(
        broker = %cli.broker,
        time_scale = cli.time_scale,
        telemetry_interval_seconds = cli.telemetry_interval,
        sensors = %cli.sensors,
        actuators = %cli.actuators,
        state_file = %cli.resolved_state_file().display(),
        control_bind = ?if cli.no_control_api { None } else { Some(cli.resolved_control_bind()) },
        noise = !cli.no_noise,
        seed = cli.seed,
        "device simulator starting"
    );
    for fault in &cli.faults {
        tracing::warn!(fault = %fault, "fault enabled at startup");
    }

    match runner::run(cli).await {
        Ok(signal) => {
            tracing::info!(
                ?signal,
                reason = signal.reason(),
                "device simulator stopped"
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            tracing::error!(error = %e, "device simulator could not start");
            ExitCode::FAILURE
        }
    }
}
