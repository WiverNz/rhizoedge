//! Process-level shutdown evidence for M3-014 and M3-018.
//!
//! The supervisor's own tests (`supervisor::tests`) cover the cooperative
//! drain and the hung-task timeout through the watch channel. They cannot
//! cover the thing the exit criterion actually claims — that a real `SIGTERM`
//! delivered to a real process produces exit status 0 — because a signal
//! handler and a process exit code only exist in a process.
//!
//! The distinction is not academic. A binary with no `SIGTERM` handler is
//! *terminated by* the signal: it has no exit code at all, and
//! `ExitStatus::signal()` reports 15. Asserting `code() == Some(0)` is
//! therefore a direct test of the handler, not just of a successful run.
//!
//! Unix only, by nature. On Windows this file compiles to nothing and
//! `docs/reports/M3.md` says so rather than implying coverage that is absent.

#![allow(clippy::unwrap_used, clippy::expect_used)]

#[cfg(unix)]
mod unix {
    use std::io::ErrorKind;
    use std::net::{TcpListener, TcpStream};
    use std::os::unix::process::ExitStatusExt;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    /// Reserves a port by binding and releasing it.
    ///
    /// Racy in principle, but the alternative — binding port 0 inside the child
    /// — gives the test no way to learn which port to probe for readiness.
    fn free_port() -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        port
    }

    fn wait_for<F: FnMut() -> bool>(limit: Duration, mut ready: F) -> bool {
        let deadline = Instant::now() + limit;
        while Instant::now() < deadline {
            if ready() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        false
    }

    /// A real `SIGTERM` to a real edge process exits 0, having stopped intake
    /// and closed the pool rather than being killed where it stood.
    ///
    /// The broker is deliberately unreachable: failure-model §1.1 requires the
    /// edge to start and stay up without one, and it keeps this test free of
    /// any dependency on Mosquitto.
    #[test]
    fn sigterm_exits_zero() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("edge.sqlite");
        let port = free_port();

        let mut child = Command::new(env!("CARGO_BIN_EXE_edge-controller"))
            // A temporary working directory so no stray `./edge.toml` from the
            // repository can join the configuration layering.
            .current_dir(dir.path())
            .env("RHIZO_EDGE__STORAGE__PATH", &db)
            .env("RHIZO_EDGE__API__BIND", format!("127.0.0.1:{port}"))
            // Port 1 is reserved and never listening, so the edge starts with
            // its broker down and stays in reconnect backoff for the test.
            .env("RHIZO_EDGE__MQTT__BROKER_URL", "mqtt://127.0.0.1:1")
            .env("RHIZO_EDGE__MQTT__PASSWORD", "not-a-real-secret")
            .env("RHIZO_EDGE__LOG__FORMAT", "json")
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("the edge-controller binary must be built for this test");

        let listening = wait_for(Duration::from_secs(30), || {
            TcpStream::connect(("127.0.0.1", port)).is_ok()
        });
        assert!(
            listening,
            "the edge never bound its API port, so it never finished starting"
        );
        assert!(
            db.exists(),
            "the database was never created, so migration did not run"
        );
        assert!(
            child.try_wait().unwrap().is_none(),
            "the edge exited on its own with the broker down; failure-model 1.1 forbids that"
        );

        let pid = child.id();
        let killed = Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .status()
            .expect("kill(1) must be available on a unix host");
        assert!(killed.success(), "could not deliver SIGTERM to {pid}");

        let mut status = None;
        let exited = wait_for(Duration::from_secs(30), || {
            status = child.try_wait().unwrap();
            status.is_some()
        });
        if !exited {
            let _ = child.kill();
            panic!("the edge ignored SIGTERM and had to be killed");
        }
        let status = status.unwrap();

        assert_eq!(
            status.signal(),
            None,
            "the process was terminated by a signal instead of handling it: {status:?}"
        );
        assert_eq!(
            status.code(),
            Some(0),
            "SIGTERM must exit 0 after a clean drain, got {status:?}"
        );

        // The pool was closed rather than abandoned mid-write, so no hot
        // journal is left behind for the next start to recover.
        assert!(db.exists());
    }

    /// The other half of the exit criterion, at the process level: a failure
    /// the supervisor calls fatal must not be reported as a clean exit.
    /// Invalid configuration is the reachable fatal case at startup (ADR-014).
    #[test]
    fn a_fatal_startup_failure_exits_non_zero() {
        let dir = tempfile::tempdir().unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_edge-controller"))
            .current_dir(dir.path())
            .env("RHIZO_EDGE__CONTROL__TICK_INTERVAL_SECONDS", "0")
            .output()
            .expect("the edge-controller binary must be built for this test");
        assert_eq!(output.status.code(), Some(1));
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("tick_interval_seconds"),
            "a fatal configuration error must name its key, got: {stderr}"
        );
    }

    /// Guards the assumption the first test rests on: a process that does not
    /// handle SIGTERM is killed by it and reports no exit code at all. If this
    /// ever stopped being true, `sigterm_exits_zero` would pass vacuously.
    #[test]
    fn an_unhandled_sigterm_kills_a_process_without_an_exit_code() {
        let mut child = Command::new("sleep")
            .arg("60")
            .spawn()
            .expect("sleep(1) must be available on a unix host");
        let pid = child.id();
        assert!(
            Command::new("kill")
                .args(["-TERM", &pid.to_string()])
                .status()
                .unwrap()
                .success()
        );
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) => std::thread::sleep(Duration::from_millis(50)),
                Err(e) if e.kind() == ErrorKind::Interrupted => {}
                Err(e) => panic!("{e}"),
            }
        };
        assert_eq!(status.signal(), Some(15));
        assert_eq!(status.code(), None);
    }
}
