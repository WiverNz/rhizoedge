//! Isolation mechanics and the persisted offline runtime state (M2-017).
//!
//! M2 prepares everything an isolated device needs and **decides nothing**. The
//! two halves are tested together on purpose: a suite that showed the mechanics
//! working without also showing that no dose happens would not distinguish this
//! milestone from the next one.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use clap::Parser;
use device_simulator::cli::Fault;
use device_simulator::offline_state::CyclePhase;
use device_simulator::{Cli, Device};
use rhizo_mqtt_contract::payload::{ConnectivityMode, EventDetail, EventKind, MeasurementKind};
use rhizo_mqtt_contract::safety::LeakState;
use rhizo_mqtt_contract::{DeviceId, Envelope, MessageId, Topic};
use rhizo_policy::MonotonicMillis;
use uuid::Uuid;

static NEXT: AtomicU64 = AtomicU64::new(0);

fn scratch_state_file() -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push("rhizo-isolation-tests");
    let _ = std::fs::create_dir_all(&path);
    path.push(format!(
        "{}-{}.state.json",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    for extension in ["json", "json.corrupt", "json.tmp"] {
        let _ = std::fs::remove_file(path.with_extension(extension));
    }
    let _ = std::fs::remove_file(&path);
    path
}

fn settings_at(state_file: &str, extra: &[&str]) -> Cli {
    let mut args = vec![
        "device-simulator",
        "--device-id",
        "plant-node-01",
        "--telemetry-interval",
        "10",
        "--initial-moisture",
        "20",
        "--state-file",
        state_file,
    ];
    args.extend_from_slice(extra);
    let cli = Cli::try_parse_from(args).expect("test flags must parse");
    cli.validate().expect("test flags must validate");
    cli
}

fn policy_envelope(version: u32) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "v": 1,
        "kind": "device.policy",
        "message_id": MessageId::from_uuid(Uuid::from_u128(1)),
        "device_id": "plant-node-01",
        "data": { "policies": [{
            "plant_id": "monstera-01",
            "policy_version": version,
            "enabled": true,
            "actuator": {
                "actuator_id": "pump-0", "kind": "irrigation_pump",
                "dose_ml": 35.0, "max_doses_per_cycle": 3,
                "absorption_wait_ms": 900_000,
            },
            "control_measurement": {
                "kind": "soil_moisture", "point": "default",
                "trigger_below": 28.0, "resume_above": 34.0,
                "confirm_duration_ms": 1_800_000, "max_age_ms": 900_000,
            },
            "required_measurements": [
                { "kind": "tank_level", "point": "reservoir", "max_age_ms": 1_800_000 },
                { "kind": "leak_state", "point": "tray", "max_age_ms": 1_800_000 },
            ],
            "advisory_measurements": [],
            "limits": {
                "cooldown_ms": 21_600_000,
                "max_volume_per_window_ml": 300.0,
                "window_ms": 86_400_000,
            },
            "safety": {
                "require_leak_clear": true,
                "require_tank_above_percent": 15.0,
                "require_pump_healthy": true,
            },
        }] },
    }))
    .unwrap()
}

fn policy_topic() -> Topic {
    Topic::Policy(DeviceId::parse("plant-node-01").unwrap())
}

/// A connected device with an activated policy, and its state file.
fn provisioned(extra: &[&str]) -> (Device, String) {
    let state_file = scratch_state_file().display().to_string();
    let mut device = Device::new(&settings_at(&state_file, extra));
    device.on_connected().unwrap();
    device.on_message(&policy_topic(), &policy_envelope(7));
    assert!(device.active_policy().is_some());
    (device, state_file)
}

// -------------------------------------------------- the isolation lifecycle

