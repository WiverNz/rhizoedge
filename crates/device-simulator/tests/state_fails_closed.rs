//! Corruption of persisted safety state can only ever make the device **less**
//! permissive.
//!
//! A property test rather than a handful of cases, because the interesting
//! corruptions are the ones nobody thought to write down. It generates
//! arbitrary damage — truncation, bit flips, random bytes, plausible-looking
//! JSON with the wrong shapes — and asserts the same four things every time.
//!
//! The failure this guards against is the tempting one: "the state file is
//! broken, so start fresh". A fresh start forgets the deduplication ring, the
//! spent daily budget, and the running cooldown, and the very next command
//! waters a plant that was already watered (SAFETY-001, SAFETY-012, SAFETY-015).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use device_simulator::state::{
    BudgetWindow, CommandRecord, InFlightDose, OfflineRuntime, PersistentState, StateStore,
    StoredPolicy, encode_state,
};
use proptest::prelude::*;
use rhizo_mqtt_contract::payload::{
    ActuatorKind, CommandOrigin, CommandResult, CommandStatus, ControlMeasurement, MeasurementKind,
    MeasurementPoint, OfflineActuator, OfflineLimits, OfflinePolicy, OfflinePolicySet,
    OfflineSafety, SensorId,
};
use rhizo_mqtt_contract::safety::FIRMWARE_MAX_DAILY_ML;
use rhizo_mqtt_contract::{CommandId, UtcMillis};
use uuid::Uuid;

static NEXT: AtomicU64 = AtomicU64::new(0);

