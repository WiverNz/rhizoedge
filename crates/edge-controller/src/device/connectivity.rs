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
}
/// Rehydrates the typed model from the bounded SQLite projection.
pub fn from_projection(mode: &str, expected_wake_at: Option<i64>) -> State {
    match (mode, expected_wake_at) {
        ("connected", _) => State::Online,
        ("sleeping", Some(expected_until)) => State::SleepingExpected { expected_until },
        ("isolated", _) => State::OfflineUnexpectedly,
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
            from_projection("sleeping", Some(42)),
            State::SleepingExpected { expected_until: 42 }
        );
        assert_eq!(from_projection("sleeping", None), State::Reconciling);
    }
}
