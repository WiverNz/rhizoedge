//! There is exactly one way to move the pump, and it goes through the shared
//! gate.
//!
//! A structural test over the crate's own source. Behaviour tests show that the
//! gate refuses what it should; this one shows there is nothing *else* — no
//! second path that a refusal would never be consulted for. PRD 020 F-020-20 is
//! the requirement everything the simulator claims rests on, and it is a
//! property of the code's shape rather than of any single execution, so it is
//! checked as one.
//!
//! # Why counting call sites, not mentions
//!
//! The obvious check — `grep -c validate_water_command` — counts the `use`
//! statement and every doc comment that names the function, and would therefore
//! fail against a codebase that documents itself. What matters is how many
//! places *call* it. This test counts call expressions and ignores comments,
//! which is the property the requirement actually states.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

/// The crate's `src` directory.
fn source_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

/// Every `.rs` file under `src`, recursively.
fn sources() -> Vec<PathBuf> {
    fn walk(dir: &Path, found: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).expect("src must be readable") {
            let path = entry.expect("a readable directory entry").path();
            if path.is_dir() {
                walk(&path, found);
            } else if path.extension().is_some_and(|e| e == "rs") {
                found.push(path);
            }
        }
    }
    let mut found = Vec::new();
    walk(&source_root(), &mut found);
    found.sort();
    assert!(!found.is_empty(), "the crate must have sources to scan");
    found
}

/// Lines of code — comments excluded — that contain a needle.
///
/// A line-level comment filter, which is the right granularity here: this crate
/// writes doc comments above items, never a `/* … */` wrapped around live code.
fn code_lines_containing(needle: &str) -> Vec<(PathBuf, usize, String)> {
    let mut hits = Vec::new();
    for path in sources() {
        let text = std::fs::read_to_string(&path).expect("a readable source file");
        for (index, line) in text.lines().enumerate() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") {
                continue;
            }
            if line.contains(needle) {
                hits.push((path.clone(), index + 1, line.trim().to_owned()));
            }
        }
    }
    hits
}

/// PRD 020 F-020-20: the only actuation path calls the shared validator.
#[test]
fn exactly_one_call_site_of_the_shared_water_command_validator() {
    let calls: Vec<_> = code_lines_containing("validate_water_command(")
        .into_iter()
        // The `use` statement names the function without calling it.
        .filter(|(_, _, line)| !line.starts_with("use ") && !line.contains("    CommandVerdict,"))
        .collect();
    assert_eq!(
        calls.len(),
        1,
        "expected exactly one call site of the shared gate, found {}:\n{}",
        calls.len(),
        calls
            .iter()
            .map(|(path, line, text)| format!("  {}:{line}  {text}", path.display()))
            .collect::<Vec<_>>()
            .join("\n")
    );
    let (path, _, _) = &calls[0];
    assert!(
        path.ends_with("command.rs"),
        "the gate is called from {}, not from the command module",
        path.display()
    );
}