fn scratch() -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push("rhizo-state-fails-closed");
    let _ = std::fs::create_dir_all(&path);
    path.push(format!(
        "{}-{}.state.json",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    for extension in ["json.corrupt", "json.tmp"] {
        let _ = std::fs::remove_file(path.with_extension(extension));
    }
    let _ = std::fs::remove_file(&path);
    path
}

fn cleanup(path: &std::path::Path) {
    let _ = std::fs::remove_file(path);
    for extension in ["json.corrupt", "json.tmp"] {
        let _ = std::fs::remove_file(path.with_extension(extension));
    }
}

/// A device that has done real work: budget spent, cooldown running, a command
/// in the ring, a dose in flight, and a policy active.
fn a_device_with_history() -> PersistentState {
    let command_id = CommandId::from_uuid(Uuid::from_u128(7));
    let mut state = PersistentState {
        applied_config_version: Some(7),
        delivered_today_ml: 460.0,
        delivered_day_epoch: Some(20_325),
        ..PersistentState::default()
    };
    state.record_command(CommandRecord {
        command_id,
        result: CommandResult {
            command_id,
            status: CommandStatus::Completed,
            requested_ml: 40.0,
            delivered_ml: Some(40.0),
            duration_ms: Some(4_878),
            clamped: false,
            reason: None,
            delivered_today_ml: 460.0,
            origin: CommandOrigin::EdgeCommand,
            detail: None,
        },
    });
    state.in_flight_dose = Some(InFlightDose {
        command_id: CommandId::from_uuid(Uuid::from_u128(8)),
        started_at_ms: Some(UtcMillis(1_756_121_500_000)),
        started_at_monotonic_ms: 912_344,
        requested_ml: 40.0,
        effective_ml: 40.0,
    });
    state.offline_runtime = OfflineRuntime {
        cycle: device_simulator::offline_state::CyclePhase::Cooldown,
        budget_window: BudgetWindow {
            elapsed_ms: 1_800_000,
            delivered_ml: 290.0,
        },
        cooldown_remaining_ms: 21_600_000,
        confirmation_elapsed_ms: 45_000,
        dose_count: 3,
    };
    state.policy_active = Some(StoredPolicy::new(
        OfflinePolicySet {
            policies: vec![OfflinePolicy {
                plant_id: SensorId::parse("monstera-01").unwrap(),
                policy_version: 7,
                enabled: true,
                actuator: Some(OfflineActuator {
                    actuator_id: SensorId::parse("pump-0").unwrap(),
                    kind: ActuatorKind::IrrigationPump,
                    dose_ml: 35.0,
                    max_doses_per_cycle: 3,
                    absorption_wait_ms: 900_000,
                }),
                control_measurement: ControlMeasurement {
                    kind: MeasurementKind::SoilMoisture,
                    point: MeasurementPoint::parse("default").unwrap(),
                    trigger_below: 28.0,
                    resume_above: 34.0,
                    confirm_duration_ms: 1_800_000,
                    max_age_ms: 900_000,
                },
                required_measurements: Vec::new(),
                advisory_measurements: Vec::new(),
                limits: OfflineLimits {
                    cooldown_ms: 21_600_000,
                    max_volume_per_window_ml: 300.0,
                    window_ms: 86_400_000,
                },
                safety: OfflineSafety {
                    require_leak_clear: true,
                    require_tank_above_percent: 15.0,
                    require_pump_healthy: true,
                },
            }],
        },
        std::collections::BTreeMap::from([(String::from("monstera-01"), 7)]),
    ));
    state
        .applied_policy_versions
        .insert(String::from("monstera-01"), 7);
    state
}

/// Asserts the four properties that must hold after **any** load of a damaged
/// file.
fn assert_never_more_permissive(loaded: &PersistentState, history: &PersistentState) {
    // The load either preserved the safety history exactly — the damage was
    // harmless, say a byte flipped inside whitespace — or it lost some of it,
    // in which case actuation MUST be disabled.
    //
    // Stated as one implication rather than two branches on purpose. Branching
    // on `actuation_permitted` and returning early is how this test came to
    // pass against a deliberately reintroduced "corrupt file, start fresh"
    // reset: a reset produces a permissive default, the early return fires, and
    // the property proves nothing. `boot_count` is excluded because it is not
    // safety state and advances on every load by design.
    let history_intact = loaded.delivered_today_ml == history.delivered_today_ml
        && loaded.delivered_day_epoch == history.delivered_day_epoch
        && loaded.offline_runtime == history.offline_runtime
        && loaded.command_ring == history.command_ring
        && loaded.in_flight_dose == history.in_flight_dose
        && loaded.policy_active == history.policy_active
        && loaded.applied_policy_versions == history.applied_policy_versions;
    assert!(
        history_intact || !loaded.actuation_permitted(),
        "safety history was lost yet actuation is still permitted:          budget {} (was {}), cooldown {} (was {}), ring {} entries (was {}),          in-flight {:?}, policy {}",
        loaded.delivered_today_ml,
        history.delivered_today_ml,
        loaded.offline_runtime.cooldown_remaining_ms,
        history.offline_runtime.cooldown_remaining_ms,
        loaded.command_ring.len(),
        history.command_ring.len(),
        loaded.in_flight_dose.is_some(),
        loaded.policy_active.is_some()
    );

    assert!(
        loaded.delivered_today_ml >= history.delivered_today_ml,
        "a corrupt load replenished the daily budget: {} < {}",
        loaded.delivered_today_ml,
        history.delivered_today_ml
    );
    assert!(
        loaded.offline_runtime.cooldown_remaining_ms
            >= history.offline_runtime.cooldown_remaining_ms,
        "a corrupt load shortened the cooldown: {} < {}",
        loaded.offline_runtime.cooldown_remaining_ms,
        history.offline_runtime.cooldown_remaining_ms
    );
    assert!(
        loaded.offline_runtime.budget_window.delivered_ml
            >= history.offline_runtime.budget_window.delivered_ml,
        "a corrupt load replenished the rolling offline budget"
    );
    assert!(
        loaded.policy_active.is_none() || loaded.policy_active == history.policy_active,
        "a corrupt load substituted a policy that was never stored"
    );
}

/// Arbitrary damage to a byte string.
fn damage() -> impl Strategy<Value = Damage> {
    prop_oneof![
        (0usize..600).prop_map(Damage::Truncate),
        (0usize..600, any::<u8>()).prop_map(|(at, byte)| Damage::Overwrite { at, byte }),
        proptest::collection::vec(any::<u8>(), 0..64).prop_map(Damage::Replace),
        Just(Damage::Empty),
        Just(Damage::JustABrace),
        Just(Damage::WrongTypes),
    ]
}

#[derive(Clone, Debug)]
enum Damage {
    Truncate(usize),
    Overwrite { at: usize, byte: u8 },
    Replace(Vec<u8>),
    Empty,
    JustABrace,
    WrongTypes,
}

impl Damage {
    fn apply(&self, original: &[u8]) -> Vec<u8> {
        match self {
            Self::Truncate(at) => original[..(*at).min(original.len())].to_vec(),
            Self::Overwrite { at, byte } => {
                let mut bytes = original.to_vec();
                if !bytes.is_empty() {
                    let index = at % bytes.len();
                    bytes[index] = *byte;
                }
                bytes
            }
            Self::Replace(bytes) => bytes.clone(),
            Self::Empty => Vec::new(),
            Self::JustABrace => b"{".to_vec(),
            // Plausible-looking JSON whose fields have the wrong types: the
            // shape a naive "just deserialize it" would accept most eagerly.
            Self::WrongTypes => br#"{"boot_count":"lots","delivered_today_ml":null,
                "command_ring":"none","offline_runtime":[],"policy_active":true}"#
                .to_vec(),
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(96))]

    /// SAFETY-001, SAFETY-012, SAFETY-015: arbitrary corruption can never
    /// restore actuation permission or make budget or cooldown more permissive.
    #[test]
    fn safety_012_corruption_never_makes_the_device_more_permissive(damage in damage()) {
        let path = scratch();
        let history = a_device_with_history();
        std::fs::write(&path, encode_state(&history).unwrap()).unwrap();

        let original = std::fs::read(&path).unwrap();
        std::fs::write(&path, damage.apply(&original)).unwrap();

        let store = StateStore::load(&path);
        assert_never_more_permissive(store.state(), &history);

        // ...and the lockout, once taken, survives the next restart.
        if !store.actuation_permitted() {
            drop(store);
            let again = StateStore::load(&path);
            prop_assert!(
                !again.actuation_permitted(),
                "the lockout must not evaporate on the next boot"
            );
        }
        cleanup(&path);
    }
}

