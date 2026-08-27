//! Every fault in the catalogue does what it claims.
//!
//! One test per fault, asserting its **observable effect** rather than that a
//! flag was set. A fault that is enabled and does nothing is worse than no
//! fault: a scenario built on it passes while testing nothing at all.
//!
//! Faults may only ever make the device behave worse. Each test below therefore
//! also has an implicit second assertion — none of them causes a dose.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use clap::Parser;
use device_simulator::cli::Fault;
use device_simulator::envelope::Publication;
use device_simulator::{Cli, Device};
use rhizo_mqtt_contract::payload::{
    CommandResult, CommandStatus, MeasurementKind, MeasurementValue, Quality, RejectReason,
    TelemetryBatch,
};
use rhizo_mqtt_contract::safety::LeakState;
use rhizo_mqtt_contract::{DeviceId, Envelope, MessageId, Topic};
use uuid::Uuid;

const SYNCED_AT_MS: i64 = 1_756_121_400_000;
static NEXT: AtomicU64 = AtomicU64::new(0);

fn scratch_state_file() -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push("rhizo-fault-tests");
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

fn settings(extra: &[&str]) -> Cli {
    let state_file = scratch_state_file().display().to_string();
    let mut args = vec![
        "device-simulator",
        "--device-id",
        "plant-node-01",
        "--telemetry-interval",
        "10",
        "--initial-moisture",
        "20",
        "--state-file",
        &state_file,
    ];
    args.extend_from_slice(extra);
    let cli = Cli::try_parse_from(args).expect("test flags must parse");
    cli.validate().expect("test flags must validate");
    cli
}

fn id() -> DeviceId {
    DeviceId::parse("plant-node-01").unwrap()
}

fn envelope(kind: &str, data: serde_json::Value) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "v": 1,
        "kind": kind,
        "message_id": MessageId::from_uuid(Uuid::from_u128(1)),
        "device_id": "plant-node-01",
        "data": data,
    }))
    .unwrap()
}

fn water(n: u128, requested_ml: f32) -> Vec<u8> {
    envelope(
        "command.water",
        serde_json::json!({
            "command_id": rhizo_mqtt_contract::CommandId::from_uuid(Uuid::from_u128(n)),
            "requested_ml": requested_ml,
            "issued_at_ms": SYNCED_AT_MS,
            "expires_at_ms": SYNCED_AT_MS + 120_000,
        }),
    )
}

/// A connected, synchronised device.
fn ready(extra: &[&str]) -> Device {
    let mut device = Device::new(&settings(extra));
    device.on_connected().unwrap();
    device.on_message(
        &Topic::Time(id()),
        &envelope(
            "edge.time",
            serde_json::json!({ "edge_time_ms": SYNCED_AT_MS }),
        ),
    );
    device
}

fn batches(published: &[Publication]) -> Vec<TelemetryBatch> {
    published
        .iter()
        .filter(|p| matches!(p.topic, Topic::Telemetry(_)))
        .map(|p| {
            Envelope::<TelemetryBatch>::from_json(p.payload.as_bytes())
                .unwrap()
                .data
        })
        .collect()
}

fn results(published: &[Publication]) -> Vec<CommandResult> {
    published
        .iter()
        .filter(|p| matches!(p.topic, Topic::CommandResult(_)))
        .map(|p| {
            Envelope::<CommandResult>::from_json(p.payload.as_bytes())
                .unwrap()
                .data
        })
        .collect()
}

/// One sampling cycle's soil-moisture sample.
fn moisture(device: &mut Device) -> rhizo_mqtt_contract::payload::MeasurementSample {
    for _ in 0..200 {
        let published = device.tick(10_000);
        if let Some(batch) = batches(&published).into_iter().next() {
            return batch
                .samples
                .into_iter()
                .find(|s| s.kind == MeasurementKind::SoilMoisture)
                .expect("the soil sensor is enabled");
        }
    }
    panic!("no sampling cycle produced a batch");
}

fn run_to_completion(device: &mut Device) -> Vec<CommandResult> {
    let mut all = Vec::new();
    for _ in 0..1_000 {
        all.extend(results(&device.tick(100)));
        if !device.pump_running() {
            break;
        }
    }
    all
}

// ------------------------------------------------------------ sensor faults

