//! Directory-driven MQTT v1 compatibility fixture checks.
//!
//! The corpus is the artefact that outlives every implementation detail: once a
//! v1 device exists, a field that silently stops decoding is a fleet-wide
//! outage. These tests therefore decode each fixture into the **concrete
//! payload type** rather than a generic `Value`, so a rename or a dropped field
//! turns the suite red instead of passing unnoticed.
//!
//! Both halves discover their files from the filesystem. Adding a fixture for an
//! already-supported message kind, or another example of an existing invalid
//! class, needs no change to this file.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use rhizo_mqtt_contract::{DecodeError, DeviceId, Envelope, MessageKind, payload::*};
use serde_json::Value;
use std::{fs, path::PathBuf};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../test/fixtures/protocol")
}

/// Every JSON path present on the wire must survive a decode/encode round trip
/// through the concrete type, with its value intact.
///
/// The check is a *subset* rule, not equality: a type may legitimately
/// materialise a defaulted field the fixture omits (`point`, `sensors`). It may
/// never lose or alter one the fixture states — that is a wire break.
fn assert_preserved(original: &Value, encoded: &Value, ctx: &str) {
    match (original, encoded) {
        (Value::Object(o), Value::Object(e)) => {
            for (key, value) in o {
                let got = e.get(key).unwrap_or_else(|| {
                    panic!(
                        "{ctx}: field {key:?} is present on the wire but absent after \
                         re-encoding — the payload type no longer carries it"
                    )
                });
                assert_preserved(value, got, &format!("{ctx}.{key}"));
            }
        }
        (Value::Array(o), Value::Array(e)) => {
            assert_eq!(o.len(), e.len(), "{ctx}: array length changed");
            for (i, (a, b)) in o.iter().zip(e).enumerate() {
                assert_preserved(a, b, &format!("{ctx}[{i}]"));
            }
        }
        (a, b) => assert_eq!(a, b, "{ctx}: value changed passing through the type"),
    }
}

fn json_files(dir: &PathBuf) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|entry| entry.unwrap().path())
        .filter(|p| p.is_file())
        .collect();
    for path in &out {
        assert_eq!(
            path.extension().and_then(|v| v.to_str()),
            Some("json"),
            "{}: only .json fixtures belong in the corpus",
            path.display()
        );
    }
    out.sort();
    out
}

/// Decodes one fixture as its concrete payload type, asserts nothing was lost
/// through that type, and runs the payload's own semantic validation.
fn check_valid(path: &PathBuf) -> MessageKind {
    let name = path.file_name().unwrap().to_string_lossy().to_string();
    let bytes = fs::read(path).unwrap();
    let original: Value = serde_json::from_slice(&bytes)
        .unwrap_or_else(|e| panic!("{name}: fixture is not valid JSON: {e}"));

    let probe = Envelope::<Value>::from_json(&bytes)
        .unwrap_or_else(|e| panic!("{name}: envelope rejected: {e}"));

    macro_rules! typed {
        ($ty:ty, $data:ident => $semantics:block) => {{
            let envelope = Envelope::<$ty>::from_json(&bytes).unwrap_or_else(|e| {
                panic!(
                    "{name}: does not decode as {} — the wire contract changed: {e}",
                    stringify!($ty)
                )
            });
            let encoded: Value = serde_json::from_str(&envelope.to_json().unwrap()).unwrap();
            assert_preserved(&original, &encoded, &name);
            let round = Envelope::<$ty>::from_json(encoded.to_string().as_bytes())
                .unwrap_or_else(|e| panic!("{name}: re-encoded form no longer decodes: {e}"));
            assert_eq!(
                round.data, envelope.data,
                "{name}: payload is not stable across a round trip"
            );
            let $data = &envelope.data;
            $semantics
        }};
    }

    match probe.kind {
        MessageKind::TelemetryBatch => typed!(TelemetryBatch, batch => {
            batch.validate().unwrap_or_else(|e| panic!("{name}: batch cardinality: {e:?}"));
            for (i, sample) in batch.samples.iter().enumerate() {
                let report = sample.validate();
                assert!(
                    report.is_valid(),
                    "{name}: sample {i} ({}) invalid: {:?}",
                    sample.kind.as_str(),
                    report.invalid_fields
                );
            }
        }),
        MessageKind::ActuatorState => typed!(ActuatorState, _d => {}),
        MessageKind::DeviceEvents => typed!(DeviceEventBatch, events => {
            events.validate().unwrap_or_else(|e| panic!("{name}: {e:?}"));
        }),
        MessageKind::DeviceStatus => typed!(DeviceStatus, _d => {}),
        MessageKind::DeviceConfig => typed!(DeviceConfig, config => {
            config.validate().unwrap_or_else(|e| panic!("{name}: {e:?}"));
        }),
        MessageKind::DevicePolicy => typed!(OfflinePolicySet, set => {
            for policy in &set.policies {
                policy.validate().unwrap_or_else(|e| {
                    panic!("{name}: policy {} invalid: {e:?}", policy.plant_id.as_str())
                });
            }
        }),
        MessageKind::EdgeTime => typed!(EdgeTime, _d => {}),
        MessageKind::CommandWater => typed!(WaterCommand, cmd => {
            cmd.validate().unwrap_or_else(|e| panic!("{name}: {e:?}"));
        }),
        MessageKind::CommandTare => typed!(TareCommand, cmd => {
            cmd.validate().unwrap_or_else(|e| panic!("{name}: {e:?}"));
        }),
        MessageKind::CommandCalibrate => typed!(CalibrateCommand, cmd => {
            cmd.validate().unwrap_or_else(|e| panic!("{name}: {e:?}"));
        }),
        MessageKind::CommandResult => typed!(CommandResult, _d => {}),
        // `EventAck` carries no wire-level semantic rule: every `u64` is a
        // representable sequence. Whether a *particular* sequence may be
        // acted on is device state — "beyond what I have buffered" is not a
        // property of the bytes — so that check lives in the simulator
        // (`buffer::acknowledge`), not here.
        MessageKind::EventAck => typed!(EventAck, _d => {}),
    }
    probe.kind
}

