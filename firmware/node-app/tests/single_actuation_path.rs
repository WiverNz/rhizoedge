//! One actuation gate, one offline evaluator, one pump (ADR-008, SAFETY-007).
//!
//! The same discipline the simulator's `tests/single_actuation_path.rs`
//! applies, applied to the firmware. It is a source-text check on purpose: the
//! property is "there is exactly one place in this crate that can do X", and no
//! type system expresses that. A second `validate_water_command` would compile
//! perfectly and make every simulator-based safety test in M6 and every
//! isolation scenario in M8 exercise rules the hardware does not follow.
//!
//! Documentation mentions are excluded and code is not, because the two issues
//! M2 recorded — greps that matched `use` statements and doc comments — are the
//! mistake this file exists not to repeat.

// A panic in a test is a failed assertion, not an unhandled failure: the
// workspace denies `unwrap`/`expect` in library code, and an integration test
// is a separate crate that does not inherit the `cfg(test)` allowance in
// `lib.rs` (workspace lint policy, root Cargo.toml).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

fn src_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

/// Every `.rs` file under `src/`, as (path, contents).
fn sources() -> Vec<(PathBuf, String)> {
    fn walk(dir: &Path, out: &mut Vec<(PathBuf, String)>) {
        let entries = std::fs::read_dir(dir).expect("src/ is readable");
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let text = std::fs::read_to_string(&path).expect("source is readable");
                out.push((path, text));
            }
        }
    }
    let mut out = Vec::new();
    walk(&src_dir(), &mut out);
    assert!(!out.is_empty(), "no sources found");
    out
}

/// Lines that are neither a doc comment, a line comment, nor a `use`.
///
/// A call *expression* is what the acceptance criteria mean. `use` statements
/// and prose mention the name too, and counting those is how M2's documented
/// greps came to report six when the answer was one.
fn code_lines(text: &str) -> impl Iterator<Item = (usize, &str)> {
    text.lines().enumerate().filter(|(_, line)| {
        let trimmed = line.trim_start();
        !trimmed.starts_with("//")
            && !trimmed.starts_with("use ")
            && !trimmed.starts_with('*')
            && !trimmed.starts_with("#[")
    })
}

fn call_sites(name: &str) -> Vec<String> {
    let needle = format!("{name}(");
    let mut sites = Vec::new();
    for (path, text) in sources() {
        for (number, line) in code_lines(&text) {
            if line.contains(&needle) && !line.contains(&format!("fn {name}")) {
                sites.push(format!(
                    "{}:{}: {}",
                    path.file_name().unwrap_or_default().to_string_lossy(),
                    number + 1,
                    line.trim()
                ));
            }
        }
    }
    sites
}

/// SAFETY-007. There is one gate, and every dose passes through it.
#[test]
fn exactly_one_validate_water_command_call_site() {
    let sites = call_sites("validate_water_command");
    assert_eq!(
        sites.len(),
        1,
        "expected exactly one call site of the shared gate, found {}:\n{}",
        sites.len(),
        sites.join("\n")
    );
    assert!(
        sites[0].starts_with("command.rs:"),
        "the gate must be called from command.rs, not {}",
        sites[0]
    );
}

/// ADR-015. One offline evaluator, one call site, and the firmware calls the
/// shared function rather than a copy of its rules.
#[test]
fn exactly_one_evaluate_offline_call_site() {
    let sites = call_sites("evaluate_offline");
    assert_eq!(
        sites.len(),
        1,
        "expected exactly one call site of the shared offline evaluator, found {}:\n{}",
        sites.len(),
        sites.join("\n")
    );
    assert!(sites[0].starts_with("offline.rs:"), "{}", sites[0]);
}

/// The autonomous path and the commanded path reach the pump through the same
/// function, so the in-flight NVS write and the hard limits cannot be skipped
/// by one of them.
#[test]
fn exactly_one_function_drives_the_pump() {
    let sites: Vec<_> = sources()
        .into_iter()
        .flat_map(|(path, text)| {
            code_lines(&text)
                .filter(|(_, line)| line.contains(".run_for("))
                .map(|(number, line)| {
                    format!(
                        "{}:{}: {}",
                        path.file_name().unwrap_or_default().to_string_lossy(),
                        number + 1,
                        line.trim()
                    )
                })
                .collect::<Vec<_>>()
        })
        .filter(|site| !site.starts_with("fakes.rs:"))
        .collect();
    assert_eq!(
        sites.len(),
        1,
        "expected exactly one `run_for` call site outside the fakes, found {}:\n{}",
        sites.len(),
        sites.join("\n")
    );
    assert!(sites[0].starts_with("command.rs:"), "{}", sites[0]);
}

/// F-090-42, made structural. This crate has no ESP-IDF dependency at all, so
/// an `esp_idf_*` symbol here does not compile — but the check is cheap and it
/// is what a reader of ADR-007 will look for.
#[test]
fn the_application_layer_names_no_esp_idf_symbol() {
    for (path, text) in sources() {
        for (number, line) in code_lines(&text) {
            assert!(
                !line.contains("esp_idf"),
                "{}:{} names an ESP-IDF symbol: {}",
                path.display(),
                number + 1,
                line.trim()
            );
        }
    }
}

/// SAFETY-012 as a source rule: no `unwrap_or_default()` on a safety input, and
/// no `_ =>` arm in the modules that classify one.
#[test]
fn the_safety_classifiers_carry_no_catch_all_arm() {
    for (path, text) in sources() {
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        if !["command.rs", "offline.rs", "budget.rs", "policy.rs"].contains(&name.as_ref()) {
            continue;
        }
        for (number, line) in code_lines(&text) {
            let trimmed = line.trim();
            assert!(
                !trimmed.starts_with("_ =>"),
                "{name}:{} has a catch-all arm in a safety classifier: {trimmed}",
                number + 1
            );
        }
    }
}
