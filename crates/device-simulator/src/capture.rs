//! Capturing what the simulator really publishes, and detecting drift.
//!
//! [ADR-008](../../../../docs/adr/008-shared-code-simulator-and-firmware.md)
//! §Risks names fixture rot: the corpus stops reflecting real messages, and it
//! quietly stops proving anything. `--capture-fixtures` writes one example of
//! every kind the simulator publishes, and the drift check compares that capture
//! against `test/fixtures/protocol/valid/`.
//!
//! # What is compared, and what is not
//!
//! **Field paths, not values.** `message_id`, `batch_id`, and every timestamp
//! differ on every run, and floating-point readings differ by noise; comparing
//! them would make the check fail constantly and therefore be turned off. What
//! must not drift is the *shape*.
//!
//! The rule is: every field path a captured payload emits must appear somewhere
//! in the corpus for that kind. Adding a field to a payload without adding it to
//! a fixture therefore fails. The converse is deliberately **not** required — a
//! fixture may contain a path the capture omits, because an absent optional
//! field is a documented option (`skip_serializing_if`) and several fixtures
//! exist precisely to pin the partial shapes.
//!
//! # A drift is a decision, not a fix
//!
//! The check fails; it never rewrites a fixture. `versioning-policy.md` makes
//! the corpus append-only precisely because the instinct on a failure is to edit
//! it, and editing it is how a wire-compatibility guarantee is lost.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use rhizo_mqtt_contract::MessageKind;

use crate::envelope::Publication;

/// Collects one example of each published kind.
#[derive(Debug, Default)]
pub struct FixtureCapture {
    examples: HashMap<String, String>,
}

impl FixtureCapture {
    /// An empty capture.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Offers a publication; the first of each kind is kept.
    ///
    /// The first rather than the last, so a capture is deterministic in what it
    /// contains regardless of how long the run lasted.
    pub fn offer(&mut self, publication: &Publication) {
        let kind = MessageKind::for_topic(&publication.topic);
        let name = file_stem(kind);
        self.examples
            .entry(name.to_owned())
            .or_insert_with(|| publication.payload.clone());
    }

    /// How many distinct kinds have been captured.
    #[must_use]
    pub fn len(&self) -> usize {
        self.examples.len()
    }

    /// Whether anything has been captured.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.examples.is_empty()
    }

    /// The captured payloads, keyed by file stem.
    #[must_use]
    pub fn examples(&self) -> &HashMap<String, String> {
        &self.examples
    }

    /// Writes one file per captured kind into a directory.
    ///
    /// # Errors
    ///
    /// Returns the first write failure.
    pub fn write_to(&self, directory: &Path) -> std::io::Result<Vec<PathBuf>> {
        std::fs::create_dir_all(directory)?;
        let mut written = Vec::new();
        for (name, payload) in &self.examples {
            let path = directory.join(format!("{name}.json"));
            // Pretty-printed: a capture is read by a person deciding whether a
            // drift is intended, and a single-line payload is unreadable.
            let value: serde_json::Value = serde_json::from_str(payload)
                .unwrap_or_else(|_| serde_json::Value::String(payload.clone()));
            std::fs::write(&path, serde_json::to_vec_pretty(&value)?)?;
            written.push(path);
        }
        written.sort();
        Ok(written)
    }
}

/// The file stem a captured kind is written under.
#[must_use]
pub const fn file_stem(kind: MessageKind) -> &'static str {
    match kind {
        MessageKind::TelemetryBatch => "telemetry-batch",
        MessageKind::ActuatorState => "actuator",
        MessageKind::DeviceEvents => "events",
        MessageKind::DeviceStatus => "status",
        MessageKind::DeviceConfig => "config",
        MessageKind::DevicePolicy => "policy",
        MessageKind::EdgeTime => "edge-time",
        MessageKind::CommandWater => "command-water",
        MessageKind::CommandTare => "command-tare",
        MessageKind::CommandCalibrate => "command-calibrate",
        MessageKind::CommandResult => "command-result",
        MessageKind::CommandResultAck => "command-result-ack",
        MessageKind::EventAck => "event-ack",
    }
}

/// Every field path in a JSON value, with array indices collapsed to `[]`.
///
/// Collapsing indices is what makes the comparison about shape: a batch with
/// five samples and a batch with two have the same paths, and a *new field* on a
/// sample has a new one.
#[must_use]
pub fn field_paths(value: &serde_json::Value) -> BTreeSet<String> {
    let mut paths = BTreeSet::new();
    walk(value, String::new(), &mut paths);
    paths
}

fn walk(value: &serde_json::Value, prefix: String, paths: &mut BTreeSet<String>) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, child) in map {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                paths.insert(path.clone());
                walk(child, path, paths);
            }
        }
        serde_json::Value::Array(items) => {
            let path = format!("{prefix}[]");
            paths.insert(path.clone());
            for item in items {
                walk(item, path.clone(), paths);
            }
        }
        // A scalar's path was inserted by its parent; its value is deliberately
        // not part of the comparison.
        _ => {}
    }
}

/// A path a captured message emits that no fixture of its kind contains.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Drift {
    /// The captured kind, by file stem.
    pub kind: String,
    /// The field path with no corpus coverage.
    pub path: String,
}