/// The named shortcuts ADR-008 §Risks warns about, and the shapes they take.
///
/// The risk is not that someone sets out to defeat the safety gate. It is that
/// a debugging affordance added in an afternoon survives into a test topology,
/// and every safety claim made against that topology becomes false without
/// anything failing.
#[test]
fn no_bypass_or_force_path_exists() {
    for forbidden in [
        "force_water",
        "debug_water",
        "raw_pump",
        "test_bypass",
        "unsafe_actuate",
        "allow_any_dose",
        "skip_validation",
        "bypass_gate",
    ] {
        let hits = code_lines_containing(forbidden);
        assert!(
            hits.is_empty(),
            "`{forbidden}` must not exist anywhere in the simulator:\n{}",
            hits.iter()
                .map(|(path, line, text)| format!("  {}:{line}  {text}", path.display()))
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
}

/// The pump is started from one place, and that place is the gate's accept arm.
#[test]
fn the_pump_is_started_from_exactly_one_place() {
    let starts: Vec<_> = code_lines_containing("self.start_pump(")
        .into_iter()
        .filter(|(path, _, _)| !path.ends_with("pump.rs"))
        .collect();
    assert_eq!(
        starts.len(),
        1,
        "the pump must be started from one place only, found {}:\n{}",
        starts.len(),
        starts
            .iter()
            .map(|(path, line, text)| format!("  {}:{line}  {text}", path.display()))
            .collect::<Vec<_>>()
            .join("\n")
    );
    assert!(starts[0].0.ends_with("command.rs"));
}

/// One clock per process (M2-014, ADR-013).
///
/// A component that read a clock directly would age at a different rate from
/// everything around it once `--time-scale` is not 1, and the resulting bug is
/// extremely confusing: readings that disagree with the timestamps attached to
/// them.
#[test]
fn nothing_outside_the_clock_module_reads_a_clock() {
    // The device's wall clock comes only from `edge.time`, so there is no
    // `Utc::now` anywhere at all — the simulator does not even depend on a
    // wall-clock crate. Asserting zero rather than "only in clock.rs" is the
    // stronger statement, and it is the one that is true.
    assert!(
        code_lines_containing("Utc::now").is_empty(),
        "a device learns the time from the Edge, never from its host"
    );

    let stray: Vec<_> = code_lines_containing("Instant::now")
        .into_iter()
        .filter(|(path, _, line)| {
            // `clock.rs` is the one place a monotonic instant is anchored.
            !path.ends_with("clock.rs")
                // The MQTT shutdown drain is a *network* timeout, not a device
                // timer: it bounds how long the socket waits for a DISCONNECT
                // to go out. Scaling it would make a 5 s drain 8 ms at scale
                // 600 and turn a clean stop back into a will.
                && !line.contains("tokio::time::Instant::now")
        })
        .collect();
    assert!(
        stray.is_empty(),
        "these read a clock outside the clock module:\n{}",
        stray
            .iter()
            .map(|(path, line, text)| format!("  {}:{line}  {text}", path.display()))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

/// M6-019: the simulator calls the **one** shared offline evaluator, from
/// exactly one place, and implements none of its rules itself.
///
/// Through M5 this test asserted the opposite — that `evaluate_offline` appeared
/// nowhere at all — because M2 deliberately built the seam and stopped. The
/// boundary has moved, and what replaces it is the stronger claim: there is one
/// call, and there is no second implementation for it to disagree with.
///
/// A simulator-specific evaluator, even a small one, even a temporary one, would
/// make every offline safety test in M6 and every isolation scenario in M8
/// exercise rules the hardware does not follow (ADR-008).
#[test]
fn exactly_one_call_site_of_the_shared_offline_evaluator() {
    let calls: Vec<_> = code_lines_containing("evaluate_offline(")
        .into_iter()
        .filter(|(_, _, line)| !line.starts_with("use "))
        .collect();
    assert_eq!(
        calls.len(),
        1,
        "expected exactly one call site of the shared offline evaluator, found {}:
{}",
        calls.len(),
        calls
            .iter()
            .map(|(path, line, text)| format!("  {}:{line}  {text}", path.display()))
            .collect::<Vec<_>>()
            .join(
                "
"
            )
    );
    let (path, _, _) = &calls[0];
    assert!(
        path.ends_with("offline.rs"),
        "the offline evaluator is called from {}, not from the offline module",
        path.display()
    );

    // ...and the decision vocabulary is *consumed*, never redefined. A local
    // `enum OfflineDecision` or a second `RefuseReason` would be the divergence.
    for forbidden in [
        "enum OfflineDecision",
        "enum RefuseReason",
        "fn decide_offline",
        // `fn evaluate_offline` with the shared crate's exact signature. The
        // simulator's own `evaluate_offline_autonomy` is the *call site* and is
        // not an implementation, which the trailing `(` distinguishes.
        "fn evaluate_offline(",
    ] {
        let hits = code_lines_containing(forbidden);
        assert!(
            hits.is_empty(),
            "`{forbidden}` must be defined in `rhizo-policy`, not here:
{}",
            hits.iter()
                .map(|(path, line, text)| format!("  {}:{line}  {text}", path.display()))
                .collect::<Vec<_>>()
                .join(
                    "
"
                )
        );
    }

    // The seam M2 prepared is still the only way the arguments are assembled.
    assert!(
        !code_lines_containing("fn offline_seam").is_empty(),
        "the integration seam must still exist"
    );
}

/// An autonomous dose reaches the pump through the *same* path a command does.
///
/// `autonomous_dose` is a second **entry**, deliberately, because an isolated
/// device has no command to validate — but it must not be a second *path*: it
/// bounds the volume with the shared `bound_dose` (protocol §5.8 steps 10-12,
/// the one implementation of the firmware ceilings) and then reaches
/// `begin_dose`, which is where `start_pump` lives.
#[test]
fn the_autonomous_path_reuses_the_shared_hard_limits_and_the_shared_dose_start() {
    let entries: Vec<_> = code_lines_containing("fn autonomous_dose")
        .into_iter()
        .collect();
    assert_eq!(entries.len(), 1, "one autonomous entry point");
    assert!(entries[0].0.ends_with("command.rs"));

    let bounds: Vec<_> = code_lines_containing("bound_dose(")
        .into_iter()
        .filter(|(_, _, line)| !line.starts_with("use "))
        .collect();
    assert_eq!(
        bounds.len(),
        1,
        "the firmware ceilings are applied from one place: {bounds:?}"
    );
    assert!(bounds[0].0.ends_with("command.rs"));

    let begins: Vec<_> = code_lines_containing("self.begin_dose(")
        .into_iter()
        .collect();
    assert_eq!(
        begins.len(),
        2,
        "a commanded dose and an autonomous one, both through `begin_dose`: {begins:?}"
    );
    assert!(
        begins
            .iter()
            .all(|(path, _, _)| path.ends_with("command.rs"))
    );
}

/// The test hook cannot be reached from the running simulator.
///
/// `store_mut_for_test` can seed a shorter cooldown, so it must never be
/// callable from anything the binary runs. It is behind a feature the shipped
/// binary does not enable; this asserts the stronger property that no
/// production code calls it either.
#[test]
fn the_test_hook_has_no_call_site_in_the_simulator() {
    let hits: Vec<_> = code_lines_containing("store_mut_for_test")
        .into_iter()
        .filter(|(_, _, line)| !line.contains("pub fn store_mut_for_test"))
        .collect();
    assert!(
        hits.is_empty(),
        "the test hook must have no call site in `src`:
{}",
        hits.iter()
            .map(|(path, line, text)| format!("  {}:{line}  {text}", path.display()))
            .collect::<Vec<_>>()
            .join(
                "
"
            )
    );
}

/// The scanner has to be able to find something, or every assertion above is
/// vacuous: an empty file list, a wrong root, or a comment filter that swallowed
/// the whole file would all read as "no violations".
#[test]
fn the_scanner_actually_reads_the_sources() {
    assert!(sources().len() > 10, "the crate has more sources than this");
    assert!(
        !code_lines_containing("pub fn").is_empty(),
        "the scanner found no code at all"
    );
    assert!(
        code_lines_containing("//! Command handling").is_empty(),
        "the comment filter must exclude comment lines"
    );
}
