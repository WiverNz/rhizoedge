//! Offline-policy persistence and atomic activation (SAFETY-019, SCEN-094/095).
//!
//! The dangerous failure is a half-written policy taking effect: a device
//! acting on a dose field from the new policy and a cooldown from the old one.
//! ADR-015 §7's sequence prevents it, and the property that matters is that
//! **power loss at any step leaves exactly one valid policy active** — the
//! previous one before activation, the new one after, and never a blend.
//!
//! M2 stores and activates. It does not evaluate: an enabled, valid, activated
//! policy is inert until M6-019 installs the single shared evaluator.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use clap::Parser;
use device_simulator::cli::{Fault, PolicyStep};
use device_simulator::envelope::Publication;
use device_simulator::policy::PolicyRejection;
use device_simulator::{Cli, Device};
use rhizo_mqtt_contract::payload::{DeviceStatus, OfflinePolicySet, PolicyError};
use rhizo_mqtt_contract::safety::{FIRMWARE_MAX_DAILY_ML, FIRMWARE_MAX_ML_PER_RUN};
use rhizo_mqtt_contract::{DeviceId, Envelope, MessageId, Topic};
use uuid::Uuid;

static NEXT: AtomicU64 = AtomicU64::new(0);

fn scratch_state_file() -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push("rhizo-policy-tests");
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
        "--state-file",
        state_file,
    ];
    args.extend_from_slice(extra);
    let cli = Cli::try_parse_from(args).expect("test flags must parse");
    cli.validate().expect("test flags must validate");
    cli
}

fn policy_topic() -> Topic {
    Topic::Policy(DeviceId::parse("plant-node-01").unwrap())
}

/// A policy that is valid for the default device: a `pump-0` actuator, soil
/// moisture at `default`, tank level at `reservoir`, leak at `tray`.
fn policy_json(plant: &str, version: u32, enabled: bool) -> serde_json::Value {
    serde_json::json!({
        "plant_id": plant,
        "policy_version": version,
        "enabled": enabled,
        "actuator": {
            "actuator_id": "pump-0",
            "kind": "irrigation_pump",
            "dose_ml": 35.0,
            "max_doses_per_cycle": 3,
            "absorption_wait_ms": 900_000,
        },
        "control_measurement": {
            "kind": "soil_moisture",
            "point": "default",
            "trigger_below": 28.0,
            "resume_above": 34.0,
            "confirm_duration_ms": 1_800_000,
            "max_age_ms": 900_000,
        },
        "required_measurements": [
            { "kind": "tank_level", "point": "reservoir", "max_age_ms": 1_800_000 },
            { "kind": "leak_state", "point": "tray", "max_age_ms": 1_800_000 },
        ],
        "advisory_measurements": [
            { "kind": "soil_temperature", "point": "default" },
        ],
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
    })
}

fn envelope(policies: Vec<serde_json::Value>) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "v": 1,
        "kind": "device.policy",
        "message_id": MessageId::from_uuid(Uuid::from_u128(1)),
        "device_id": "plant-node-01",
        "data": { "policies": policies },
    }))
    .unwrap()
}

fn one(plant: &str, version: u32, enabled: bool) -> Vec<u8> {
    envelope(vec![policy_json(plant, version, enabled)])
}

fn statuses(published: &[Publication]) -> Vec<DeviceStatus> {
    published
        .iter()
        .filter(|p| matches!(p.topic, Topic::Status(_)))
        .map(|p| {
            Envelope::<DeviceStatus>::from_json(p.payload.as_bytes())
                .unwrap()
                .data
        })
        .collect()
}

/// A device with one policy already active, and the state file it uses.
fn with_active_policy(extra: &[&str]) -> (Device, String) {
    let state_file = scratch_state_file().display().to_string();
    let mut device = Device::new(&settings_at(&state_file, extra));
    device.on_connected().unwrap();
    device.on_message(&policy_topic(), &one("monstera-01", 7, true));
    assert!(
        device.active_policy().is_some(),
        "the fixture must activate"
    );
    (device, state_file)
}

// -------------------------------------------------------- the happy path

