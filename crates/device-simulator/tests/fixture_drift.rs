//! The fixture corpus still describes what the simulator really publishes.
//!
//! ADR-008 §Risks names fixture rot: the corpus stops reflecting real messages
//! and quietly stops proving anything. This drives a real device through a real
//! cycle, captures one example of every kind it publishes, and checks the
//! capture against `test/fixtures/protocol/valid/`.
//!
//! # Why this is a Rust test rather than a script
//!
//! M2-011 sketches `tools/check_fixture_drift.py` invoked on the output of
//! `--capture-fixtures`. The check is implemented here instead, for the same
//! reason `rhizo-docscheck` is Rust: it then runs inside `cargo test` with no
//! second toolchain in CI, it cannot drift from the capture code it checks, and
//! it needs no broker. `--capture-fixtures` still exists and still writes the
//! same files — it is how a person inspects a drift once this test reports one.
//!
//! # A drift fails; it never rewrites
//!
//! `versioning-policy.md` makes the corpus append-only precisely because the
//! instinct on a failure is to edit the fixture.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use device_simulator::capture::{FixtureCapture, field_paths, file_stem};
use device_simulator::{Cli, Device};
use rhizo_mqtt_contract::{DeviceId, MessageId, Topic};
use uuid::Uuid;

const SYNCED_AT_MS: i64 = 1_756_121_400_000;

fn corpus_directory() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("crates/<name> is two levels below the workspace root")
        .join("test/fixtures/protocol/valid")
}

/// The union of field paths across every fixture, keyed by capture file stem.
///
/// A union rather than per-file matching: several fixtures exist for one kind
/// specifically to pin the partial shapes, and the question the check asks is
/// "is this path described anywhere in the corpus", not "is there one fixture
/// that describes the whole message".
fn corpus() -> HashMap<String, BTreeSet<String>> {
    let mut union: HashMap<String, BTreeSet<String>> = HashMap::new();
    let directory = corpus_directory();
    for entry in std::fs::read_dir(&directory).expect("the fixture corpus must exist") {
        let path = entry.expect("a readable fixture").path();
        if path.extension().is_none_or(|e| e != "json") {
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("a readable fixture");
        let value: serde_json::Value =
            serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let kind = value["kind"]
            .as_str()
            .unwrap_or_else(|| panic!("{} has no `kind`, so it cannot be matched", path.display()));
        let stem = stem_for_kind(kind);
        union
            .entry(stem.to_owned())
            .or_default()
            .extend(field_paths(&value));
    }
    assert!(
        !union.is_empty(),
        "the corpus is empty; nothing to check against"
    );
    union
}

/// Maps a wire `kind` to the same file stem [`file_stem`] produces.
fn stem_for_kind(kind: &str) -> &'static str {
    let message_kind: rhizo_mqtt_contract::MessageKind =
        serde_json::from_value(serde_json::Value::String(kind.to_owned()))
            .unwrap_or_else(|_| panic!("`{kind}` is not a message kind this contract knows"));
    file_stem(message_kind)
}

fn envelope(kind: &str, data: serde_json::Value) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "v": 1,
        "kind": kind,
        "message_id": MessageId::from_uuid(Uuid::from_u128(1)),
        "device_id": "plant-node-01",
        "data": data,
    }))
    .unwrap()
}

fn topic(kind: &str) -> Topic {
    let id = DeviceId::parse("plant-node-01").unwrap();
    match kind {
        "time" => Topic::Time(id),
        "config" => Topic::Config(id),
        "water" => Topic::CommandWater(id),
        other => panic!("no such topic: {other}"),
    }
}

fn scratch_state_file() -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push("rhizo-fixture-drift");
    let _ = std::fs::create_dir_all(&path);
    path.push(format!("{}.state.json", std::process::id()));
    for extension in ["json", "json.corrupt", "json.tmp"] {
        let _ = std::fs::remove_file(path.with_extension(extension));
    }
    let _ = std::fs::remove_file(&path);
    path
}