#[test]
fn isolation_leaves_the_process_sampling_and_buffering_and_the_plant_evolving() {
    let (mut device, _) = provisioned(&[]);
    let moisture_before = device.environment().soil.true_vwc();
    let uptime_before = device.uptime_ms();

    device.on_disconnected();
    assert!(device.is_isolated());

    for _ in 0..30 {
        let published = device.tick(10_000);
        assert!(
            published.is_empty(),
            "nothing can leave the device while it is alone"
        );
    }

    assert!(
        device.uptime_ms() > uptime_before,
        "monotonic time continues"
    );
    assert!(
        device.environment().soil.true_vwc() < moisture_before,
        "the plant keeps drying while nobody is watching"
    );
    assert!(
        device.buffered_cycles() > 0,
        "cycles go to the bounded ring"
    );
    assert!(
        device
            .recent_samples()
            .get(&MeasurementKind::SoilMoisture)
            .is_some(),
        "sensors keep sampling"
    );
    assert_eq!(
        device.connectivity().mode,
        ConnectivityMode::Isolated,
        "and the device knows it is alone"
    );
}

#[test]
fn reconnection_is_detected_and_reported_without_resetting_the_policy() {
    let (mut device, _) = provisioned(&[]);
    device.on_disconnected();
    device.tick(6 * 60 * 60 * 1_000);

    let published = device.on_connected().unwrap();
    assert!(!device.is_isolated());

    let status = published
        .iter()
        .find(|p| matches!(p.topic, Topic::Status(_)))
        .map(|p| {
            Envelope::<rhizo_mqtt_contract::payload::DeviceStatus>::from_json(p.payload.as_bytes())
                .unwrap()
                .data
        })
        .expect("reconnecting publishes status");
    let connectivity = status.connectivity.unwrap();
    assert_eq!(connectivity.mode, ConnectivityMode::Connected);
    assert_eq!(connectivity.isolated_ms, 6 * 60 * 60 * 1_000);

    assert_eq!(
        device.applied_policy_versions()["monstera-01"],
        7,
        "reconnecting does not disturb the activated policy"
    );
    assert!(device.active_policy().is_some());
}

#[test]
fn a_device_isolated_by_a_fault_keeps_running_and_then_reconnects_by_itself() {
    let (mut device, _) = provisioned(&[]);
    device.enable_fault(Fault::Disconnect { seconds: 300 });
    assert!(device.is_isolated());

    for _ in 0..30 {
        device.tick(10_000);
    }
    assert!(!device.is_isolated_by_fault(), "the isolation elapses");
    assert!(device.buffered_cycles() > 0);
}

// ----------------------------------------- persisted runtime, SAFETY-015

#[test]
fn offline_runtime_state_round_trips_through_the_state_file() {
    let state_file = scratch_state_file().display().to_string();
    {
        let mut device = Device::new(&settings_at(&state_file, &[]));
        device
            .store_mut_for_test(|state| {
                state.offline_runtime.cycle = CyclePhase::Cooldown;
                state.offline_runtime.cooldown_remaining_ms = 21_600_000;
                state.offline_runtime.budget_window.delivered_ml = 70.0;
                state.offline_runtime.budget_window.elapsed_ms = 1_800_000;
                state.offline_runtime.confirmation_elapsed_ms = 45_000;
                state.offline_runtime.dose_count = 2;
            })
            .unwrap();
    }
    let device = Device::new(&settings_at(&state_file, &[]));
    let runtime = device.store().state().offline_runtime;
    assert_eq!(runtime.cycle, CyclePhase::Cooldown);
    assert_eq!(runtime.cooldown_remaining_ms, 21_600_000);
    assert_eq!(runtime.budget_window.delivered_ml, 70.0);
    assert_eq!(runtime.budget_window.elapsed_ms, 1_800_000);
    assert_eq!(runtime.confirmation_elapsed_ms, 45_000);
    assert_eq!(runtime.dose_count, 2);
}

