//! The protocol fixture corpus, decoded through the firmware's dependency set
//! (M9-003).
//!
//! `crates/mqtt-contract/tests/fixtures.rs` already runs this corpus. Running it
//! here too is not duplication, because it is a *different question*: the
//! firmware depends on the contract crate with `default-features = false`, so
//! this asserts that the corpus decodes without the `std` feature's
//! conveniences and against exactly the dependency set that is compiled into a
//! device.
//!
//! Every payload is decoded as its **concrete type**, never as
//! `serde_json::Value`. That is the whole point: renaming a field turns this
//! red, whereas a `Value` would swallow it.
//!
//! The list is explicit rather than derived from a glob. A fixture added to the
//! corpus and not named here fails the count assertion below, which is the
//! difference between a corpus that is checked and one that is merely present.

// A panic in a test is a failed assertion, not an unhandled failure: the
// workspace denies `unwrap`/`expect` in library code, and an integration test
// is a separate crate that does not inherit the `cfg(test)` allowance in
// `lib.rs` (workspace lint policy, root Cargo.toml).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

use rhizo_mqtt_contract::Envelope;
use rhizo_mqtt_contract::payload::{
    ActuatorState, CommandResult, CommandResultAck, DeviceConfig, DeviceEventBatch, DeviceStatus,
    EdgeTime, EventAck, OfflinePolicySet, TelemetryBatch, WaterCommand,
};

fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("test")
        .join("fixtures")
        .join("protocol")
        .join("valid")
}

fn read(name: &str) -> Vec<u8> {
    let path = corpus().join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("{} is readable: {e}", path.display()))
}

/// Decodes one fixture as its concrete payload type and round-trips it.
///
/// The round trip is what makes "additive within v1" checked rather than
/// intended: a field the firmware's types do not know about is dropped on
/// re-encode, and comparing the two decodes catches it.
fn decodes<T>(name: &str)
where
    T: serde::de::DeserializeOwned + serde::Serialize + PartialEq + std::fmt::Debug,
{
    let bytes = read(name);
    let envelope = Envelope::<T>::from_json(&bytes)
        .unwrap_or_else(|e| panic!("{name} decodes as its concrete payload type: {e}"));
    let re_encoded = envelope.to_json().expect("re-encodes");
    let again = Envelope::<T>::from_json(re_encoded.as_bytes()).expect("round trips");
    assert_eq!(envelope, again, "{name} does not round trip");
}

#[test]
fn every_valid_fixture_decodes_as_its_concrete_payload_type() {
    decodes::<ActuatorState>("actuator.json");
    decodes::<ActuatorState>("actuator-running.json");
    decodes::<rhizo_mqtt_contract::payload::CalibrateCommand>("command-calibrate.json");
    decodes::<CommandResultAck>("command-result-ack.json");
    decodes::<CommandResult>("command-result-interrupted.json");
    decodes::<CommandResult>("command-result.json");
    decodes::<rhizo_mqtt_contract::payload::TareCommand>("command-tare.json");
    decodes::<WaterCommand>("command-water.json");
    decodes::<DeviceConfig>("config-battery-mode.json");
    decodes::<DeviceConfig>("config.json");
    decodes::<EdgeTime>("edge-time.json");
    decodes::<EventAck>("event-ack-first.json");
    decodes::<EventAck>("event-ack.json");
    decodes::<DeviceEventBatch>("events-replay-gap.json");
    decodes::<DeviceEventBatch>("events.json");
    decodes::<OfflinePolicySet>("policy-disabled.json");
    decodes::<OfflinePolicySet>("policy-enabled.json");
    decodes::<DeviceStatus>("status-monitoring-only.json");
    decodes::<DeviceStatus>("status-sleeping.json");
    decodes::<DeviceStatus>("status-with-capabilities.json");
    decodes::<TelemetryBatch>("telemetry-batch.json");
    decodes::<TelemetryBatch>("telemetry-battery-kinds.json");
    decodes::<TelemetryBatch>("telemetry-partial.json");
    decodes::<TelemetryBatch>("telemetry-unknown.json");
}

/// A fixture added to the corpus and not named above fails here.
///
/// Without this the list would silently stop covering the corpus, which is the
/// way a fixture suite quietly stops proving anything.
#[test]
fn the_named_list_covers_the_whole_corpus() {
    let present: Vec<String> = std::fs::read_dir(corpus())
        .expect("the corpus directory exists")
        .flatten()
        .filter(|entry| entry.path().extension().is_some_and(|e| e == "json"))
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        present.len(),
        24,
        "the corpus has {} fixtures and this test names 24; add the new one to \
         `every_valid_fixture_decodes_as_its_concrete_payload_type`: {present:?}",
        present.len()
    );
}

/// A pre-M16 fixture must decode **and** re-encode with no `delivery` key.
///
/// PRD 160 makes this the check that turns "additive within v1" from an
/// intention into a property, and it costs nothing to assert now — before the
/// field exists — so that adding it later has to keep this true.
#[test]
fn a_pre_m16_command_result_round_trips_with_no_delivery_key() {
    let bytes = read("command-result.json");
    let envelope = Envelope::<CommandResult>::from_json(&bytes).expect("decodes");
    let re_encoded = envelope.to_json().expect("re-encodes");
    assert!(
        !re_encoded.contains("delivery"),
        "a pre-M16 result gained a delivery key on re-encode: {re_encoded}"
    );
}