/// The undamaged control: without it, a property that passes because *every*
/// load fails would look identical to one that passes for the right reason.
#[test]
fn an_undamaged_file_loads_with_its_history_intact_and_actuation_permitted() {
    let path = scratch();
    let history = a_device_with_history();
    std::fs::write(&path, encode_state(&history).unwrap()).unwrap();

    let store = StateStore::load(&path);
    assert!(store.actuation_permitted());
    assert_eq!(store.state().delivered_today_ml, 460.0);
    assert_eq!(
        store.state().offline_runtime.cooldown_remaining_ms,
        21_600_000
    );
    assert!(store.state().policy_active.is_some());
    assert!(store.state().in_flight_dose.is_some());
    cleanup(&path);
}

#[test]
fn the_failed_closed_state_is_at_the_permissive_ceiling_of_every_field() {
    let fault = device_simulator::state::PersistentStateFault {
        reason: String::from("state_file_corrupt"),
        detail: String::from("test"),
    };
    let state = PersistentState::failed_closed(fault);
    assert!(!state.actuation_permitted());
    assert_eq!(state.delivered_today_ml, FIRMWARE_MAX_DAILY_ML);
    assert_eq!(state.offline_runtime.cooldown_remaining_ms, u64::MAX);
    assert_eq!(state.offline_runtime.dose_count, u16::MAX);
    assert_eq!(
        state.offline_runtime.cycle,
        device_simulator::offline_state::CyclePhase::Idle,
        "a corrupt load resumes no cycle it cannot vouch for"
    );
    assert!(state.policy_active.is_none());
    assert!(state.policy_staging.is_none());
    assert!(state.command_ring.is_empty());
    assert!(state.in_flight_dose.is_none());
}