#[test]
fn valid_fixtures_decode_as_their_concrete_payload_type() {
    let files = json_files(&root().join("valid"));
    assert!(files.len() >= 12, "the corpus lost fixtures");
    for path in &files {
        check_valid(path);
    }
}

/// Every message kind in protocol §3 must keep at least one wire example, so a
/// kind cannot quietly lose its only compatibility guard.
#[test]
fn valid_fixtures_cover_every_message_kind() {
    let covered: Vec<MessageKind> = json_files(&root().join("valid"))
        .iter()
        .map(check_valid)
        .collect();
    for kind in [
        MessageKind::TelemetryBatch,
        MessageKind::ActuatorState,
        MessageKind::DeviceEvents,
        MessageKind::DeviceStatus,
        MessageKind::DeviceConfig,
        MessageKind::DevicePolicy,
        MessageKind::EdgeTime,
        MessageKind::CommandWater,
        MessageKind::CommandTare,
        MessageKind::CommandCalibrate,
        MessageKind::CommandResult,
        MessageKind::EventAck,
    ] {
        assert!(covered.contains(&kind), "no valid fixture for {kind:?}");
    }
}

/// The failure class an invalid fixture must produce, named by its directory.
#[derive(Clone, Copy, Debug)]
enum Expected {
    EnvelopeMissingField,
    UnsupportedVersion,
    DeviceIdInvalid,
    BatchEmpty,
    SampleUnitMismatch,
    SampleOutOfRange,
    PolicyDoseAboveHardLimit,
    PolicyInvalidHysteresis,
    PolicyNonScalarControlKind,
    EventDuplicateId,
}

impl Expected {
    fn from_dir(name: &str) -> Self {
        match name {
            "envelope_missing_field" => Self::EnvelopeMissingField,
            "unsupported_version" => Self::UnsupportedVersion,
            "device_id_invalid" => Self::DeviceIdInvalid,
            "batch_empty" => Self::BatchEmpty,
            "sample_unit_mismatch" => Self::SampleUnitMismatch,
            "sample_out_of_range" => Self::SampleOutOfRange,
            "policy_dose_above_hard_limit" => Self::PolicyDoseAboveHardLimit,
            "policy_invalid_hysteresis" => Self::PolicyInvalidHysteresis,
            "policy_non_scalar_control_kind" => Self::PolicyNonScalarControlKind,
            "event_duplicate_id" => Self::EventDuplicateId,
            other => panic!(
                "invalid/{other}/ declares no expected failure variant. Every invalid \
                 fixture must state what it proves: add {other:?} to `Expected::from_dir` \
                 or move the fixture into an existing class."
            ),
        }
    }

