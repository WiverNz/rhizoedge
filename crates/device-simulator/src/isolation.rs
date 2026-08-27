//! The device's own view of whether it can reach the Edge.
//!
//! [connectivity-modes.md](../../../../docs/architecture/connectivity-modes.md)
//! mode C: the device is alone with its sensors, its actuator, and its policy.
//!
//! # What isolation is, and what it is not
//!
//! Losing the broker is losing the *connection*, not losing the device. While
//! isolated the process keeps running, virtual time keeps advancing, the
//! physical model keeps evolving, sensors keep sampling, and history goes into
//! the bounded buffer. Telemetry cannot leave, `edge.time` cannot arrive, and
//! Edge-commanded watering is impossible — but a device with no valid clock is
//! still a fully functioning sensor node (protocol §5.12).
//!
//! **In M2 an isolated device never waters**, even holding a valid enabled
//! policy. Evaluation and autonomous scheduling arrive together in M6-019.

use rhizo_mqtt_contract::payload::{Connectivity, ConnectivityMode};

/// How long a device has been alone, and how long it last was.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IsolationState {
    /// Monotonic instant the current isolation began, while isolated.
    since_ms: Option<u64>,
    /// Duration of the most recent completed isolation.
    last_ms: u64,
}

impl Default for IsolationState {
    fn default() -> Self {
        // A device begins its life isolated: it has not reached a broker yet,
        // and saying "connected" before a ConnAck would be a claim about the
        // world it has no evidence for.
        Self {
            since_ms: Some(0),
            last_ms: 0,
        }
    }
}

impl IsolationState {
    /// Whether the device currently believes it is alone.
    #[must_use]
    pub const fn is_isolated(&self) -> bool {
        self.since_ms.is_some()
    }

    /// How long the most recent completed isolation lasted.
    #[must_use]
    pub const fn last_isolation_ms(&self) -> u64 {
        self.last_ms
    }

    /// Records a lost connection, if one was held.
    ///
    /// Idempotent: a second disconnection while already isolated does not
    /// restart the clock, or a flapping link would report an isolation far
    /// shorter than the one the plant actually experienced.
    pub const fn on_disconnected(&mut self, monotonic_now_ms: u64) {
        if self.since_ms.is_none() {
            self.since_ms = Some(monotonic_now_ms);
        }
    }

    /// Records a restored connection, returning how long the isolation lasted.
    pub const fn on_connected(&mut self, monotonic_now_ms: u64) -> u64 {
        if let Some(since) = self.since_ms.take() {
            self.last_ms = monotonic_now_ms.saturating_sub(since);
        }
        self.last_ms
    }

    /// The device's own view, for `device.status` (protocol §5.5).
    ///
    /// While isolated, `isolated_ms` is how long the current isolation has run.
    /// While connected it is how long the **most recent** one lasted — the only
    /// way the Edge can ever learn it, since a device cannot publish while it
    /// is isolated. `mode` distinguishes the two, and the value is kept stable
    /// rather than reported once, so a lost status message does not lose the
    /// fact that a plant ran alone for six hours.
    #[must_use]
    pub const fn connectivity(&self, monotonic_now_ms: u64) -> Connectivity {
        match self.since_ms {
            Some(since) => Connectivity {
                mode: ConnectivityMode::Isolated,
                isolated_ms: monotonic_now_ms.saturating_sub(since),
            },
            None => Connectivity {
                mode: ConnectivityMode::Connected,
                isolated_ms: self.last_ms,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_device_begins_isolated_because_it_has_not_met_an_edge_yet() {
        let state = IsolationState::default();
        assert!(state.is_isolated());
        assert_eq!(state.connectivity(0).mode, ConnectivityMode::Isolated);
    }

    #[test]
    fn connecting_ends_the_isolation_and_records_how_long_it_ran() {
        let mut state = IsolationState::default();
        let six_hours = 6 * 60 * 60 * 1_000;
        assert_eq!(state.on_connected(six_hours), six_hours);
        assert!(!state.is_isolated());

        let connectivity = state.connectivity(six_hours);
        assert_eq!(connectivity.mode, ConnectivityMode::Connected);
        assert_eq!(
            connectivity.isolated_ms, six_hours,
            "the first status after a reconnection is how the edge learns it"
        );
    }

    #[test]
    fn a_flapping_link_reports_the_whole_isolation_not_the_last_flap() {
        let mut state = IsolationState::default();
        state.on_connected(1_000);
        state.on_disconnected(2_000);
        // Several further "disconnections" while already isolated.
        for now in [3_000, 4_000, 5_000] {
            state.on_disconnected(now);
        }
        assert_eq!(
            state.connectivity(10_000).isolated_ms,
            8_000,
            "the plant was alone for eight seconds, not for five"
        );
        assert_eq!(state.on_connected(10_000), 8_000);
    }

    #[test]
    fn the_reported_duration_grows_while_the_isolation_continues() {
        let mut state = IsolationState::default();
        state.on_connected(0);
        state.on_disconnected(1_000);
        assert_eq!(state.connectivity(1_500).isolated_ms, 500);
        assert_eq!(state.connectivity(61_000).isolated_ms, 60_000);
    }

    #[test]
    fn the_previous_duration_survives_until_the_next_isolation() {
        let mut state = IsolationState::default();
        state.on_connected(0);
        state.on_disconnected(0);
        state.on_connected(5_000);
        // Repeated heartbeats keep reporting it, so a lost status does not lose
        // the fact.
        for now in [5_000, 60_000, 600_000] {
            assert_eq!(state.connectivity(now).isolated_ms, 5_000);
            assert_eq!(state.connectivity(now).mode, ConnectivityMode::Connected);
        }
        state.on_disconnected(600_000);
        state.on_connected(600_100);
        assert_eq!(state.last_isolation_ms(), 100, "and is then replaced");
    }

    #[test]
    fn a_monotonic_clock_that_appears_to_go_backwards_reports_zero_not_nonsense() {
        let mut state = IsolationState::default();
        state.on_connected(0);
        state.on_disconnected(10_000);
        assert_eq!(
            state.connectivity(5_000).isolated_ms,
            0,
            "saturating, never a wrapped enormous duration"
        );
    }
}