/// SAFETY-015. The rule stated plainly: with no trustworthy evidence that time
/// passed, assume none did.
#[test]
fn safety_015_reboot_does_not_replenish_budget_or_shorten_cooldown() {
    let state_file = scratch_state_file().display().to_string();
    {
        let mut device = Device::new(&settings_at(&state_file, &[]));
        device
            .store_mut_for_test(|state| {
                state.offline_runtime.cooldown_remaining_ms = 21_600_000;
                state.offline_runtime.budget_window.delivered_ml = 290.0;
                state.offline_runtime.budget_window.elapsed_ms = 3_600_000;
            })
            .unwrap();
    }

    // Ten reboots in a row. A device that reboots repeatedly does not thereby
    // earn more water.
    for _ in 0..10 {
        let device = Device::new(&settings_at(&state_file, &[]));
        let runtime = device.store().state().offline_runtime;
        assert_eq!(
            runtime.cooldown_remaining_ms, 21_600_000,
            "a reboot must not shorten a cooldown"
        );
        assert_eq!(
            runtime.budget_window.delivered_ml, 290.0,
            "a reboot must not replenish a budget"
        );
        assert_eq!(
            runtime.budget_window.elapsed_ms, 3_600_000,
            "and must not credit the window with time nobody observed"
        );
    }
}

#[test]
fn observed_time_counts_the_cooldown_down_and_a_reboot_keeps_what_is_left() {
    let state_file = scratch_state_file().display().to_string();
    {
        let mut device = Device::new(&settings_at(&state_file, &[]));
        device
            .store_mut_for_test(|state| state.offline_runtime.cooldown_remaining_ms = 600_000)
            .unwrap();
        device.on_connected().unwrap();
        // Five virtual minutes the device really observed.
        for _ in 0..30 {
            device.tick(10_000);
        }
        assert_eq!(
            device.store().state().offline_runtime.cooldown_remaining_ms,
            300_000,
            "observed time really does count down"
        );
    }
    let device = Device::new(&settings_at(&state_file, &[]));
    assert_eq!(
        device.store().state().offline_runtime.cooldown_remaining_ms,
        300_000,
        "and what was left survives the reboot intact"
    );
}

// ---------------------------------------------------------- the M6-019 seam

#[test]
fn the_seam_offers_exactly_the_shared_evaluators_arguments() {
    let (mut device, _) = provisioned(&[]);
    device.on_connected().unwrap();
    device.tick(10_000);

    let seam = device
        .offline_seam("monstera-01", 10_000)
        .expect("an activated policy is reachable through the seam");

    assert_eq!(seam.policy.plant_id.as_str(), "monstera-01");
    assert_eq!(seam.policy.policy_version, 7);
    assert_eq!(
        seam.elapsed,
        MonotonicMillis(10_000),
        "the seam carries the monotonic time the device *observed*, which is what \n         `credit_elapsed` produces and what a reboot credits as zero"
    );
    assert_eq!(seam.state.cooldown_remaining, MonotonicMillis(0));

    // The inputs are gathered, with real ages, and nothing is defaulted.
    assert!(seam.inputs.control.is_some(), "the control measurement");
    assert_eq!(seam.inputs.required.len(), 2, "tank level and leak state");
    assert_eq!(seam.inputs.leak, Some(LeakState::Clear));
    assert!(seam.inputs.tank_percent.is_some());
    assert_eq!(seam.inputs.pump_healthy, Some(true));
}

#[test]
fn the_seam_reports_absent_inputs_as_absent_rather_than_as_defaults() {
    // No tank sensor and no actuator: the two inputs a defaulting
    // implementation would quietly fill in as "full" and "healthy".
    let state_file = scratch_state_file().display().to_string();
    let mut device = Device::new(&settings_at(
        &state_file,
        &["--sensors", "soil,leak", "--actuators", ""],
    ));
    device.on_connected().unwrap();
    // The policy would be rejected on this device, so seed the runtime state
    // directly and assert on the input gathering itself.
    device.tick(10_000);

    assert_eq!(
        device.tank_reading(),
        None,
        "absence of a level sensor is not a full reservoir"
    );
    assert_eq!(
        device.pump_health(),
        None,
        "absence of an actuator is not a healthy one"
    );
    assert!(device.leak_reading().is_some(), "this one is present");
}

