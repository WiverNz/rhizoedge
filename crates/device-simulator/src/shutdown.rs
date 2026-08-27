//! Graceful shutdown.
//!
//! A device that vanishes without warning is a *tested* case — that is what the
//! LWT is for — but it must not be the only case. On a deliberate stop the
//! simulator publishes `status: offline` with `reason: "shutdown"` (protocol
//! §5.6) and disconnects cleanly, so an operator can tell "I stopped it" from
//! "it died".

use tokio::signal;

/// The signal that ended the run.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Signal {
    /// An interactive interrupt.
    Interrupt,
    /// A termination request from a supervisor: SIGTERM on Unix, a console
    /// close or break on Windows.
    Terminate,
    /// `--duration` elapsed.
    DurationElapsed,
}

impl Signal {
    /// The `reason` reported in the final `device.status`.
    #[must_use]
    pub const fn reason(self) -> &'static str {
        // All three are deliberate stops. `connection_lost` belongs to the LWT
        // and must never be published by a process that is still running.
        "shutdown"
    }
}

/// Resolves when the process is asked to stop.
///
/// # Errors
///
/// Returns an error when a signal handler cannot be installed, which is a
/// startup failure rather than something to retry.
pub async fn wait() -> std::io::Result<Signal> {
    #[cfg(unix)]
    {
        use signal::unix::{SignalKind, signal as unix_signal};
        let mut term = unix_signal(SignalKind::terminate())?;
        tokio::select! {
            r = signal::ctrl_c() => r.map(|()| Signal::Interrupt),
            _ = term.recv() => Ok(Signal::Terminate),
        }
    }
    #[cfg(windows)]
    {
        use signal::windows::{ctrl_break, ctrl_close, ctrl_shutdown};
        let mut brk = ctrl_break()?;
        let mut close = ctrl_close()?;
        let mut down = ctrl_shutdown()?;
        tokio::select! {
            r = signal::ctrl_c() => r.map(|()| Signal::Interrupt),
            _ = brk.recv() => Ok(Signal::Terminate),
            _ = close.recv() => Ok(Signal::Terminate),
            _ = down.recv() => Ok(Signal::Terminate),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_deliberate_stop_reports_shutdown_not_connection_lost() {
        for s in [
            Signal::Interrupt,
            Signal::Terminate,
            Signal::DurationElapsed,
        ] {
            assert_eq!(s.reason(), "shutdown");
        }
    }
}
