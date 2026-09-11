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
/// The `devices.connectivity_mode` vocabulary, as it is stored.
///
/// A string column becomes a *type* before anything decides on it, because a
/// `match` on `&str` can never be exhaustive: it needs a `_` arm, and that arm
/// is where "a value we do not recognise" quietly acquires a meaning nobody
/// chose. This one used to resolve an unrecognised mode to `Reconciling` — a
/// **reachable** state — directly against the rule the derivation below states,
/// and against SAFETY-012.
///
/// With the parse split out, [`from_projection`]'s match is exhaustive and
/// carries no catch-all: a mode added to the schema fails to compile until
/// someone decides what it means for watering.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StoredMode {
    /// The device answered.
    Connected,
    /// The device announced a bounded sleep.
    Sleeping,
    /// The device is known to be away.
    Isolated,
    /// The device is reachable and still replaying its buffer.
    Reconciling,
    /// Anything else the column holds.
    ///
    /// Unreachable through today's writers, which emit exactly the four above.
    /// It exists because "unreachable through today's writers" is a claim about
    /// code that can change, and the cost of being wrong about it is a device
    /// that is reported as reachable when nothing knows where it is.
    Unrecognised,
}

impl StoredMode {
    /// The one place a stored string becomes a mode.
    fn parse(raw: &str) -> Self {
        match raw {
            "connected" => Self::Connected,
            "sleeping" => Self::Sleeping,
            "isolated" => Self::Isolated,
            "reconciling" => Self::Reconciling,
            _ => Self::Unrecognised,
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
/// **Inconsistency resolves to absent, never to a reachable state**
/// (SAFETY-012). A `sleeping` row missing either half of its window is
/// inconsistent, and so is a mode this build does not recognise: both answer
/// [`State::OfflineUnexpectedly`]. Absent is the answer an operator can act on
/// — it says a device needs attention — where `reconciling` says the opposite,
/// that something transient is already resolving itself.
pub fn from_projection(
    mode: &str,
    expected_wake_at: Option<i64>,
    overdue_at: Option<i64>,
    now_ms: i64,
) -> State {
    match StoredMode::parse(mode) {
        StoredMode::Connected => State::Online,
        // Spelled out rather than closed with a `_` arm. A guard is not
        // exhaustiveness — `if now_ms < deadline` would still need a fallback —
        // so the completeness question and the deadline question are asked
        // separately, and neither is answered by a wildcard.
        StoredMode::Sleeping => match (expected_wake_at, overdue_at) {
            (Some(expected_until), Some(deadline)) => {
                if now_ms < deadline {
                    State::SleepingExpected { expected_until }
                } else {
                    State::OfflineUnexpectedly
                }
            }
            (Some(_), None) | (None, Some(_)) | (None, None) => State::OfflineUnexpectedly,
        },
        StoredMode::Isolated | StoredMode::Unrecognised => State::OfflineUnexpectedly,
        StoredMode::Reconciling => State::Reconciling,
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
    /// **SAFETY-012, structurally.** A mode this build does not recognise is
    /// *absent*, not reachable.
    ///
    /// The catch-all used to answer `Reconciling`, which reads to an operator
    /// as "reachable, and already sorting itself out" — the opposite of what an
    /// unrecognised value warrants. Nothing writes such a value today; that is
    /// a claim about code that can change, and the cost of being wrong is a
    /// device reported as reachable when nothing knows where it is.
    #[test]
    fn safety_012_an_unrecognised_mode_is_absent_not_reachable() {
        for mode in [
            "",
            "online",
            "offline",
            "asleep",
            "CONNECTED",
            "reconciling ",
        ] {
            assert_eq!(
                from_projection(mode, Some(900), Some(1_800), 0),
                State::OfflineUnexpectedly,
                "{mode:?} must not be reported as reachable"
            );
            assert_eq!(
                from_projection(mode, Some(900), Some(1_800), 0).api_name(),
                "isolated",
                "{mode:?}"
            );
        }
    }

    /// An unrecognised mode advertises no wake, exactly as `isolated` does.
    #[test]
    fn an_unrecognised_mode_advertises_no_wake() {
        assert_eq!(
            from_projection("who knows", Some(900), Some(1_800), 0).expected_wake_at(),
            None
        );
    }

    /// **The only wildcard in this file turns a string into `Unrecognised`.**
    ///
    /// A `match` on `&str` cannot be exhaustive, so one catch-all is
    /// unavoidable — the point of [`StoredMode`] is to concentrate it in the
    /// parse, where it produces a *named* variant, and to leave the derivation
    /// exhaustive so a mode added to the schema fails to compile until someone
    /// decides what it means for watering.
    ///
    /// The domain's `no_catch_all_arm_on_a_safety_match` reads
    /// `irrigation/gate.rs` and does not reach this crate; this is its sibling,
    /// and the reason it is written per file rather than once is that each file
    /// has its own answer to "which wildcards are legitimate here".
    #[test]
    fn the_only_catch_all_arm_produces_the_unrecognised_variant() {
        let source = include_str!("connectivity.rs");
        let offenders: Vec<(usize, &str)> = source
            .lines()
            .enumerate()
            .filter(|(_, line)| {
                let trimmed = line.trim_start();
                !trimmed.starts_with("//") && trimmed.starts_with("_ =>")
            })
            .filter(|(_, line)| !line.contains("Self::Unrecognised"))
            .map(|(number, line)| (number + 1, line.trim()))
            .collect();
        assert!(
            offenders.is_empty(),
            "a catch-all arm must not decide a connectivity state;              only the string parse may have one:
{offenders:?}"
        );
    }
}
