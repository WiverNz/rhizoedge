//! The boot sequence, and the ordering that is the requirement (M9-007, SAFETY-011).
//!
//! ```text
//! PumpOff -> RailsOff -> NvsLoad -> [unfinished dose? -> report interrupted]
//!         -> WifiConnect -> MqttConnect -> Subscribed -> TimeSynced -> Running
//! ```
//!
//! `pump_off()` is the **first** statement, before Wi-Fi, before MQTT, before
//! NVS. The rails follow it: an unexpected reset must not leave a transceiver
//! powered from a battery for two weeks.
//!
//! # The part software cannot cover
//!
//! Before any Rust runs — during reset and the bootloader window — the pin
//! floats. Only a hardware pull-down makes that safe, which is why the
//! requirement is documented in the board profile beside the pin it constrains
//! and why HIL-1 puts a multimeter on the line across twenty resets. **No
//! amount of correct firmware compensates for wiring it the other way.**
//!
//! The ordering is asserted here by recording the sequence of adapter calls: an
//! ordering regression is otherwise invisible until hardware exists.

use rhizo_mqtt_contract::payload::CommandResult;

use crate::persist::{BootIdentity, PersistedState};
use crate::ports::{NvsStore, PowerRail, Pump, Rng, Watchdog};

/// What a boot produced.
#[derive(Clone, Debug)]
pub struct BootOutcome {
    /// The state this boot runs against.
    pub state: PersistedState,
    /// The identity this boot runs under.
    pub identity: BootIdentity,
    /// Whether the persisted state was absent or unreadable.
    ///
    /// A fresh start is logged and reported as an `nvs_reset` device event. It
    /// does **not** block boot — a device that refuses to start because its
    /// flash is damaged is a device that needs a visit — but nothing in it is
    /// trusted either: an empty dedup ring means the device has no evidence
    /// about past commands, and the caller treats that as a reason to refuse
    /// autonomous actuation, not as a clean slate.
    pub nvs_reset: bool,
    /// A dose that was in flight when the device reset.
    pub interrupted: Option<CommandResult>,
}

