//! Pretty log output (M0-006).
//!
//! Its own binary because a process may install only one global subscriber and
//! `tracing_output.rs` installs the JSON one.
//!
//! The assertions here are deliberately loose. Pretty output exists to be read
//! by a person at a terminal; pinning its exact layout would turn a cosmetic
//! upstream change into a failing build without telling us anything true about
//! the system. What is asserted is what a developer actually relies on: the
//! level, the message, and every structured field are all present and legible.

// A panic in a test is a failed assertion, not an unhandled failure.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io;
use std::sync::{Arc, Mutex};

use rhizo_telemetry::{LogFormat, init_tracing_with_writer};

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

#[test]
fn pretty_output_is_human_readable_and_keeps_fields_visible() {
    let buf = Buffer::default();
    init_tracing_with_writer(LogFormat::Pretty, "info", buf.clone())
        .expect("this process installs exactly one subscriber");

    tracing::info!(
        plant_id = "monstera-01",
        requested_ml = 45_u32,
        "issuing dose"
    );

    let out = buf.take();
    assert!(!out.is_empty(), "pretty output must not be empty");

    // Not JSON — this is the format a person reads.
    assert!(
        serde_json::from_str::<serde_json::Value>(out.lines().next().unwrap()).is_err(),
        "pretty output should not be JSON: {out}"
    );

    assert!(out.contains("INFO"), "level must be visible: {out}");
    assert!(
        out.contains("issuing dose"),
        "message must be present: {out}"
    );
    assert!(out.contains("plant_id"), "field names stay visible: {out}");
    assert!(
        out.contains("monstera-01"),
        "field values stay visible: {out}"
    );
    assert!(
        out.contains("requested_ml"),
        "every field is rendered: {out}"
    );
    assert!(out.contains("45"), "every field value is rendered: {out}");
}
