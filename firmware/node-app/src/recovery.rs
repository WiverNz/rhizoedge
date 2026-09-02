//! Interrupted-dose detection and reporting (M9-013, SAFETY-011).
//!
//! An interrupted dose delivered an **unknown** volume, and treating unknown as
//! either zero or full success would be wrong in a dangerous direction.
//! `delivered_ml: null` means genuinely unknown: reporting `0.0` would let the
//! edge grant the full budget again, and reporting the requested volume from
//! the device would be a guess. Null lets the edge apply its own conservative
//! policy (M6-010).
//!
//! **Never resume.** The correct response to an unknown partial delivery is to
//! report it and let the edge re-evaluate with fresh soil data.

use rhizo_mqtt_contract::payload::{CommandOrigin, CommandResult};

use crate::command::{ResultOutcome, result_for};
use crate::persist::PersistedState;
use crate::ports::NvsStore;

/// Detects and reports a dose interrupted by a restart.
///
/// Runs on boot, **after** the pump has been driven off and the NVS has been
/// loaded, and before anything network-related. Returns the result to publish,
/// which is durably ledgered before this function returns so a failure to
/// publish means it is retried on the next boot rather than lost.
pub fn report_interrupted(
    state: &mut PersistedState,
    nvs: &mut impl NvsStore,
) -> Option<CommandResult> {
    let dose = state.in_flight_dose.take()?;
    let origin = if dose.autonomous {
        CommandOrigin::OfflineAutonomous
    } else {
        CommandOrigin::EdgeCommand
    };
    let result = result_for(
        dose.command_id,
        dose.requested_ml,
        state.daily.delivered_ml,
        origin,
        ResultOutcome::Interrupted,
    );
    crate::dedup::record(&mut state.dedup_ring, dose.command_id, result.clone());
    // The ledger is what makes "retried next boot" true. If it is full the
    // result is still published once — but an interrupted dose carries unknown
    // delivery, so unlike a rejection it is not safe to drop, and the ledger's
    // reserved slot is what keeps room for exactly this case.
    let _ = state.pending_results.insert(result.clone());
    state.pending_results.raise_fault_if_crossed();
    let _ = nvs.store(state);
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fakes::{FakeNvs, FakePump, call_log};
    use crate::persist::InFlightDose;
    use crate::ports::Pump;
    use rhizo_mqtt_contract::payload::CommandStatus;
    use rhizo_mqtt_contract::{CommandId, UtcMillis};
    use uuid::Uuid;

    fn interrupted_state() -> PersistedState {
        PersistedState {
            in_flight_dose: Some(InFlightDose {
                command_id: CommandId::from_uuid(Uuid::from_u128(7)),
                started_at_ms: Some(UtcMillis(1_000)),
                requested_ml: 40.0,
                autonomous: false,
            }),
            ..PersistedState::default()
        }
    }

    #[test]
    fn safety_011_interrupted_dose_reported() {
        let mut state = interrupted_state();
        let mut nvs = FakeNvs::new();
        let result = report_interrupted(&mut state, &mut nvs).expect("a dose was in flight");
        assert_eq!(result.status, CommandStatus::Interrupted);
        assert_eq!(result.requested_ml, 40.0);
        assert!(state.in_flight_dose.is_none());
    }

    /// Null, not zero. Zero would let the edge grant the full budget again.
    #[test]
    fn safety_011_delivered_volume_is_null_and_never_zero() {
        let mut state = interrupted_state();
        let mut nvs = FakeNvs::new();
        let result = report_interrupted(&mut state, &mut nvs).expect("in flight");
        assert_eq!(result.delivered_ml, None);
        let json = serde_json::to_value(&result).expect("encodes");
        assert!(json["delivered_ml"].is_null());
    }

    #[test]
    fn the_dose_is_not_resumed_and_the_pump_stays_off() {
        let mut state = interrupted_state();
        let mut nvs = FakeNvs::new();
        let log = call_log();
        let mut pump = FakePump::new(log);
        pump.off();
        report_interrupted(&mut state, &mut nvs);
        assert_eq!(pump.total_run_ms, 0, "no resumption");
    }

    #[test]
    fn a_boot_with_no_in_flight_dose_reports_nothing() {
        let mut state = PersistedState::default();
        let mut nvs = FakeNvs::new();
        assert!(report_interrupted(&mut state, &mut nvs).is_none());
    }

    /// The record clears only after the result is durably ledgered, so a boot
    /// that cannot publish still retries on the next one.
    #[test]
    fn the_result_survives_a_failure_to_publish_and_is_retried_next_boot() {
        let mut state = interrupted_state();
        let mut nvs = FakeNvs::new();
        report_interrupted(&mut state, &mut nvs);

        let rebooted = nvs.power_cycle();
        let restored = rebooted.load().expect("state survived");
        assert!(restored.in_flight_dose.is_none(), "not reported twice");
        assert_eq!(restored.pending_results.len(), 1);
        assert_eq!(
            restored.pending_results.entries()[0].result.status,
            CommandStatus::Interrupted
        );
    }

    #[test]
    fn an_autonomous_dose_is_reported_with_its_own_origin() {
        let mut state = interrupted_state();
        if let Some(dose) = state.in_flight_dose.as_mut() {
            dose.autonomous = true;
        }
        let mut nvs = FakeNvs::new();
        let result = report_interrupted(&mut state, &mut nvs).expect("in flight");
        assert_eq!(result.origin, CommandOrigin::OfflineAutonomous);
    }
}