#[test]
fn a_valid_policy_is_staged_verified_activated_and_acknowledged() {
    let state_file = scratch_state_file().display().to_string();
    let mut device = Device::new(&settings_at(&state_file, &[]));
    device.on_connected().unwrap();

    let published = device.on_message(&policy_topic(), &one("monstera-01", 7, true));

    let active = device.active_policy().expect("a policy must be active");
    assert_eq!(active.policies.len(), 1);
    assert_eq!(active.policies[0].policy_version, 7);
    assert_eq!(device.applied_policy_versions()["monstera-01"], 7);

    // Acknowledged: the applied version is announced in status.
    let status = statuses(&published)
        .pop()
        .expect("activation republishes status");
    assert_eq!(status.applied_policy_versions["monstera-01"], 7);

    // Staging is empty once activation completes, so nothing is left where a
    // later activation could pick it up.
    assert!(device.store().state().policy_staging.is_none());
    assert!(
        device
            .store()
            .state()
            .policy_active
            .as_ref()
            .unwrap()
            .verify(),
        "the stored blob carries a checksum that matches it"
    );
}

#[test]
fn activation_and_acknowledgement_survive_a_restart() {
    let (device, state_file) = with_active_policy(&[]);
    drop(device);

    let mut device = Device::new(&settings_at(&state_file, &[]));
    let active = device
        .active_policy()
        .expect("the activated policy must survive a reboot");
    assert_eq!(active.policies[0].policy_version, 7);
    assert_eq!(device.applied_policy_versions()["monstera-01"], 7);

    let status = statuses(&device.on_connected().unwrap()).pop().unwrap();
    assert_eq!(
        status.applied_policy_versions["monstera-01"], 7,
        "and the device keeps acknowledging it"
    );
}

#[test]
fn a_second_plant_is_added_without_disturbing_the_first() {
    let (mut device, _) = with_active_policy(&[]);
    device.on_message(&policy_topic(), &one("fern-02", 1, true));

    let active = device.active_policy().unwrap();
    assert_eq!(active.policies.len(), 2);
    assert_eq!(device.applied_policy_versions()["monstera-01"], 7);
    assert_eq!(device.applied_policy_versions()["fern-02"], 1);
}

#[test]
fn an_omitted_plant_retains_its_last_policy() {
    let (mut device, _) = with_active_policy(&[]);
    // A message about a different plant only. Protocol §5.11: an omitted plant
    // retains its last policy, because a dropped message must not be able to
    // silently disable — or silently enable — autonomy.
    device.on_message(&policy_topic(), &one("fern-02", 1, true));

    let active = device.active_policy().unwrap();
    let monstera = active
        .policies
        .iter()
        .find(|p| p.plant_id.as_str() == "monstera-01")
        .expect("the omitted plant keeps its policy");
    assert!(monstera.enabled);
    assert_eq!(monstera.policy_version, 7);
}

#[test]
fn disabling_a_plant_is_a_republish_at_a_higher_version() {
    let (mut device, _) = with_active_policy(&[]);
    device.on_message(&policy_topic(), &one("monstera-01", 8, false));

    let active = device.active_policy().unwrap();
    assert_eq!(active.policies.len(), 1);
    assert!(!active.policies[0].enabled);
    assert_eq!(device.applied_policy_versions()["monstera-01"], 8);
}

// ------------------------------------------------------------- rejections

#[test]
fn an_invalid_policy_is_rejected_and_the_previous_one_stays_active() {
    let (mut device, _) = with_active_policy(&[]);
    let before = device.active_policy().cloned().unwrap();

    let mut over_limit = policy_json("monstera-01", 8, true);
    over_limit["actuator"]["dose_ml"] = serde_json::json!(f64::from(FIRMWARE_MAX_ML_PER_RUN) + 1.0);
    let published = device.on_message(&policy_topic(), &envelope(vec![over_limit]));

    assert!(
        published.is_empty(),
        "a rejection publishes no acknowledgement"
    );
    assert_eq!(
        device.active_policy().unwrap(),
        &before,
        "steps 1 to 4 are non-destructive"
    );
    assert_eq!(device.applied_policy_versions()["monstera-01"], 7);
    assert!(matches!(
        device.last_policy_rejection(),
        Some(PolicyRejection::Contract {
            error: PolicyError::DoseAboveHardLimit,
            ..
        })
    ));
}