#[test]
fn the_seam_is_absent_for_a_plant_with_no_activated_policy() {
    let state_file = scratch_state_file().display().to_string();
    let mut device = Device::new(&settings_at(&state_file, &[]));
    device.on_connected().unwrap();
    assert!(
        device.offline_seam("monstera-01", 0).is_none(),
        "absence of a policy is not permission (SAFETY-013)"
    );
    assert!(device.policy_plants().is_empty());

    device.on_message(&policy_topic(), &policy_envelope(7));
    assert_eq!(device.policy_plants(), vec!["monstera-01".to_owned()]);
    assert!(device.offline_seam("fern-02", 0).is_none());
}

// ------------------------------------- M6-019: the evaluator is now installed

/// The boundary M2 recorded, moved.
///
/// Through M5 this test asserted the *opposite*: an enabled, valid, activated
/// policy on a bone-dry isolated plant was completely inert, because no
/// evaluator existed. M6-019 supplies the one shared
/// `rhizo_policy::evaluate_offline` and the simulator's single call site, so the
/// same setup now waters — bounded by the policy, by the per-cycle dose limit,
/// by the rolling window, and by the firmware ceilings.
///
/// Both halves are asserted here: that it *does* water, and that what it does is
/// bounded. "It waters" alone would be a weaker claim than the milestone makes.
#[test]
fn an_isolated_device_with_an_enabled_policy_waters_within_its_bounds() {
    let (mut device, _) = provisioned(&[]);
    device.on_disconnected();
    device.environment_mut().soil.set_vwc(5.0);
    device.environment_mut().tank.set_leak(LeakState::Clear);

    let tank_before = device.environment().tank.remaining_ml();
    // Twenty-three simulated hours in ten-minute steps, kept continuously dry so
    // the trigger condition never lapses. Deliberately short of a full window:
    // the budget replenishes only when the device has *observed* a whole window
    // pass, and this test is about what one window permits.
    for _ in 0..(23 * 6) {
        let published = device.tick(600_000);
        assert!(published.is_empty(), "isolated, so nothing is published");
        device.environment_mut().soil.set_vwc(5.0);
    }

    assert!(
        device.environment().tank.remaining_ml() < tank_before,
        "an isolated device with a validated, enabled policy waters the plant"
    );
    assert!(
        device.delivered_today_ml() > 0.0,
        "and the water is counted against the device's own rolling total"
    );

    // ...and it is bounded. The policy allows 35 ml twice per cycle with a
    // six-hour cooldown — 70 ml per cycle, four cycles a day — under a 300 ml
    // rolling window cap that the device may not cross inside one window.
    let delivered = tank_before - device.environment().tank.remaining_ml();
    assert!(
        delivered <= 300.0,
        "the rolling window cap bounds a day of ideal dosing conditions, got {delivered} ml"
    );
    assert!(
        device
            .store()
            .state()
            .offline_runtime
            .budget_window
            .delivered_ml
            <= 300.0,
        "and the device's own accumulator agrees"
    );
    assert!(
        device.store().state().offline_runtime.dose_count <= 2,
        "the per-cycle dose limit is never exceeded"
    );
    assert!(
        device.store().state().in_flight_dose.is_none(),
        "no dose is left in flight"
    );

    // The record of what the machine did to a living plant is buffered, because
    // there is nobody to publish it to — which is what being isolated means.
    let buffered = device.buffered_events();
    assert!(buffered > 0, "the autonomous doses are in the audit buffer");

    // **And it names the plant it watered** (protocol §5.4). Without this the
    // edge has to infer ownership from the actuator bindings that exist when the
    // replay lands, which is a different fact — and an isolated device is
    // exactly when an operator has time to rebind a pump.
    let doses: Vec<_> = device
        .store()
        .state()
        .offline_events
        .replay_events()
        .into_iter()
        .filter(|e| e.kind == EventKind::WateringOfflineAutonomous)
        .collect();
    assert!(!doses.is_empty(), "at least one dose was buffered");
    for event in &doses {
        let EventDetail::Watering { plant_id, .. } = &event.detail else {
            panic!("an autonomous dose must carry a watering detail");
        };
        assert_eq!(
            plant_id.as_ref().map(|id| id.as_str()),
            Some("monstera-01"),
            "the dose must name the plant whose policy authorised it"
        );
    }
}