    fn assert_rejected(self, bytes: &[u8], ctx: &str) {
        fn samples(bytes: &[u8], ctx: &str) -> Vec<String> {
            Envelope::<TelemetryBatch>::from_json(bytes)
                .unwrap_or_else(|e| panic!("{ctx}: envelope must still decode: {e}"))
                .data
                .samples
                .iter()
                .flat_map(|s| s.validate().invalid_fields)
                .collect()
        }
        fn policies(bytes: &[u8], ctx: &str) -> Vec<Result<(), PolicyError>> {
            Envelope::<OfflinePolicySet>::from_json(bytes)
                .unwrap_or_else(|e| panic!("{ctx}: envelope must still decode: {e}"))
                .data
                .policies
                .iter()
                .map(OfflinePolicy::validate)
                .collect()
        }
        fn decode_err(bytes: &[u8], ctx: &str) -> DecodeError {
            match Envelope::<Value>::from_json(bytes) {
                Err(e) => e,
                Ok(_) => panic!("{ctx}: decoded successfully but must be rejected"),
            }
        }

        match self {
            Self::EnvelopeMissingField => {
                let e = decode_err(bytes, ctx);
                assert!(matches!(e, DecodeError::Envelope), "{ctx}: got {e:?}");
            }
            Self::UnsupportedVersion => {
                let e = decode_err(bytes, ctx);
                assert!(
                    matches!(e, DecodeError::UnsupportedVersion),
                    "{ctx}: got {e:?}"
                );
            }
            Self::DeviceIdInvalid => {
                // §2 grammar violations surface as a field decode failure; the
                // literal must also be rejected by the grammar itself, which is
                // what makes topic injection impossible.
                let e = decode_err(bytes, ctx);
                assert!(matches!(e, DecodeError::Json(_)), "{ctx}: got {e:?}");
                let raw: Value = serde_json::from_slice(bytes).unwrap();
                let id = raw["device_id"].as_str().expect("device_id string");
                assert!(
                    DeviceId::parse(id).is_err(),
                    "{ctx}: {id:?} must violate the §2 grammar"
                );
            }
            Self::BatchEmpty => {
                let batch = Envelope::<TelemetryBatch>::from_json(bytes)
                    .unwrap_or_else(|e| panic!("{ctx}: envelope must still decode: {e}"));
                assert_eq!(batch.data.validate(), Err(BatchError::Empty), "{ctx}");
            }
            Self::SampleUnitMismatch => {
                let fields = samples(bytes, ctx);
                assert!(fields.iter().any(|f| f == "unit"), "{ctx}: got {fields:?}");
            }
            Self::SampleOutOfRange => {
                let fields = samples(bytes, ctx);
                assert!(fields.iter().any(|f| f == "value"), "{ctx}: got {fields:?}");
            }
            Self::PolicyDoseAboveHardLimit => {
                let got = policies(bytes, ctx);
                assert!(
                    got.contains(&Err(PolicyError::DoseAboveHardLimit)),
                    "{ctx}: got {got:?}"
                );
            }
            Self::PolicyInvalidHysteresis => {
                let got = policies(bytes, ctx);
                assert!(
                    got.contains(&Err(PolicyError::InvalidHysteresis)),
                    "{ctx}: got {got:?}"
                );
            }
            Self::PolicyNonScalarControlKind => {
                let got = policies(bytes, ctx);
                assert!(
                    got.contains(&Err(PolicyError::NonScalarControlKind)),
                    "{ctx}: got {got:?}"
                );
            }
            Self::EventDuplicateId => {
                let batch = Envelope::<DeviceEventBatch>::from_json(bytes)
                    .unwrap_or_else(|e| panic!("{ctx}: envelope must still decode: {e}"));
                assert_eq!(
                    batch.data.validate(),
                    Err(EventBatchError::DuplicateEventId),
                    "{ctx}"
                );
            }
        }
    }
}

#[test]
fn invalid_fixtures_fail_with_their_documented_variant() {
    let base = root().join("invalid");
    let mut classes = 0;
    let mut files = 0;
    for entry in fs::read_dir(&base).unwrap() {
        let dir = entry.unwrap().path();
        assert!(
            dir.is_dir(),
            "{}: invalid fixtures live in a directory naming their expected \
             failure variant, not loose in invalid/",
            dir.display()
        );
        let expected = Expected::from_dir(&dir.file_name().unwrap().to_string_lossy());
        let members = json_files(&dir);
        assert!(!members.is_empty(), "{}: no fixtures", dir.display());
        for path in &members {
            let ctx = format!(
                "{}/{}",
                dir.file_name().unwrap().to_string_lossy(),
                path.file_name().unwrap().to_string_lossy()
            );
            expected.assert_rejected(&fs::read(path).unwrap(), &ctx);
            files += 1;
        }
        classes += 1;
    }
    assert!(classes >= 10 && files >= 11, "the corpus lost fixtures");
}

/// An unrecognised measurement kind is stored and marked advisory rather than
/// rejected — forward compatibility is a wire guarantee (ADR-017, SAFETY-012).
#[test]
fn unknown_measurement_kind_is_accepted_and_advisory() {
    let bytes = fs::read(root().join("valid/telemetry-unknown.json")).unwrap();
    let batch = Envelope::<TelemetryBatch>::from_json(&bytes).unwrap();
    let sample = &batch.data.samples[0];
    assert!(matches!(sample.kind, MeasurementKind::Unknown(_)));
    assert!(!sample.kind.is_known());
    assert!(sample.advisory_only());
    assert!(batch.data.validate().is_ok());
}