#[test]
fn no_policy_can_raise_a_firmware_hard_limit() {
    let (mut device, _) = with_active_policy(&[]);
    let cases: Vec<(&str, serde_json::Value)> = vec![
        (
            "dose above the per-run limit",
            serde_json::json!(f64::from(FIRMWARE_MAX_ML_PER_RUN) + 0.1),
        ),
        ("an absurd dose", serde_json::json!(10_000.0)),
    ];
    for (label, dose) in cases {
        let mut smuggled = policy_json("monstera-01", 9, true);
        smuggled["actuator"]["dose_ml"] = dose;
        // ...and a field that does not exist on the type at all.
        smuggled["max_ml_per_run"] = serde_json::json!(10_000.0);
        device.on_message(&policy_topic(), &envelope(vec![smuggled]));
        assert_eq!(
            device.applied_policy_versions()["monstera-01"],
            7,
            "{label} must not be applied"
        );
    }

    let mut window = policy_json("monstera-01", 9, true);
    window["limits"]["max_volume_per_window_ml"] =
        serde_json::json!(f64::from(FIRMWARE_MAX_DAILY_ML) + 1.0);
    device.on_message(&policy_topic(), &envelope(vec![window]));
    assert_eq!(device.applied_policy_versions()["monstera-01"], 7);
}

#[test]
fn a_policy_naming_an_undeclared_actuator_is_rejected() {
    let (mut device, _) = with_active_policy(&[]);
    let mut wrong = policy_json("monstera-01", 8, true);
    wrong["actuator"]["actuator_id"] = serde_json::json!("pump-9");
    device.on_message(&policy_topic(), &envelope(vec![wrong]));

    assert!(matches!(
        device.last_policy_rejection(),
        Some(PolicyRejection::UndeclaredActuator { .. })
    ));
    assert_eq!(device.applied_policy_versions()["monstera-01"], 7);
}

#[test]
fn a_monitoring_only_device_rejects_any_policy_that_would_water() {
    let state_file = scratch_state_file().display().to_string();
    let mut device = Device::new(&settings_at(&state_file, &["--actuators", ""]));
    device.on_connected().unwrap();
    device.on_message(&policy_topic(), &one("monstera-01", 7, true));

    assert!(
        device.active_policy().is_none(),
        "a plant with no actuator has no actuation path at all (SAFETY-018)"
    );
    assert!(matches!(
        device.last_policy_rejection(),
        Some(PolicyRejection::UndeclaredActuator { .. })
    ));
}

#[test]
fn a_policy_referencing_a_sensor_this_device_lacks_is_rejected() {
    let state_file = scratch_state_file().display().to_string();
    // No tank sensor, but the policy requires `tank_level` at `reservoir`.
    let mut device = Device::new(&settings_at(&state_file, &["--sensors", "soil,leak"]));
    device.on_connected().unwrap();
    device.on_message(&policy_topic(), &one("monstera-01", 7, true));

    assert!(device.active_policy().is_none());
    assert!(matches!(
        device.last_policy_rejection(),
        Some(PolicyRejection::UnproducibleMeasurement { .. })
    ));
}

#[test]
fn one_bad_plant_rejects_the_whole_message_rather_than_half_applying_it() {
    let (mut device, _) = with_active_policy(&[]);
    let mut bad = policy_json("fern-02", 1, true);
    bad["control_measurement"]["resume_above"] = serde_json::json!(1.0);
    let published = device.on_message(
        &policy_topic(),
        &envelope(vec![policy_json("monstera-01", 8, true), bad]),
    );

    assert!(published.is_empty());
    assert_eq!(
        device.applied_policy_versions()["monstera-01"],
        7,
        "the good plant in the same message is not applied either: a half-applied \
         set is the failure this sequence exists to prevent"
    );
    assert!(!device.applied_policy_versions().contains_key("fern-02"));
}

#[test]
fn a_malformed_payload_leaves_the_active_policy_untouched() {
    let (mut device, _) = with_active_policy(&[]);
    for payload in [
        b"not json".to_vec(),
        b"{}".to_vec(),
        b"".to_vec(),
        envelope(vec![serde_json::json!({ "plant_id": "x" })]),
    ] {
        assert!(device.on_message(&policy_topic(), &payload).is_empty());
    }
    assert_eq!(device.applied_policy_versions()["monstera-01"], 7);
    assert!(device.active_policy().is_some());
}

// -------------------------------------------------------- version monotonicity