/// SAFETY-013: absence of a policy is not permission.
#[test]
fn an_isolated_device_with_no_policy_never_waters() {
    let state_file = scratch_state_file().display().to_string();
    let mut device = Device::new(&settings_at(&state_file, &[]));
    device.on_disconnected();
    device.environment_mut().soil.set_vwc(5.0);
    let tank_before = device.environment().tank.remaining_ml();
    for _ in 0..(24 * 6) {
        device.tick(600_000);
        device.environment_mut().soil.set_vwc(5.0);
    }
    assert!(device.policy_plants().is_empty());
    assert_eq!(device.environment().tank.remaining_ml(), tank_before);
    assert_eq!(device.delivered_today_ml(), 0.0);
}

/// A **connected** device takes its instructions from the Edge. Evaluating
/// while connected would create the second control path ADR-015 is careful not
/// to create.
#[test]
fn a_connected_device_never_waters_autonomously() {
    let (mut device, _) = provisioned(&[]);
    device.on_connected().unwrap();
    device.environment_mut().soil.set_vwc(5.0);
    let tank_before = device.environment().tank.remaining_ml();
    for _ in 0..(24 * 6) {
        device.tick(600_000);
        device.environment_mut().soil.set_vwc(5.0);
    }
    assert_eq!(
        device.environment().tank.remaining_ml(),
        tank_before,
        "a connected device waters only when the edge tells it to"
    );
}

/// SAFETY-003 and SAFETY-004 on the offline path: a leak or an empty reservoir
/// refuses an autonomous dose exactly as it refuses a commanded one.
#[test]
fn a_leak_or_an_empty_tank_refuses_an_autonomous_dose() {
    for fault in ["leak", "tank-empty"] {
        let (mut device, _) = provisioned(&[]);
        device.on_disconnected();
        device.enable_fault(fault.parse::<Fault>().unwrap());
        device.environment_mut().soil.set_vwc(5.0);
        let tank_before = device.environment().tank.remaining_ml();
        for _ in 0..(24 * 6) {
            device.tick(600_000);
            device.environment_mut().soil.set_vwc(5.0);
        }
        assert_eq!(
            device.environment().tank.remaining_ml(),
            tank_before,
            "`{fault}` must refuse every autonomous dose"
        );
        assert_eq!(device.delivered_today_ml(), 0.0, "{fault}");
    }
}

#[test]
fn commanded_actuation_still_works_after_an_isolation() {
    // The converse of the test above: M2 removes nothing. A device that came
    // back from an isolation still obeys the edge.
    let (mut device, _) = provisioned(&[]);
    device.on_disconnected();
    device.tick(60 * 60 * 1_000);
    device.on_connected().unwrap();

    let now_ms = 1_756_121_400_000_i64;
    device.on_message(
        &Topic::Time(DeviceId::parse("plant-node-01").unwrap()),
        &serde_json::to_vec(&serde_json::json!({
            "v": 1, "kind": "edge.time",
            "message_id": MessageId::from_uuid(Uuid::from_u128(2)),
            "device_id": "plant-node-01",
            "data": { "edge_time_ms": now_ms },
        }))
        .unwrap(),
    );
    device.on_message(
        &Topic::CommandWater(DeviceId::parse("plant-node-01").unwrap()),
        &serde_json::to_vec(&serde_json::json!({
            "v": 1, "kind": "command.water",
            "message_id": MessageId::from_uuid(Uuid::from_u128(3)),
            "device_id": "plant-node-01",
            "data": {
                "command_id": Uuid::from_u128(4),
                "requested_ml": 40.0,
                "issued_at_ms": now_ms,
                "expires_at_ms": now_ms + 120_000,
            },
        }))
        .unwrap(),
    );
    assert!(
        device.pump_running(),
        "an isolation must not leave the device unable to obey a command"
    );
}
