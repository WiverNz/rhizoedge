//! Connectivity state; edge-observed receipt time is authoritative.
/// Complete registry connectivity model. API names retain the established vocabulary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum State {
    /// Reachable after an accepted online status.
    Online,
    /// Intentionally absent inside its bounded edge-derived window.
    SleepingExpected {
        /// End of the expected window, in edge-clock milliseconds.
        expected_until: i64,
    },
    /// Absent without valid intent, or beyond the expected window.
    OfflineUnexpectedly,
    /// Reachable while buffered history is being reconciled.
    Reconciling,
}
impl State {
    /// Existing API representation plus the battery-aware sleeping value.
    pub const fn api_name(self) -> &'static str {
        match self {
            Self::Online => "connected",
            Self::SleepingExpected { .. } => "sleeping",
            Self::OfflineUnexpectedly => "isolated",
            Self::Reconciling => "reconciling",
        }
    }
    /// The expected wake instant, which exists only while a window is open.
    ///
    /// Every other state answers `None`, so a window left in the row by a device
    /// that has since woken, been retired from battery mode, or gone overdue can
    /// never be reported as if it were still meaningful
    /// (`http-api-boundaries.md` §2.3).
    pub const fn expected_wake_at(self) -> Option<i64> {
        match self {
            Self::SleepingExpected { expected_until } => Some(expected_until),
            Self::Online | Self::OfflineUnexpectedly | Self::Reconciling => None,
        }
    }
}
/// Derives the reported state from the bounded SQLite projection **and the
/// edge's own clock**.
///
/// The deadline is re-checked on every read, which is what makes SAFETY-021 hold
/// without depending on a writer. The liveness timer still performs the durable
/// transition, its event, and its counter — but if that timer is late, wedged,
/// or has not run since the process started, an overdue sleeper is *still*
/// reported as `isolated`, because "asleep" is computed here rather than
/// remembered. A stored state needs a writer, and a writer that fails leaves a
/// device permanently asleep, which is the precise failure the invariant exists
/// to prevent.
///
/// A `sleeping` row missing either half of its window is inconsistent, and
/// inconsistency resolves to *absent*, never to a reachable state (SAFETY-012).
pub fn from_projection(
    mode: &str,
    expected_wake_at: Option<i64>,
    overdue_at: Option<i64>,
    now_ms: i64,
) -> State {
    match mode {
        "connected" => State::Online,
        "sleeping" => match (expected_wake_at, overdue_at) {
            (Some(expected_until), Some(deadline)) if now_ms < deadline => {
                State::SleepingExpected { expected_until }
            }
            _ => State::OfflineUnexpectedly,
        },
        "isolated" => State::OfflineUnexpectedly,
        _ => State::Reconciling,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn edge_liveness_overrides_advisory_report() {
        assert_eq!(State::Online.api_name(), "connected");
        assert_eq!(
            State::SleepingExpected { expected_until: 42 }.api_name(),
            "sleeping"
        );
        assert_eq!(State::OfflineUnexpectedly.api_name(), "isolated");
        assert_eq!(State::Reconciling.api_name(), "reconciling");
        assert_eq!(
            from_projection("sleeping", Some(900), Some(1_800), 0),
            State::SleepingExpected {
                expected_until: 900
            }
        );
        assert_eq!(
            from_projection("isolated", Some(900), Some(1_800), 0),
            State::OfflineUnexpectedly
        );
        assert_eq!(
            from_projection("reconciling", None, None, 0),
            State::Reconciling
        );
    }
    /// SAFETY-021 read-side: the deadline is what ends the window, not a writer.
    #[test]
    fn safety_021_overdue_sleeper_becomes_isolated() {
        let open = from_projection("sleeping", Some(900), Some(1_800), 1_799);
        assert_eq!(
            open,
            State::SleepingExpected {
                expected_until: 900
            }
        );
        assert_eq!(open.expected_wake_at(), Some(900));
        // The row is untouched; only the clock moved past `overdue_at`.
        let overdue = from_projection("sleeping", Some(900), Some(1_800), 1_800);
        assert_eq!(overdue, State::OfflineUnexpectedly);
        assert_eq!(overdue.api_name(), "isolated");
        assert_eq!(
            overdue.expected_wake_at(),
            None,
            "an overdue device must not advertise a wake it already missed"
        );
    }
    /// An inconsistent `sleeping` row is absent, never reachable (SAFETY-012).
    #[test]
    fn safety_021_an_incomplete_sleep_window_is_never_reachable() {
        for row in [
            from_projection("sleeping", None, None, 0),
            from_projection("sleeping", Some(900), None, 0),
            from_projection("sleeping", None, Some(1_800), 0),
        ] {
            assert_eq!(row, State::OfflineUnexpectedly, "{row:?}");
        }
    }
    /// Negative control: only `SleepingExpected` may carry a wake instant.
    #[test]
    fn no_other_state_reports_an_expected_wake() {
        assert_eq!(State::Online.expected_wake_at(), None);
        assert_eq!(State::OfflineUnexpectedly.expected_wake_at(), None);
        assert_eq!(State::Reconciling.expected_wake_at(), None);
    }
}