#[test]
fn a_policy_at_or_below_the_applied_version_is_ignored() {
    let (mut device, _) = with_active_policy(&[]);
    for version in [1, 6, 7] {
        // A change that would be visible if it were applied.
        let mut older = policy_json("monstera-01", version, false);
        older["limits"]["cooldown_ms"] = serde_json::json!(1);
        let published = device.on_message(&policy_topic(), &envelope(vec![older]));
        assert!(published.is_empty(), "version {version} must be ignored");
        let active = device.active_policy().unwrap();
        assert!(
            active.policies[0].enabled,
            "a rollback republishing an old retained policy must not regress the device"
        );
        assert_eq!(active.policies[0].limits.cooldown_ms, 21_600_000);
    }
    assert_eq!(device.applied_policy_versions()["monstera-01"], 7);
}

#[test]
fn an_older_version_is_ignored_without_even_being_validated() {
    let (mut device, _) = with_active_policy(&[]);
    // Invalid *and* old. If it were validated it would be reported as a
    // rejection; being ignored means the version check came first.
    let mut invalid_and_old = policy_json("monstera-01", 3, true);
    invalid_and_old["actuator"]["dose_ml"] = serde_json::json!(10_000.0);
    device.on_message(&policy_topic(), &envelope(vec![invalid_and_old]));
    assert!(
        device.last_policy_rejection().is_none(),
        "an old version is not an error, it is a retained republication"
    );
}

// ------------------------------------------------- interruption, SAFETY-019

/// SCEN-095: interruption at every step leaves exactly one valid active policy.
#[test]
fn safety_019_interrupted_activation_leaves_one_valid_policy() {
    for step in PolicyStep::ALL {
        let state_file = scratch_state_file().display().to_string();
        let mut device = Device::new(&settings_at(&state_file, &[]));
        device.on_connected().unwrap();
        // A policy already in force, so "the previous one" is a real thing to
        // be left with rather than "none".
        device.on_message(&policy_topic(), &one("monstera-01", 7, true));
        let previous = device.active_policy().cloned().unwrap();

        device.enable_fault(Fault::PolicyInterrupt { step });
        let mut upgrade = policy_json("monstera-01", 8, true);
        upgrade["limits"]["cooldown_ms"] = serde_json::json!(1_000);
        device.on_message(&policy_topic(), &envelope(vec![upgrade]));

        // The device died and rebooted at `step`. Read what survived from disk.
        drop(device);
        let device = Device::new(&settings_at(&state_file, &[]));
        let active = device
            .active_policy()
            .unwrap_or_else(|| panic!("{step}: no policy is active at all"));
        assert_eq!(
            active.policies.len(),
            1,
            "{step}: exactly one policy, never a blend"
        );

        let version = active.policies[0].policy_version;
        assert!(
            version == 7 || version == 8,
            "{step}: active version {version} is neither the old nor the new one"
        );
        if version == 7 {
            assert_eq!(active, &previous, "{step}: the previous policy, intact");
        } else {
            assert_eq!(
                active.policies[0].limits.cooldown_ms, 1_000,
                "{step}: the new policy, whole"
            );
        }
        assert!(
            device
                .store()
                .state()
                .policy_active
                .as_ref()
                .unwrap()
                .verify(),
            "{step}: the active blob must match its checksum"
        );
        assert_eq!(
            device.applied_policy_versions()["monstera-01"],
            version,
            "{step}: the acknowledged version must match the active one"
        );
    }
}

#[test]
fn interrupting_before_activation_leaves_the_previous_policy_and_after_it_the_new_one() {
    // The two halves of the property stated separately, so a regression that
    // made *every* step keep the old policy would be visible rather than
    // silently satisfying the property test above.
    let outcome = |step: PolicyStep| -> u32 {
        let state_file = scratch_state_file().display().to_string();
        let mut device = Device::new(&settings_at(&state_file, &[]));
        device.on_connected().unwrap();
        device.on_message(&policy_topic(), &one("monstera-01", 7, true));
        device.enable_fault(Fault::PolicyInterrupt { step });
        device.on_message(&policy_topic(), &one("monstera-01", 8, true));
        drop(device);
        Device::new(&settings_at(&state_file, &[]))
            .active_policy()
            .unwrap()
            .policies[0]
            .policy_version
    };
    assert_eq!(outcome(PolicyStep::Validate), 7);
    assert_eq!(outcome(PolicyStep::Stage), 7);
    assert_eq!(outcome(PolicyStep::Verify), 7);
    assert_eq!(outcome(PolicyStep::Activate), 8, "activation is the moment");
    assert_eq!(outcome(PolicyStep::Acknowledge), 8);
}

