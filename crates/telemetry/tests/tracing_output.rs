//! Asserts on the bytes a real subscriber writes (M0-006).
//!
//! These live in an integration test because `init_tracing_with_writer`
//! installs a *global* subscriber and a process may install only one. This
//! binary therefore initialises exactly once, and every test serialises on the
//! same buffer so their output cannot interleave.
//!
//! The subject is the JSON format, because that is the production one and the
//! one other tools depend on the shape of. Pretty output is exercised in
//! `pretty_output.rs`, which needs its own process for the same reason.

// A panic in a test is a failed assertion, not an unhandled failure.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use rhizo_telemetry::{LogFormat, init_tracing_with_writer};

/// An in-memory `MakeWriter` so the test can read what the subscriber wrote.
#[derive(Clone, Default)]
struct Buffer(Arc<Mutex<Vec<u8>>>);

impl Buffer {
    fn take(&self) -> String {
        let mut guard = self.0.lock().unwrap();
        let out = String::from_utf8(guard.clone()).unwrap();
        guard.clear();
        out
    }
}

impl io::Write for Buffer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Buffer {
    type Writer = Self;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// The single global subscriber for this test binary, plus the lock that stops
/// two tests writing into the buffer at once.
struct Harness {
    buffer: Buffer,
    serial: Mutex<()>,
}

fn harness() -> &'static Harness {
    static HARNESS: OnceLock<Harness> = OnceLock::new();
    HARNESS.get_or_init(|| {
        let buffer = Buffer::default();
        // The directive is deliberately explicit rather than read from
        // `RUST_LOG`: `init_tracing_with_writer` ignores the environment, so
        // these assertions hold whatever the developer's shell has set.
        init_tracing_with_writer(LogFormat::Json, "info,quiet_target=warn", buffer.clone())
            .expect("this process installs exactly one subscriber");
        Harness {
            buffer,
            serial: Mutex::new(()),
        }
    })
}

/// Takes the serialising lock and clears anything a previous test left behind.
fn session() -> (MutexGuard<'static, ()>, &'static Buffer) {
    let h = harness();
    let guard = h.serial.lock().unwrap_or_else(|p| p.into_inner());
    h.buffer.take();
    (guard, &h.buffer)
}

#[test]
fn json_output_is_parseable_and_carries_level_target_and_fields() {
    let (_guard, buf) = session();

    tracing::info!(
        plant_id = "monstera-01",
        requested_ml = 45_u32,
        mode = "auto",
        "issuing dose"
    );

    let out = buf.take();
    let line = out.lines().next().expect("an event was emitted");
    let v: serde_json::Value = serde_json::from_str(line).expect("each line is one JSON object");

    assert_eq!(v["level"], "INFO");
    assert_eq!(v["target"], "tracing_output");
    assert_eq!(v["message"], "issuing dose");

    // The point of the whole exercise: correlation identifiers are their own
    // fields, so a log can be filtered by plant rather than grepped.
    assert_eq!(v["plant_id"], "monstera-01");
    assert_eq!(v["requested_ml"], 45);
    assert_eq!(v["mode"], "auto");

    // ...and are NOT interpolated into the message.
    assert!(
        !v["message"].as_str().unwrap().contains("monstera-01"),
        "message must not carry the identifier: {line}"
    );

    assert!(
        v.get("timestamp").is_some(),
        "events are timestamped: {line}"
    );
}

#[test]
fn a_span_supplies_correlation_fields_to_events_inside_it() {
    let (_guard, buf) = session();

    let span = tracing::info_span!("handle_message", device_id = "plant-node-01");
    let entered = span.enter();
    tracing::info!(kind = "telemetry.soil", "message decoded");
    drop(entered);

    let out = buf.take();
    let line = out.lines().next().expect("an event was emitted");
    let v: serde_json::Value = serde_json::from_str(line).unwrap();

    assert_eq!(v["span"]["device_id"], "plant-node-01");
    assert_eq!(v["span"]["name"], "handle_message");
    assert_eq!(v["kind"], "telemetry.soil");
}

#[test]
fn the_configured_filter_suppresses_events_below_its_level() {
    let (_guard, buf) = session();

    tracing::debug!("this is below the configured `info` threshold");
    assert_eq!(buf.take(), "", "debug must be filtered out at info");

    tracing::warn!("this is above it");
    assert!(
        buf.take().contains("above it"),
        "warn must pass the info filter"
    );
}

#[test]
fn a_per_target_directive_overrides_the_global_level() {
    let (_guard, buf) = session();

    // The directive is `info,quiet_target=warn`.
    tracing::info!(target: "quiet_target", "suppressed");
    assert_eq!(buf.take(), "", "the per-target directive raises the bar");

    tracing::warn!(target: "quiet_target", "not suppressed");
    assert!(buf.take().contains("not suppressed"));
}
