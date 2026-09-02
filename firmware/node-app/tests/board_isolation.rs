//! Board isolation, checked structurally rather than by convention
//! (M9-003, M9-022, F-090-45, F-090-46).
//!
//! Board isolation that is only a convention stops being true the first time
//! somebody is in a hurry, and the cost lands in M10–M14 as "editing safety
//! code to change hardware". So it is a test.
//!
//! # Why it lives here and not in `esp32-node/tests/`
//!
//! `firmware/esp32-node` builds for `riscv32imc-esp-espidf`, so `cargo test`
//! there would cross-compile a test binary that cannot run without a board.
//! These are checks on the **source text**, which needs no target at all, so
//! they run in the host workspace where they can actually execute — with no
//! board, no ESP-IDF installation, and no nightly toolchain.
//!
//! The paths are relative to this crate, so the test fails loudly if the
//! firmware crate is moved rather than passing vacuously on an empty scan.
//! Every scan asserts it found files first, for the same reason.

// A panic in a test is a failed assertion, not an unhandled failure: the
// workspace denies `unwrap`/`expect` in library code, and an integration test
// is a separate crate that does not inherit the `cfg(test)` allowance in
// `lib.rs` (workspace lint policy, root Cargo.toml).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

fn firmware_src() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("esp32-node")
        .join("src")
}

/// Every `.rs` file under the firmware crate, as (relative path, contents).
fn firmware_sources() -> Vec<(String, String)> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, String)>) {
        let entries =
            std::fs::read_dir(dir).unwrap_or_else(|e| panic!("{} is readable: {e}", dir.display()));
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(root, &path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let relative = path
                    .strip_prefix(root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .replace('\\', "/");
                let text = std::fs::read_to_string(&path).expect("source is readable");
                out.push((relative, text));
            }
        }
    }
    let root = firmware_src();
    assert!(
        root.is_dir(),
        "the firmware crate is not where this test expects it: {}",
        root.display()
    );
    let mut out = Vec::new();
    walk(&root, &root, &mut out);
    assert!(
        out.len() >= 5,
        "found only {} firmware sources; the scan is not looking where it thinks",
        out.len()
    );
    out
}

/// Lines that are not comments.
fn code_lines(text: &str) -> impl Iterator<Item = (usize, &str)> {
    text.lines().enumerate().filter(|(_, line)| {
        let trimmed = line.trim_start();
        !trimmed.starts_with("//") && !trimmed.starts_with('*')
    })
}

/// Whether a path is inside the board layer.
fn is_board(path: &str) -> bool {
    path.starts_with("board/") || path == "board.rs"
}

/// F-090-45. No **concrete** GPIO number or pin polarity outside `src/board/`.
///
/// Two distinct things, and the difference is the whole design:
///
/// * a *concrete* pin — `Gpio5`, `gpio5`, `pins.gpio5` — is board detail and
///   may appear only in the board layer;
/// * a *parameter* named `active_high`, handed down from the board, is the
///   abstraction working. `GpioPump` must be able to accept a polarity without
///   knowing which one, or the board would have nothing to supply it to.
///
/// So the rule is: no pin token with a number in it, and no **constant bound to
/// a literal** whose name says it is a pin or a polarity. The second half is
/// what stops the test being satisfied by moving a pin map into
/// `src/hal/consts.rs` to tidy up.
#[test]
fn no_gpio_number_or_polarity_outside_the_board_layer() {
    let mut offences = Vec::new();
    for (path, text) in firmware_sources() {
        if is_board(&path) {
            continue;
        }
        for (number, line) in code_lines(&text) {
            let trimmed = line.trim();

            // A concrete pin: `gpio` immediately followed by digits.
            let concrete_pin = line
                .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
                .any(|token| {
                    let lower = token.to_ascii_lowercase();
                    lower.starts_with("gpio")
                        && lower.len() > 4
                        && lower[4..].chars().all(|c| c.is_ascii_digit())
                });

            // A pin map moved out of the board layer: a `const` or `static`
            // whose name claims to be a pin or a polarity, bound to a literal.
            let literal_map = (trimmed.starts_with("const ") || trimmed.starts_with("static "))
                && trimmed.contains('=')
                && {
                    let name = trimmed
                        .split_whitespace()
                        .nth(1)
                        .unwrap_or_default()
                        .trim_end_matches(':')
                        .to_ascii_uppercase();
                    name.contains("GPIO")
                        || name.ends_with("_PIN")
                        || name.contains("ACTIVE_HIGH")
                        || name.contains("ACTIVE_LOW")
                };

            if concrete_pin || literal_map {
                offences.push(format!("{path}:{}: {trimmed}", number + 1));
            }
        }
    }
    assert!(
        offences.is_empty(),
        "board detail leaked outside src/board/:\n{}",
        offences.join("\n")
    );
}

