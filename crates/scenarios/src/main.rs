//! Command-line entry point for the M8 assembled-system scenario suite.
#![forbid(unsafe_code)]

use clap::Parser;
use rhizo_scenarios::{harness::Harness, scenarios};
use std::process::ExitCode;

#[derive(Debug, Parser)]
#[command(name = "scenario-runner", version)]
struct Cli {
    /// Run only these named scenarios. May be supplied more than once.
    #[arg(long, action = clap::ArgAction::Append)]
    scenario: Vec<String>,
    /// Print the deterministic scenario catalogue and exit.
    #[arg(long)]
    list: bool,
    /// Fixed deterministic seed.
    #[arg(long, env = "RHIZO_SCENARIO__SEED", default_value_t = 159)]
    seed: u64,
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    if cli.list {
        for scenario in scenarios::catalogue() {
            println!("{} [{}]", scenario.name, scenario.proves.join(","));
        }
        return ExitCode::SUCCESS;
    }
    let selected = match scenarios::select(&cli.scenario) {
        Ok(selected) => selected,
        Err(error) => {
            eprintln!("error: {error:#}");
            return ExitCode::FAILURE;
        }
    };
    let harness = match Harness::from_env(cli.seed).await {
        Ok(harness) => harness,
        Err(error) => {
            eprintln!("error: {error:#}");
            return ExitCode::FAILURE;
        }
    };
    if let Err(error) = harness.assert_time_scale_agreement().await {
        eprintln!("startup check failed: {error:#}");
        return ExitCode::FAILURE;
    }
    for scenario in selected {
        println!("\n=== {} ===", scenario.name);
        if let Err(error) = harness.run(scenario).await {
            eprintln!("scenario {} FAILED: {error:#}", scenario.name);
            if let Err(dump_error) = harness.dump_failure(scenario.name).await {
                eprintln!("failure diagnostics also failed: {dump_error:#}");
            }
            return ExitCode::FAILURE;
        }
        println!("scenario {} PASSED", scenario.name);
    }
    ExitCode::SUCCESS
}
