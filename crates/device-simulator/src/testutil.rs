//! Shared helpers for this crate's unit tests.
//!
//! Compiled only under `cfg(test)`. Nothing here is reachable from a running
//! simulator, which is the point: a test affordance that shipped would be a
//! second way to configure the device.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use clap::Parser;

use crate::cli::Cli;

/// Distinguishes state files taken by tests running concurrently.
static NEXT: AtomicU64 = AtomicU64::new(0);

/// A scratch path for a test's persistent state, unique per test and removed
/// before use so a previous run cannot leak into this one.
pub fn scratch_state_file() -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push("rhizo-device-simulator-tests");
    let _ = std::fs::create_dir_all(&path);
    path.push(format!(
        "{}-{}.state.json",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("json.corrupt"));
    path
}

/// Builds validated settings for `plant-node-01` with the given extra flags.
///
/// A scratch `--state-file` is supplied unless the caller names one, so a unit
/// test never writes into the working directory and never inherits another
/// test's persisted state.
pub fn cli(args: &[&str]) -> Cli {
    let mut full = vec!["device-simulator", "--device-id", "plant-node-01"];
    full.extend_from_slice(args);
    let scratch;
    if !args.iter().any(|a| a.starts_with("--state-file")) {
        scratch = scratch_state_file().display().to_string();
        full.push("--state-file");
        full.push(&scratch);
    }
    let cli = Cli::try_parse_from(full).expect("test arguments must parse");
    cli.validate().expect("test arguments must validate");
    cli
}