/// The board layer exists, names pins, and is therefore actually doing its job.
///
/// The negative test above would also pass if nobody had written a pin map at
/// all, which is why this one exists: the property is "pins are *here*", not
/// "pins are nowhere".
#[test]
fn the_board_layer_is_where_the_pins_actually_live() {
    let board: Vec<_> = firmware_sources()
        .into_iter()
        .filter(|(path, _)| is_board(path))
        .collect();
    assert!(!board.is_empty(), "there is no board layer");
    let names_a_pin = board.iter().any(|(_, text)| {
        code_lines(text).any(|(_, line)| line.contains("gpio") || line.contains("GPIO"))
    });
    assert!(
        names_a_pin,
        "the board layer names no pin, so either it is not the pin map or the pin map is elsewhere"
    );
}

/// F-090-46. Zero or two board profiles must be a `compile_error!` that names
/// the available profiles.
///
/// Checked by reading the guard rather than by running two failing builds: the
/// build check belongs in CI (M9-002) where a real `cargo build` can be run,
/// and a unit test that shells out to a cross-compiler would be a test that
/// only passes on a machine with ESP-IDF installed.
#[test]
fn zero_or_two_board_profiles_is_a_compile_error_naming_the_profiles() {
    let (_, text) = firmware_sources()
        .into_iter()
        .find(|(path, _)| path == "board/mod.rs")
        .expect("src/board/mod.rs exists");

    assert!(
        text.contains("all(feature = \"board-devkitm1\", feature = \"board-xiao-esp32c3\")"),
        "no guard against two profiles"
    );
    assert!(
        text.contains("not(any(feature = \"board-devkitm1\", feature = \"board-xiao-esp32c3\"))"),
        "no guard against zero profiles"
    );
    // Code occurrences only: the module documentation mentions the macro by
    // name, and counting prose would make this assertion about the comments.
    let guards = code_lines(&text)
        .filter(|(_, line)| line.contains("compile_error!"))
        .count();
    assert_eq!(
        guards, 3,
        "expected a guard for zero profiles, one for two, and one for the reserved profile"
    );
    // Every message must name the profiles: a build that fails with "feature
    // combination invalid" tells a developer nothing they can act on. The scan
    // is over code only, for the same reason the count above is.
    let code: String = code_lines(&text)
        .map(|(_, line)| line)
        .collect::<Vec<_>>()
        .join("\n");
    for message in code.split("compile_error!").skip(1) {
        let head: String = message.chars().take(600).collect();
        assert!(
            head.contains("board-devkitm1") && head.contains("board-xiao-esp32c3"),
            "a compile_error! does not name the available profiles: {head}"
        );
    }
}