#[test]
fn a_staged_blob_left_by_an_interruption_is_never_activated_later() {
    let state_file = scratch_state_file().display().to_string();
    let mut device = Device::new(&settings_at(&state_file, &[]));
    device.on_connected().unwrap();
    device.on_message(&policy_topic(), &one("monstera-01", 7, true));
    device.enable_fault(Fault::PolicyInterrupt {
        step: PolicyStep::Stage,
    });
    device.on_message(&policy_topic(), &one("monstera-01", 8, true));
    drop(device);

    let device = Device::new(&settings_at(&state_file, &[]));
    assert_eq!(
        device.active_policy().unwrap().policies[0].policy_version,
        7,
        "a blob that was staged but never activated is not in force"
    );
    assert_eq!(
        device.applied_policy_versions()["monstera-01"],
        7,
        "and it is not acknowledged either"
    );
}

// --------------------------------------------------- corrupt store, SCEN-094

#[test]
fn a_corrupt_state_file_leaves_no_policy_active_and_substitutes_no_default() {
    let (device, state_file) = with_active_policy(&[]);
    drop(device);
    std::fs::write(&state_file, b"not a state file at all").unwrap();

    let device = Device::new(&settings_at(&state_file, &[]));
    assert!(
        device.active_policy().is_none(),
        "a corrupt store activates nothing, and invents nothing"
    );
    assert!(device.applied_policy_versions().is_empty());
    assert!(
        !device.actuation_permitted(),
        "and the device refuses to actuate at all while it cannot trust its state"
    );
}

#[test]
fn a_policy_offered_to_a_faulted_device_is_still_refused_activation_permission() {
    let (device, state_file) = with_active_policy(&[]);
    drop(device);
    std::fs::write(&state_file, b"corrupt").unwrap();

    let mut device = Device::new(&settings_at(&state_file, &[]));
    device.on_connected().unwrap();
    // The policy path itself still works — a device in diagnostic mode may
    // accept configuration — but nothing it stores can grant actuation while
    // the persistent-state fault stands.
    device.on_message(&policy_topic(), &one("monstera-01", 7, true));
    assert!(
        !device.actuation_permitted(),
        "no policy can lift a persistent-state fault"
    );
}

// ------------------------------------------------------- M2 does not evaluate

/// The boundary PRD 020 states explicitly: an enabled, valid, activated policy
/// is **inert** in M2.
#[test]
fn an_enabled_valid_policy_causes_no_autonomous_dose() {
    let (mut device, _) = with_active_policy(&["--initial-moisture", "10"]);
    let policy = device.active_policy().unwrap();
    assert!(policy.policies[0].enabled, "the policy really is enabled");

    // Bone dry, far below the trigger, for a simulated day.
    device.environment_mut().soil.set_vwc(5.0);
    let tank_before = device.environment().tank.remaining_ml();
    for _ in 0..(24 * 60) {
        let published = device.tick(60_000);
        assert!(
            !published
                .iter()
                .any(|p| matches!(p.topic, Topic::CommandResult(_))),
            "a command result with no command means an autonomous dose happened"
        );
        assert!(!device.pump_running(), "M2 schedules no autonomous dose");
    }
    assert_eq!(
        device.environment().tank.remaining_ml(),
        tank_before,
        "not one millilitre moved"
    );
    assert_eq!(device.delivered_today_ml(), 0.0);
}

#[test]
fn the_active_policy_is_reachable_as_an_input_for_the_later_evaluator() {
    let (device, _) = with_active_policy(&[]);
    // The seam M6-019 connects to `rhizo_policy::evaluate_offline`: the policy
    // and its version, readable, with nothing in M2 acting on them.
    let policy: &OfflinePolicySet = device.active_policy().unwrap();
    assert_eq!(policy.policies[0].plant_id.as_str(), "monstera-01");
    assert_eq!(device.applied_policy_versions()["monstera-01"], 7);
}
