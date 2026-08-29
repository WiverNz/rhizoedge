//! Compact development log output.
//!
//! This is a separate binary because a process may install only one global
//! subscriber. The explicit in-memory writer is intentionally non-terminal,
//! which also proves redirected output remains free of ANSI escapes.

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
fn compact_output_is_single_line_correlation_first_and_plain_when_redirected() {
    let buffer = Buffer::default();
    init_tracing_with_writer(LogFormat::Compact, "info", buffer.clone())
        .expect("this process installs exactly one subscriber");

    let span = tracing::info_span!(target: "edge_controller::pipeline", "ingest", device_id = tracing::field::Empty);
    span.record("device_id", "plant-node-01");
    let entered = span.enter();
    tracing::info!(target: "edge_controller::pipeline", kind = "device.status", sequence = 7_u64, "device connected");
    drop(entered);

    let output = buffer.take();
    assert_eq!(
        output.lines().count(),
        1,
        "one event must occupy one line: {output}"
    );
    assert!(
        output.contains("INFO  EDGE  plant-node-01 device connected"),
        "component, correlation, and message must lead: {output}"
    );
    assert!(
        output.contains("kind=device.status"),
        "structured fields remain named: {output}"
    );
    assert!(
        output.contains("sequence=7"),
        "structured values remain visible: {output}"
    );
    assert!(
        !output.contains("source="),
        "normal INFO omits source locations: {output}"
    );
    assert!(
        !output.contains('\u{1b}'),
        "a non-terminal writer must not receive ANSI escapes: {output:?}"
    );
}