/// Runs the boot sequence.
///
/// Takes every peripheral it touches so the ordering is visible in one place
/// rather than spread across an initialisation routine.
pub fn boot<P: Pump, N: NvsStore, R: Rng, W: Watchdog>(
    pump: &mut P,
    rails: &mut [&mut dyn PowerRail],
    nvs: &mut N,
    watchdog: &mut W,
    rng: &mut R,
) -> BootOutcome {
    // 1. The pump, first, before anything else can fail.
    pump.off();

    // 2. Every switched rail, for the same reason one step later.
    for rail in rails.iter_mut() {
        rail.disable();
    }

    // 3. The watchdog, so a hang from here on is a reset that lands in step 1.
    watchdog.feed();

    // 4. Persistent state.
    let loaded = nvs.load();
    let nvs_reset = loaded.is_none();
    let mut state = loaded.unwrap_or_default();

    // 5. Boot identity, which advances the persisted generation.
    let identity = crate::identity::begin_boot(&mut state, None, rng);

    // 6. An unfinished dose, reported before anything network-related.
    let interrupted = crate::recovery::report_interrupted(&mut state, nvs);

    let _ = nvs.store(&state);

    BootOutcome {
        state,
        identity,
        nvs_reset,
        interrupted,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fakes::{CountingRng, FakeNvs, FakePump, FakeRail, FakeWatchdog, call_log};
    use crate::persist::InFlightDose;
    use rhizo_mqtt_contract::payload::CommandStatus;
    use rhizo_mqtt_contract::{CommandId, UtcMillis};
    use uuid::Uuid;

    /// SAFETY-011. The ordering is the requirement, so the ordering is what is
    /// asserted — not merely that `pump_off` was called at some point.
    #[test]
    fn safety_011_boot_state_pump_off() {
        let log = call_log();
        let mut pump = FakePump::new(log.clone());
        let mut sensor_rail = FakeRail::new(log.clone(), "sensor");
        let mut rs485_rail = FakeRail::new(log.clone(), "rs485");
        let mut nvs = FakeNvs::new();
        let mut watchdog = FakeWatchdog::default();
        let mut rng = CountingRng::new(1);

        boot(
            &mut pump,
            &mut [&mut sensor_rail, &mut rs485_rail],
            &mut nvs,
            &mut watchdog,
            &mut rng,
        );

        let calls = log.borrow().clone();
        assert_eq!(
            calls.first().map(String::as_str),
            Some("pump_off"),
            "pump_off is the first statement, before anything else: {calls:?}"
        );
        assert_eq!(
            calls,
            vec![
                "pump_off".to_owned(),
                "rail_disable(sensor)".to_owned(),
                "rail_disable(rs485)".to_owned(),
            ]
        );
        assert!(!pump.energised);
        assert!(!sensor_rail.is_enabled());
        assert!(!rs485_rail.is_enabled());
        assert_eq!(watchdog.feeds, 1, "the watchdog is fed before any I/O wait");
    }

    /// A watchdog reset is an ordinary boot as far as this sequence is
    /// concerned, and an ordinary boot drives the pump off first.
    #[test]
    fn a_watchdog_reset_path_leaves_the_pump_off() {
        let log = call_log();
        let mut pump = FakePump::new(log.clone());
        pump.energised = true;
        let mut nvs = FakeNvs::new();
        let mut watchdog = FakeWatchdog::default();
        let mut rng = CountingRng::new(1);
        boot(&mut pump, &mut [], &mut nvs, &mut watchdog, &mut rng);
        assert!(!pump.energised);
    }

    #[test]
    fn no_initialisation_path_can_energise_the_pump() {
        let log = call_log();
        let mut pump = FakePump::new(log.clone());
        let mut nvs = FakeNvs::new();
        let mut watchdog = FakeWatchdog::default();
        let mut rng = CountingRng::new(1);
        boot(&mut pump, &mut [], &mut nvs, &mut watchdog, &mut rng);
        assert_eq!(pump.total_run_ms, 0);
        assert!(
            !log.borrow()
                .iter()
                .any(|call| call.starts_with("pump_run_for")),
            "boot never runs the pump"
        );
    }

    #[test]
    fn an_interrupted_dose_is_reported_after_the_pump_is_off() {
        let log = call_log();
        let state = PersistedState {
            in_flight_dose: Some(InFlightDose {
                command_id: CommandId::from_uuid(Uuid::from_u128(9)),
                started_at_ms: Some(UtcMillis(1_000)),
                requested_ml: 40.0,
                autonomous: false,
            }),
            ..PersistedState::default()
        };
        let mut nvs = FakeNvs::with(&state);
        let mut pump = FakePump::new(log.clone());
        let mut watchdog = FakeWatchdog::default();
        let mut rng = CountingRng::new(1);

        let outcome = boot(&mut pump, &mut [], &mut nvs, &mut watchdog, &mut rng);
        let interrupted = outcome
            .interrupted
            .expect("an interrupted dose is reported");
        assert_eq!(interrupted.status, CommandStatus::Interrupted);
        assert_eq!(interrupted.delivered_ml, None);
        assert_eq!(log.borrow().first().map(String::as_str), Some("pump_off"));
    }

    #[test]
    fn a_corrupt_store_starts_fresh_and_says_so() {
        let log = call_log();
        let mut nvs = FakeNvs::new();
        nvs.corrupt();
        let mut pump = FakePump::new(log);
        let mut watchdog = FakeWatchdog::default();
        let mut rng = CountingRng::new(1);
        let outcome = boot(&mut pump, &mut [], &mut nvs, &mut watchdog, &mut rng);
        assert!(outcome.nvs_reset);
        assert_eq!(outcome.identity.boot_generation, 1);
    }

    #[test]
    fn the_boot_generation_advances_across_reboots() {
        let log = call_log();
        let mut nvs = FakeNvs::new();
        let mut generations = Vec::new();
        for _ in 0..3 {
            let mut pump = FakePump::new(log.clone());
            let mut watchdog = FakeWatchdog::default();
            let mut rng = CountingRng::new(1);
            let outcome = boot(&mut pump, &mut [], &mut nvs, &mut watchdog, &mut rng);
            generations.push(outcome.identity.boot_generation);
        }
        assert_eq!(generations, vec![1, 2, 3]);
    }
}
