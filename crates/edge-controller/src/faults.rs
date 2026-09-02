//! Process-boundary crash hooks for the M8 scenario suite (M8-010, M8-016).
//!
//! # Why these exist at all
//!
//! SAFETY-010 says an edge restart cannot replay a completed command, and
//! SCEN-102 says an edge restart midway through a replay loses nothing and
//! duplicates nothing. Both are statements about a process that dies at a
//! *specific* instant — after a publish reached the socket but before the row
//! recording it committed, or after one replay batch committed but before the
//! next. Racing an external `docker kill` against those windows produces a test
//! that passes for the wrong reason most of the time, so M8-010's issue notes
//! ask for a fault hook in the edge instead.
//!
//! # Why they are a compile-time feature and not a runtime flag
//!
//! The hook is a [`std::process::exit`] on the actuation path. An environment
//! variable is something a deployment can acquire by accident — a stray line in
//! a shared `.env`, a copied Compose fragment — and the failure mode is an edge
//! that vanishes mid-dose. With `e2e-faults` off, [`armed`] is a `const false`
//! that the optimiser deletes, and there is no marker path, no filesystem
//! access, and no exit in the binary at all.
//!
//! Both conditions must hold for a hook to fire: the feature compiled in, and
//! the scenario runner having planted a one-shot marker file. The marker is
//! removed as it is consumed, so one armed fault kills the process once.

/// Marker armed before a water command is published, consumed after the publish
/// reaches the broker and before `commands.status` records it (SCEN-051).
pub const EXIT_AFTER_COMMAND_PUBLISH: &str = "/var/lib/rhizo/fault-exit-after-command-publish";

/// Marker armed before a device replays its buffered history, consumed after an
/// incomplete batch commits (SCEN-102).
pub const EXIT_MID_REPLAY: &str = "/var/lib/rhizo/fault-exit-mid-replay";

/// Whether this build can inject process-boundary crashes at all.
///
/// Reported by `GET /api/v1/overview` so the scenario runner can refuse to run
/// against an image that would silently skip the scenarios needing it.
#[must_use]
pub const fn available() -> bool {
    cfg!(feature = "e2e-faults")
}

/// Whether `marker` is armed, consuming it if so.
///
/// Returns `false` unconditionally without the `e2e-faults` feature.
#[must_use]
pub fn armed(marker: &str) -> bool {
    if !available() {
        let _ = marker;
        return false;
    }
    #[cfg(feature = "e2e-faults")]
    {
        let path = std::path::Path::new(marker);
        if path.exists() {
            // Removed before the exit, not after: a marker that survived the
            // crash would re-arm on the next boot and kill the restarted edge
            // in the same place, which is an infinite loop rather than a test.
            let _ = std::fs::remove_file(path);
            return true;
        }
        false
    }
    #[cfg(not(feature = "e2e-faults"))]
    false
}

/// The exit code a fault hook terminates with.
///
/// Distinct from every code the edge produces on its own, so a scenario can
/// tell "the fault fired" from "the process failed".
pub const FAULT_EXIT_CODE: i32 = 86;

#[cfg(test)]
mod tests {
    use super::*;

    /// The feature is off by default, so an ordinary build — including every
    /// unit-test build that does not opt in — cannot inject a crash.
    #[test]
    fn a_default_build_has_no_armed_fault() {
        if !available() {
            assert!(!armed(EXIT_AFTER_COMMAND_PUBLISH));
            assert!(!armed(EXIT_MID_REPLAY));
        }
    }

    /// Both markers live under the edge's own data directory, which is the one
    /// path a Compose service is guaranteed to share with the runner.
    #[test]
    fn both_markers_are_under_the_edge_data_directory() {
        assert!(EXIT_AFTER_COMMAND_PUBLISH.starts_with("/var/lib/rhizo/"));
        assert!(EXIT_MID_REPLAY.starts_with("/var/lib/rhizo/"));
    }
}