#[test]
fn stuck_sensor_repeats_one_bit_identical_reading_forever() {
    let mut device = ready(&[]);
    let moving: Vec<u64> = (0..3)
        .map(|_| {
            moisture(&mut device)
                .value
                .map(|v| match v {
                    MeasurementValue::Scalar(s) => s.to_bits(),
                    MeasurementValue::Boolean(b) => u64::from(b),
                })
                .unwrap()
        })
        .collect();
    assert!(
        moving.windows(2).any(|w| w[0] != w[1]),
        "a healthy sensor must actually move, or the fault proves nothing"
    );

    device.enable_fault(Fault::StuckSensor);
    let frozen: Vec<u64> = (0..5)
        .map(|_| {
            moisture(&mut device)
                .value
                .map(|v| match v {
                    MeasurementValue::Scalar(s) => s.to_bits(),
                    MeasurementValue::Boolean(b) => u64::from(b),
                })
                .unwrap()
        })
        .collect();
    assert!(
        frozen.windows(2).all(|w| w[0] == w[1]),
        "bit-identical, not merely similar: {frozen:?}"
    );

    device.disable_fault("stuck-sensor");
    let thawed = moisture(&mut device);
    assert_ne!(
        thawed.value.map(|v| match v {
            MeasurementValue::Scalar(s) => s.to_bits(),
            MeasurementValue::Boolean(b) => u64::from(b),
        }),
        Some(frozen[0]),
        "disabling the fault must really unfreeze the sensor"
    );
}

#[test]
fn invalid_soil_emits_out_of_range_or_failed_reads_and_never_a_non_finite_number() {
    let mut device = ready(&[]);
    device.enable_fault(Fault::InvalidSoil { rate: 1.0 });

    let mut out_of_range = 0;
    let mut failed_reads = 0;
    for _ in 0..40 {
        let sample = moisture(&mut device);
        match sample.value {
            Some(MeasurementValue::Scalar(v)) => {
                assert!(
                    v.is_finite(),
                    "protocol §4 forbids emitting a non-finite number"
                );
                assert!(!(0.0..=100.0).contains(&v), "{v} is inside the valid range");
                assert_eq!(sample.quality, Quality::Suspect);
                out_of_range += 1;
            }
            None => {
                assert_eq!(
                    sample.quality,
                    Quality::Fault,
                    "a failed read publishes null with fault quality"
                );
                failed_reads += 1;
            }
            Some(MeasurementValue::Boolean(_)) => panic!("moisture is a scalar kind"),
        }
    }
    assert!(out_of_range > 0, "both shapes must occur");
    assert!(failed_reads > 0, "both shapes must occur");
}

#[test]
fn leak_asserts_the_sensor_and_refuses_every_dose() {
    let mut device = ready(&[]);
    device.enable_fault(Fault::Leak);
    assert_eq!(device.environment().tank.leak(), LeakState::Detected);

    let result = &results(&device.on_message(&Topic::CommandWater(id()), &water(1, 40.0)))[0];
    assert_eq!(result.reason, Some(RejectReason::LeakDetected));
    assert!(!device.pump_running());

    device.disable_fault("leak");
    assert_eq!(device.environment().tank.leak(), LeakState::Clear);
}

#[test]
fn tank_empty_drives_the_level_to_zero_and_refuses() {
    let mut device = ready(&[]);
    device.enable_fault(Fault::TankEmpty);
    assert_eq!(device.environment().tank.true_percent(), 0.0);

    let result = &results(&device.on_message(&Topic::CommandWater(id()), &water(1, 40.0)))[0];
    assert_eq!(result.reason, Some(RejectReason::TankLow));
    assert!(!device.pump_running());
}

// ------------------------------------------------------------- clock faults

#[test]
fn clock_unsync_causes_every_water_command_to_be_refused() {
    let mut device = ready(&[]);
    assert!(device.clock_synced(), "synchronised before the fault");

    device.enable_fault(Fault::ClockUnsync);
    assert!(!device.clock_synced());
    let result = &results(&device.on_message(&Topic::CommandWater(id()), &water(1, 40.0)))[0];
    assert_eq!(result.reason, Some(RejectReason::ClockUnsynced));
    assert!(!device.pump_running());

    device.disable_fault("clock-unsync");
    assert!(device.clock_synced(), "and the fault is reversible");
}