/// F-090-51. Exactly one `esp_deep_sleep` call site, and it is in `hal/sleep.rs`.
#[test]
fn exactly_one_deep_sleep_call_site() {
    let sites: Vec<_> = firmware_sources()
        .into_iter()
        .flat_map(|(path, text)| {
            code_lines(&text)
                .filter(|(_, line)| line.contains("esp_deep_sleep("))
                .map(|(number, line)| format!("{path}:{}: {}", number + 1, line.trim()))
                .collect::<Vec<_>>()
        })
        .collect();
    assert_eq!(
        sites.len(),
        1,
        "expected exactly one esp_deep_sleep call site, found:\n{}",
        sites.join("\n")
    );
    assert!(sites[0].starts_with("hal/sleep.rs:"), "{}", sites[0]);
}

/// F-090-56. No stabilisation constant for any specific sensor part.
///
/// M10-011 measures the real figure for whatever probe is fitted; a hardcoded
/// value is a per-part guess baked into a binary, and the whole reason M10-011
/// exists is that nobody knows the number yet.
#[test]
fn no_hardcoded_sensor_warmup_constant() {
    let mut offences = Vec::new();
    for (path, text) in firmware_sources() {
        for (number, line) in code_lines(&text) {
            let lower = line.to_ascii_lowercase();
            if lower.contains("sen0601") {
                offences.push(format!("{path}:{}: names a specific part", number + 1));
            }
            // `warmup_ms = 2500` or `warmup_ms: 2500` — a literal bound to the
            // name. A parameter named `warmup_ms` is fine; a number is not.
            if let Some(rest) = lower.split_once("warmup_ms").map(|(_, rest)| rest) {
                let rest = rest.trim_start();
                if let Some(value) = rest.strip_prefix(':').or_else(|| rest.strip_prefix('='))
                    && value.trim_start().starts_with(|c: char| c.is_ascii_digit())
                {
                    offences.push(format!("{path}:{}: {}", number + 1, line.trim()));
                }
            }
        }
    }
    assert!(
        offences.is_empty(),
        "a stabilisation constant for a specific part appears in the firmware:\n{}",
        offences.join("\n")
    );
}

/// ADR-018 §7. No battery field reaches an irrigation decision.
///
/// The check is that the safety-relevant modules of the application crate never
/// mention battery state at all. `MeasurementKind::is_power_telemetry` already
/// excludes it from a policy's control measurement, and this is the second
/// half: it must not arrive by some other route either.
#[test]
fn no_battery_field_reaches_the_irrigation_path() {
    let app_src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut offences = Vec::new();
    for name in ["command.rs", "offline.rs", "budget.rs", "policy.rs"] {
        let path = app_src.join(name);
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("{} is readable: {e}", path.display()));
        for (number, line) in code_lines(&text) {
            if line.to_ascii_lowercase().contains("battery") {
                offences.push(format!("{name}:{}: {}", number + 1, line.trim()));
            }
        }
    }
    assert!(
        offences.is_empty(),
        "a battery field appears in the irrigation path:\n{}",
        offences.join("\n")
    );
}

/// The firmware crate depends on the shared crates **by path**, with default
/// features off (F-090-04).
#[test]
fn the_shared_crates_are_path_dependencies_with_default_features_off() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("esp32-node")
        .join("Cargo.toml");
    let text = std::fs::read_to_string(&manifest).expect("the firmware manifest is readable");
    for crate_name in ["rhizo-mqtt-contract", "rhizo-policy"] {
        let line = text
            .lines()
            .find(|line| line.starts_with(crate_name))
            .unwrap_or_else(|| panic!("{crate_name} is not a dependency"));
        assert!(
            line.contains("path = "),
            "{crate_name} is not a path dependency: {line}"
        );
        assert!(
            line.contains("default-features = false"),
            "{crate_name} does not disable default features: {line}"
        );
    }
}

/// The application crate has no ESP-IDF dependency, which is what makes
/// F-090-42 structural rather than grep-enforced.
#[test]
fn the_application_crate_has_no_esp_idf_dependency() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let text = std::fs::read_to_string(manifest).expect("the manifest is readable");
    for line in text.lines() {
        assert!(
            !line.starts_with("esp-idf"),
            "the application crate depends on ESP-IDF: {line}"
        );
    }
}