/// Drives a full cycle and captures one example of every kind published.
fn capture_a_full_cycle() -> FixtureCapture {
    use clap::Parser;
    let state_file = scratch_state_file();
    let cli = Cli::try_parse_from([
        "device-simulator",
        "--device-id",
        "plant-node-01",
        "--telemetry-interval",
        "10",
        "--initial-moisture",
        "15",
        "--state-file",
        &state_file.display().to_string(),
    ])
    .unwrap();
    cli.validate().unwrap();

    let mut device = Device::new(&cli);
    let mut capture = FixtureCapture::new();
    let offer = |capture: &mut FixtureCapture, publications: Vec<_>| {
        for publication in &publications {
            capture.offer(publication);
        }
    };

    offer(&mut capture, device.on_connected().unwrap());
    // Synchronise, so the envelope carries `device_time_ms` and `clock_synced`
    // and the capture is of a device in its normal operating state.
    offer(
        &mut capture,
        device.on_message(
            &topic("time"),
            &envelope(
                "edge.time",
                serde_json::json!({ "edge_time_ms": SYNCED_AT_MS }),
            ),
        ),
    );
    // Apply a config, so `applied_config_version` is a real value.
    offer(
        &mut capture,
        device.on_message(
            &topic("config"),
            &envelope(
                "device.config",
                serde_json::json!({
                    "config_version": 7,
                    "telemetry_interval_seconds": 10,
                    "pump": { "ml_per_second": 8.2, "enabled": true },
                    "tank": { "min_percent": 15.0 },
                    "sensors": { "soil": true, "weight": true, "tank": true, "leak": true },
                }),
            ),
        ),
    );
    // One sampling cycle, and an actuator-state change.
    offer(&mut capture, device.tick(10_000));
    // A full command cycle, for the result.
    offer(
        &mut capture,
        device.on_message(
            &topic("water"),
            &envelope(
                "command.water",
                serde_json::json!({
                    "command_id": "018fd7b1-4c2e-7f10-a3b8-9d1e2f304050",
                    "requested_ml": 40.0,
                    "issued_at_ms": SYNCED_AT_MS,
                    "expires_at_ms": SYNCED_AT_MS + 120_000,
                }),
            ),
        ),
    );
    for _ in 0..200 {
        offer(&mut capture, device.tick(100));
        if !device.pump_running() {
            break;
        }
    }
    offer(&mut capture, device.tick(100));
    capture
}

#[test]
fn the_capture_contains_every_kind_the_simulator_publishes() {
    let capture = capture_a_full_cycle();
    let mut kinds: Vec<_> = capture.examples().keys().cloned().collect();
    kinds.sort();
    assert_eq!(
        kinds,
        vec![
            "actuator",
            "command-result",
            "events",
            "status",
            "telemetry-batch",
        ],
        "the cycle must exercise every kind the device emits in M2"
    );
}

#[test]
fn the_corpus_still_covers_what_the_simulator_publishes() {
    let capture = capture_a_full_cycle();
    let drifts = device_simulator::capture::drift(&capture, &corpus());
    assert!(
        drifts.is_empty(),
        "the simulator publishes field paths the fixture corpus does not describe:\n{}\n\n\
         This is a decision, not a fix. If the new field is intended, add a fixture \
         to test/fixtures/protocol/valid/ that contains it. Do not edit an existing \
         fixture: the corpus is append-only (versioning-policy.md).",
        drifts
            .iter()
            .map(|d| format!("  {}: {}", d.kind, d.path))
            .collect::<Vec<_>>()
            .join("\n")
    );
}

#[test]
fn every_captured_message_decodes_as_its_concrete_payload_type() {
    use rhizo_mqtt_contract::Envelope;
    use rhizo_mqtt_contract::payload::{
        ActuatorState, CommandResult, DeviceEventBatch, DeviceStatus, TelemetryBatch,
    };

    let capture = capture_a_full_cycle();
    for (kind, payload) in capture.examples() {
        let bytes = payload.as_bytes();
        match kind.as_str() {
            "status" => {
                Envelope::<DeviceStatus>::from_json(bytes).expect("a decodable status");
            }
            "telemetry-batch" => {
                let decoded =
                    Envelope::<TelemetryBatch>::from_json(bytes).expect("a decodable batch");
                decoded.data.validate().expect("a structurally valid batch");
                for sample in &decoded.data.samples {
                    assert!(sample.validate().is_valid(), "{sample:?}");
                }
            }
            "actuator" => {
                Envelope::<ActuatorState>::from_json(bytes).expect("a decodable actuator state");
            }
            "command-result" => {
                Envelope::<CommandResult>::from_json(bytes).expect("a decodable result");
            }
            "events" => {
                let decoded = Envelope::<DeviceEventBatch>::from_json(bytes)
                    .expect("a decodable event batch");
                decoded
                    .data
                    .validate()
                    .expect("no duplicate ids in a batch");
                assert!(decoded.data.replay, "a reconnection replays history");
            }
            other => panic!("no decode check for captured kind `{other}`"),
        }
    }
}

/// The check has to be able to see a drift, or every assertion above is vacuous.
#[test]
fn a_deliberately_added_field_is_detected() {
    let capture = capture_a_full_cycle();
    let mut corpus = corpus();
    // Remove one path the capture certainly emits: the same effect as the
    // capture gaining a field the corpus never described.
    let status = corpus.get_mut("status").expect("status fixtures exist");
    assert!(
        status.remove("data.limits.max_ml_per_run"),
        "the corpus must describe the limits block, or this control proves nothing"
    );

    let drifts = device_simulator::capture::drift(&capture, &corpus);
    assert!(
        drifts
            .iter()
            .any(|d| d.kind == "status" && d.path == "data.limits.max_ml_per_run"),
        "an uncovered field path must be reported, got {drifts:?}"
    );
}

#[test]
fn the_corpus_loader_reads_real_fixtures() {
    let corpus = corpus();
    assert!(
        corpus.len() >= 4,
        "the corpus should cover at least the kinds the device publishes"
    );
    assert!(
        corpus["telemetry-batch"].contains("data.samples[].kind"),
        "the loader is not reading fixture structure"
    );
}