#[test]
fn clock_skew_offsets_the_devices_idea_of_the_time() {
    let mut device = ready(&[]);
    device.enable_fault(Fault::ClockSkew { seconds: 3_600 });

    // An hour ahead: a command that expires in two minutes is long expired as
    // far as this device is concerned.
    let result = &results(&device.on_message(&Topic::CommandWater(id()), &water(1, 40.0)))[0];
    assert_eq!(
        result.reason,
        Some(RejectReason::Expired),
        "the skew must reach the expiry check, not only the published timestamp"
    );

    // ...and the same skew appears in what the device publishes, so the two
    // cannot disagree.
    let published = device.tick(10_000);
    let envelope = published
        .iter()
        .find(|p| matches!(p.topic, Topic::Telemetry(_)))
        .map(|p| serde_json::from_str::<serde_json::Value>(&p.payload).unwrap())
        .expect("a batch");
    let device_time = envelope["device_time_ms"].as_i64().unwrap();
    assert!(
        device_time >= SYNCED_AT_MS + 3_600_000,
        "the published timestamp must carry the skew too"
    );
}

// -------------------------------------------------------------- pump faults

#[test]
fn pump_no_delivery_runs_the_pump_without_moving_water() {
    let mut device = ready(&[]);
    device.enable_fault(Fault::PumpNoDelivery);
    let tank_before = device.environment().tank.remaining_ml();
    let moisture_before = device.environment().soil.true_vwc();

    device.on_message(&Topic::CommandWater(id()), &water(1, 40.0));
    assert!(device.pump_running(), "the pump really runs");
    let results = run_to_completion(&mut device);

    assert_eq!(results[0].delivered_ml, Some(0.0));
    assert_eq!(device.environment().tank.remaining_ml(), tank_before);
    assert!(device.environment().soil.true_vwc() <= moisture_before);
}

#[test]
fn pump_stuck_on_is_stopped_by_the_independent_run_guard() {
    let mut device = ready(&[]);
    device.on_message(&Topic::CommandWater(id()), &water(1, 40.0));
    device.enable_fault(Fault::PumpStuckOn);

    let results = run_to_completion(&mut device);
    assert!(
        !device.pump_running(),
        "something else has to stop it, and does"
    );
    assert_eq!(
        results[0].status,
        CommandStatus::Failed,
        "a pump the guard had to cut is a hardware failure, not a completion"
    );
}

// ----------------------------------------------------------- restart faults

#[test]
fn restart_reboots_the_device_once_and_does_not_loop() {
    let cli = settings(&["--fault", "restart"]);
    let mut device = Device::new(&cli);
    let boot_before = device.store().state().boot_count;

    device.tick(100);
    assert!(device.take_restart_notice(), "the run loop must be told");
    assert_eq!(device.store().state().boot_count, boot_before + 1);

    // A second tick must not restart again, or the device would never run.
    device.tick(100);
    assert!(!device.take_restart_notice());
    assert_eq!(device.store().state().boot_count, boot_before + 1);
}

#[test]
fn a_restart_reboots_the_device_but_not_the_plant() {
    let mut device = ready(&[]);
    device.environment_mut().soil.set_vwc(11.0);
    device.environment_mut().tank.set_percent(42.0);
    let boot_before = device.store().state().boot_count;

    device.restart();

    assert_eq!(device.store().state().boot_count, boot_before + 1);
    assert!(!device.clock_synced(), "a boot has no wall clock");
    assert_eq!(device.uptime_ms(), 0);
    assert_eq!(
        device.environment().soil.true_vwc(),
        11.0,
        "the soil does not dry out because the controller rebooted"
    );
    assert_eq!(device.environment().tank.true_percent(), 42.0);
}

#[test]
fn restart_mid_dose_kills_the_device_after_the_state_write_and_reports_interrupted() {
    let mut device = ready(&["--fault", "restart-mid-dose"]);
    let boot_before = device.store().state().boot_count;

    let published = device.on_message(&Topic::CommandWater(id()), &water(9, 40.0));

    assert!(!device.pump_running(), "the device died during actuation");
    assert_eq!(device.store().state().boot_count, boot_before + 1);
    let interrupted = results(&published)
        .into_iter()
        .chain(results(&device.on_connected().unwrap()))
        .find(|r| r.command_id == rhizo_mqtt_contract::CommandId::from_uuid(Uuid::from_u128(9)))
        .expect("the interrupted dose must be reported");
    assert_eq!(interrupted.status, CommandStatus::Interrupted);
    assert_eq!(interrupted.delivered_ml, None);

    // The fault is one-shot: the next command runs normally.
    device.on_message(
        &Topic::Time(id()),
        &envelope(
            "edge.time",
            serde_json::json!({ "edge_time_ms": SYNCED_AT_MS + 1 }),
        ),
    );
    device.on_message(&Topic::CommandWater(id()), &water(10, 40.0));
    assert!(device.pump_running(), "the fault must not fire twice");
}