/// Compares a capture against a corpus, returning every uncovered path.
///
/// `corpus` maps a file stem to the union of paths across every fixture of that
/// kind.
#[must_use]
pub fn drift(capture: &FixtureCapture, corpus: &HashMap<String, BTreeSet<String>>) -> Vec<Drift> {
    let mut drifts = Vec::new();
    for (kind, payload) in &capture.examples {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(payload) else {
            drifts.push(Drift {
                kind: kind.clone(),
                path: String::from("<not valid JSON>"),
            });
            continue;
        };
        let Some(covered) = corpus.get(kind) else {
            drifts.push(Drift {
                kind: kind.clone(),
                path: String::from("<no fixture of this kind exists>"),
            });
            continue;
        };
        for path in field_paths(&value) {
            if !covered.contains(&path) {
                drifts.push(Drift {
                    kind: kind.clone(),
                    path,
                });
            }
        }
    }
    drifts.sort_by(|a, b| (&a.kind, &a.path).cmp(&(&b.kind, &b.path)));
    drifts
}

#[cfg(test)]
mod tests {
    use super::*;
    use rhizo_mqtt_contract::{DeviceId, Topic};

    fn publication(topic: Topic, payload: &str) -> Publication {
        Publication::new(topic, payload.to_owned())
    }

    fn id() -> DeviceId {
        DeviceId::parse("plant-node-01").unwrap()
    }

    #[test]
    fn paths_collapse_array_indices_and_ignore_values() {
        let one = serde_json::json!({ "data": { "samples": [ { "kind": "soil_moisture", "value": 31.7 } ] } });
        let many = serde_json::json!({
            "data": { "samples": [
                { "kind": "soil_moisture", "value": 0.0 },
                { "kind": "leak_state", "value": false },
            ] }
        });
        assert_eq!(
            field_paths(&one),
            field_paths(&many),
            "the number of samples and their values are not shape"
        );
        assert!(field_paths(&one).contains("data.samples[].kind"));
    }

    #[test]
    fn a_new_field_is_a_new_path() {
        let before = field_paths(&serde_json::json!({ "data": { "a": 1 } }));
        let after = field_paths(&serde_json::json!({ "data": { "a": 1, "b": 2 } }));
        assert!(after.contains("data.b"));
        assert!(!before.contains("data.b"));
    }

    #[test]
    fn the_first_example_of_each_kind_is_kept() {
        let mut capture = FixtureCapture::new();
        assert!(capture.is_empty());
        capture.offer(&publication(Topic::Status(id()), r#"{"first":true}"#));
        capture.offer(&publication(Topic::Status(id()), r#"{"second":true}"#));
        capture.offer(&publication(Topic::Telemetry(id()), r#"{"batch":true}"#));
        assert_eq!(capture.len(), 2);
        assert_eq!(capture.examples()["status"], r#"{"first":true}"#);
    }

    #[test]
    fn a_capture_covered_by_the_corpus_reports_no_drift() {
        let mut capture = FixtureCapture::new();
        capture.offer(&publication(
            Topic::Status(id()),
            r#"{"v":1,"data":{"status":"online"}}"#,
        ));
        let corpus = HashMap::from([(
            String::from("status"),
            field_paths(&serde_json::json!({
                "v": 1,
                "data": { "status": "online", "reason": "shutdown" }
            })),
        )]);
        assert!(
            drift(&capture, &corpus).is_empty(),
            "a fixture may cover more than the capture emits"
        );
    }

    #[test]
    fn a_field_the_corpus_does_not_know_about_is_reported() {
        let mut capture = FixtureCapture::new();
        capture.offer(&publication(
            Topic::Status(id()),
            r#"{"v":1,"data":{"status":"online","new_field":42}}"#,
        ));
        let corpus = HashMap::from([(
            String::from("status"),
            field_paths(&serde_json::json!({ "v": 1, "data": { "status": "online" } })),
        )]);
        assert_eq!(
            drift(&capture, &corpus),
            vec![Drift {
                kind: String::from("status"),
                path: String::from("data.new_field"),
            }]
        );
    }

    #[test]
    fn a_kind_with_no_fixture_at_all_is_reported() {
        let mut capture = FixtureCapture::new();
        capture.offer(&publication(Topic::Events(id()), r#"{"v":1}"#));
        let drifts = drift(&capture, &HashMap::new());
        assert_eq!(drifts.len(), 1);
        assert_eq!(drifts[0].kind, "events");
        assert!(drifts[0].path.contains("no fixture"));
    }

    #[test]
    fn a_capture_writes_one_readable_file_per_kind() {
        let mut directory = std::env::temp_dir();
        directory.push(format!("rhizo-capture-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);

        let mut capture = FixtureCapture::new();
        capture.offer(&publication(
            Topic::Status(id()),
            r#"{"v":1,"data":{"status":"online"}}"#,
        ));
        capture.offer(&publication(
            Topic::Telemetry(id()),
            r#"{"v":1,"data":{"samples":[]}}"#,
        ));
        let written = capture.write_to(&directory).unwrap();
        assert_eq!(written.len(), 2);
        assert!(written.iter().any(|p| p.ends_with("status.json")));
        assert!(written.iter().any(|p| p.ends_with("telemetry-batch.json")));

        let text = std::fs::read_to_string(directory.join("status.json")).unwrap();
        assert!(text.contains('\n'), "a capture is read by a person");
        serde_json::from_str::<serde_json::Value>(&text).unwrap();
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn every_message_kind_has_a_distinct_file_stem() {
        let kinds = [
            MessageKind::EventAck,
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
            MessageKind::CommandResultAck,
        ];
        let stems: BTreeSet<_> = kinds.iter().map(|k| file_stem(*k)).collect();
        assert_eq!(stems.len(), kinds.len());
    }
}
