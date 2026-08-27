//! Directory-driven MQTT v1 compatibility fixture checks.
#![allow(clippy::unwrap_used)]

use rhizo_mqtt_contract::{
    Envelope,
    payload::{MeasurementKind, TelemetryBatch},
};
use serde_json::Value;
use std::{fs, path::PathBuf};
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../test/fixtures/protocol")
}
#[test]
fn fixtures_valid_envelopes_decode_and_reencode() {
    let entries = fs::read_dir(root().join("valid")).unwrap();
    let mut count = 0;
    for entry in entries {
        let path = entry.unwrap().path();
        if path.extension().and_then(|v| v.to_str()) != Some("json") {
            continue;
        }
        let bytes = fs::read(&path).unwrap();
        let env = Envelope::<Value>::from_json(&bytes)
            .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let original: Value = serde_json::from_slice(&bytes).unwrap();
        let encoded: Value = serde_json::from_str(&env.to_json().unwrap()).unwrap();
        assert_eq!(encoded, original);
        count += 1;
    }
    assert!(count >= 10);
}
#[test]
fn fixtures_invalid_envelopes_or_payloads_are_rejected() {
    for name in [
        "unsupported-version.json",
        "missing-envelope-field.json",
        "invalid-device-id.json",
    ] {
        let bytes = fs::read(root().join("invalid").join(name)).unwrap();
        assert!(Envelope::<Value>::from_json(&bytes).is_err(), "{name}");
    }
    for name in ["empty-batch.json", "unit-mismatch.json"] {
        let bytes = fs::read(root().join("invalid").join(name)).unwrap();
        let env = Envelope::<TelemetryBatch>::from_json(&bytes).unwrap();
        if name == "empty-batch.json" {
            assert!(env.data.validate().is_err());
        } else {
            assert!(!env.data.samples[0].validate().is_valid());
        }
    }
}
#[test]
fn fixtures_unknown_kind_is_advisory() {
    let bytes = fs::read(root().join("valid/telemetry-unknown.json")).unwrap();
    let env = Envelope::<TelemetryBatch>::from_json(&bytes).unwrap();
    assert!(matches!(
        env.data.samples[0].kind,
        MeasurementKind::Unknown(_)
    ));
    assert!(env.data.samples[0].advisory_only());
}