// ------------------------------------------------------------ disconnection

#[test]
fn disconnect_isolates_the_device_while_it_keeps_sampling() {
    let mut device = ready(&[]);
    device.enable_fault(Fault::Disconnect { seconds: 60 });
    assert!(device.is_isolated_by_fault());
    assert!(!device.is_connected(), "the connection is really gone");

    let moisture_before = device.environment().soil.true_vwc();
    for _ in 0..6 {
        // Ten seconds each: the model evolves and cycles are buffered, but
        // nothing reaches the broker.
        assert!(
            batches(&device.tick(10_000)).is_empty(),
            "nothing can be published while isolated"
        );
    }
    assert!(
        device.buffered_cycles() > 0,
        "cycles go to the bounded ring"
    );
    assert!(
        device.environment().soil.true_vwc() < moisture_before,
        "the plant keeps drying while the device is alone"
    );
    assert!(!device.is_isolated_by_fault(), "the isolation elapses");
}

// -------------------------------------------------------------- composition

#[test]
fn faults_compose_and_the_most_restrictive_refusal_wins() {
    let mut device = ready(&[]);
    device.enable_fault(Fault::Leak);
    device.enable_fault(Fault::TankEmpty);
    assert_eq!(device.faults().len(), 2);
    assert_eq!(device.environment().tank.leak(), LeakState::Detected);
    assert_eq!(device.environment().tank.true_percent(), 0.0);

    // The gate checks leak before tank, so the leak is the reported reason —
    // and either way there is no dose.
    let result = &results(&device.on_message(&Topic::CommandWater(id()), &water(1, 40.0)))[0];
    assert_eq!(result.reason, Some(RejectReason::LeakDetected));
    assert!(!device.pump_running());
}

/// Whatever is injected, nothing can produce a dose that the gate did not
/// authorise. This is the property the whole fault catalogue is subordinate to.
#[test]
fn no_fault_can_cause_a_dose() {
    let every_fault = [
        Fault::Disconnect { seconds: 1 },
        Fault::Duplicate { rate: 1.0 },
        Fault::Reorder { rate: 1.0 },
        Fault::InvalidSoil { rate: 1.0 },
        Fault::StuckSensor,
        Fault::ClockUnsync,
        Fault::ClockSkew { seconds: -90 },
        Fault::Leak,
        Fault::TankEmpty,
        Fault::PumpNoDelivery,
        Fault::PumpStuckOn,
    ];
    for fault in every_fault {
        let mut device = ready(&[]);
        let tank_before = device.environment().tank.remaining_ml();
        device.enable_fault(fault);
        for _ in 0..50 {
            device.tick(10_000);
        }
        assert!(
            !device.pump_running(),
            "{fault} started the pump with no command at all"
        );
        assert!(
            device.environment().tank.remaining_ml() <= tank_before,
            "{fault} moved water"
        );
    }
}

#[test]
fn every_fault_in_the_catalogue_has_a_test_here() {
    // The catalogue and this file must not drift apart: a fault added without
    // a test is a fault nobody has checked does anything.
    let covered = [
        "disconnect",
        "duplicate",
        "reorder",
        "invalid-soil",
        "stuck-sensor",
        "clock-unsync",
        "clock-skew",
        "leak",
        "tank-empty",
        "pump-no-delivery",
        "pump-stuck-on",
        "restart-mid-dose",
        "restart",
        "policy-interrupt",
    ];
    assert_eq!(
        covered.len(),
        Fault::NAMES.len(),
        "the catalogue changed; add or remove a test to match"
    );
    for spec in Fault::NAMES {
        let name = spec.split(':').next().unwrap();
        assert!(covered.contains(&name), "`{name}` has no test");
    }
    // `duplicate` and `reorder` are transport faults, tested as a pure pipeline
    // in `fault::pipeline_tests`; `policy-interrupt` is tested with policy
    // activation in M2-016. Both are named here so the count cannot drift.
}
