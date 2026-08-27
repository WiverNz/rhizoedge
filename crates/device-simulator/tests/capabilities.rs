//! The device declares what it can do, and the declaration matches reality.
//!
//! [ADR-016](../../../docs/adr/016-plant-binding-and-policy-model.md) forbids
//! the edge from assuming what a device can do: `device == pump controller` is
//! exactly the assumption that makes monitoring-only plants second-class.
//!
//! The assertion that carries the weight is that the declaration and the
//! sampling come from **one** table. A device that declared `illuminance` and
//! never sent it would be a bug the edge could not detect until a plant sat in
//! `Uncertain` forever waiting for a reading that was never coming.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use clap::Parser;
use device_simulator::envelope::Publication;
use device_simulator::{Cli, Device};
use rhizo_mqtt_contract::payload::{
    ConnectivityMode, DeviceStatus, MeasurementKind, Quality, TelemetryBatch,
};
use rhizo_mqtt_contract::{DeviceId, Envelope, Topic};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn scratch_state_file() -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push("rhizo-capability-tests");
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

fn settings(extra: &[&str]) -> Cli {
    settings_at(&scratch_state_file().display().to_string(), extra)
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

fn declared_kinds(status: &DeviceStatus) -> Vec<MeasurementKind> {
    status
        .capabilities
        .sensors
        .iter()
        .flat_map(|s| s.kinds.clone())
        .collect()
}

/// The criterion that matters: what is declared is what is sampled, exactly.
#[test]
fn declared_capabilities_match_the_sampled_kinds_exactly() {
    for sensors in [
        "soil,weight,tank,leak",
        "soil",
        "tank,leak",
        "weight",
        "soil,tank",
    ] {
        let mut device = Device::new(&settings(&["--sensors", sensors]));
        let status = statuses(&device.on_connected().unwrap())
            .pop()
            .expect("connecting publishes status");
        let batch = batches(&device.tick(10_000))
            .pop()
            .expect("a sampling cycle produces a batch");

        let declared = declared_kinds(&status);
        let sampled: Vec<_> = batch.samples.iter().map(|s| s.kind.clone()).collect();
        assert_eq!(
            declared, sampled,
            "`--sensors {sensors}` declares {declared:?} but samples {sampled:?}"
        );

        // ...and every sample names a sensor that was actually declared.
        for sample in &batch.samples {
            let sensor_id = sample
                .sensor_id
                .as_ref()
                .expect("samples name their sensor");
            assert!(
                status
                    .capabilities
                    .sensors
                    .iter()
                    .any(|s| s.sensor_id.as_str() == sensor_id.as_str()),
                "sample from undeclared sensor {}",
                sensor_id.as_str()
            );
        }
    }
}

#[test]
fn a_device_with_no_actuators_declares_an_empty_array_and_still_runs() {
    let mut device = Device::new(&settings(&["--actuators", ""]));
    let status = statuses(&device.on_connected().unwrap()).pop().unwrap();
    assert!(
        status.capabilities.actuators.is_empty(),
        "the shape most real plants have, not an edge case to tolerate"
    );
    assert!(
        !status.capabilities.sensors.is_empty(),
        "and it still senses everything"
    );

    // It keeps running: sampling, heartbeats, and a working status.
    let mut cycles = 0;
    for _ in 0..5 {
        cycles += batches(&device.tick(10_000)).len();
    }
    assert!(
        cycles >= 4,
        "a monitoring-only device is a fully working device"
    );

    // The wire shape is an empty array, not an absent field: the edge must be
    // able to tell "declared none" from "did not say".
    let json: serde_json::Value =
        serde_json::from_str(&device.on_connected().unwrap()[0].payload).unwrap();
    assert_eq!(
        json["data"]["capabilities"]["actuators"],
        serde_json::json!([])
    );
}

#[test]
fn sensor_ids_are_stable_across_a_restart() {
    let state_file = scratch_state_file().display().to_string();
    let ids_of = |device: &mut Device| -> Vec<String> {
        statuses(&device.on_connected().unwrap())
            .pop()
            .unwrap()
            .capabilities
            .sensors
            .iter()
            .map(|s| s.sensor_id.as_str().to_owned())
            .collect()
    };

    let mut first = Device::new(&settings_at(&state_file, &[]));
    let before = ids_of(&mut first);
    drop(first);

    let mut second = Device::new(&settings_at(&state_file, &[]));
    assert_eq!(
        ids_of(&mut second),
        before,
        "a sensor id the edge bound a plant to must survive a reboot"
    );
    assert!(!before.is_empty());
}

#[test]
fn an_uncalibrated_sensor_declares_it_and_publishes_matching_quality() {
    let mut device = Device::new(&settings(&["--sensors", "soil"]));
    let status = statuses(&device.on_connected().unwrap()).pop().unwrap();

    let uncalibrated: Vec<_> = status
        .capabilities
        .sensors
        .iter()
        .filter(|s| s.calibrated == Some(false))
        .flat_map(|s| s.kinds.clone())
        .collect();
    assert_eq!(
        uncalibrated,
        vec![MeasurementKind::SoilEc],
        "a cheap conductivity probe is not calibrated, and says so"
    );

    let batch = batches(&device.tick(10_000)).pop().unwrap();
    for sample in &batch.samples {
        let expected = if uncalibrated.contains(&sample.kind) {
            Quality::Uncalibrated
        } else {
            Quality::Ok
        };
        assert_eq!(
            sample.quality,
            expected,
            "{:?} declared calibrated={:?} but published {:?}",
            sample.kind,
            uncalibrated.contains(&sample.kind),
            sample.quality
        );
    }
}

#[test]
fn applied_policy_versions_is_present_and_empty_before_any_policy_arrives() {
    let mut device = Device::new(&settings(&[]));
    let published = device.on_connected().unwrap();
    let status = statuses(&published).pop().unwrap();
    assert!(status.applied_policy_versions.is_empty());

    // Present as an empty object on the wire, not omitted: "no policy applied"
    // and "did not say" are different answers to the edge's drift check.
    let json: serde_json::Value = serde_json::from_str(&published[0].payload).unwrap();
    assert_eq!(
        json["data"]["applied_policy_versions"],
        serde_json::json!({})
    );
}

#[test]
fn connectivity_reports_the_devices_own_view() {
    let mut device = Device::new(&settings(&[]));

    // Before any connection: isolated, because the device has no evidence of
    // an edge and claiming otherwise would be a guess.
    assert_eq!(device.connectivity().mode, ConnectivityMode::Isolated);

    let status = statuses(&device.on_connected().unwrap()).pop().unwrap();
    let connectivity = status.connectivity.expect("status declares connectivity");
    assert_eq!(connectivity.mode, ConnectivityMode::Connected);

    // An isolation, and the duration it ran for.
    device.on_disconnected();
    device.tick(6 * 60 * 60 * 1_000);
    let isolated = device.connectivity();
    assert_eq!(isolated.mode, ConnectivityMode::Isolated);
    assert_eq!(isolated.isolated_ms, 6 * 60 * 60 * 1_000);

    let status = statuses(&device.on_connected().unwrap()).pop().unwrap();
    let connectivity = status.connectivity.unwrap();
    assert_eq!(connectivity.mode, ConnectivityMode::Connected);
    assert_eq!(
        connectivity.isolated_ms,
        6 * 60 * 60 * 1_000,
        "the first status after a reconnection is the only way the edge can \
         learn that a plant ran alone for six hours"
    );
}

#[test]
fn a_device_with_no_sensors_declares_none_and_publishes_none() {
    let mut device = Device::new(&settings(&["--sensors", ""]));
    let status = statuses(&device.on_connected().unwrap()).pop().unwrap();
    assert!(status.capabilities.sensors.is_empty());
    for _ in 0..5 {
        assert!(
            batches(&device.tick(10_000)).is_empty(),
            "an empty batch MUST NOT be published"
        );
    }
}

#[test]
fn the_declaration_carries_the_point_each_sensor_samples_at() {
    let mut device = Device::new(&settings(&[]));
    let status = statuses(&device.on_connected().unwrap()).pop().unwrap();
    let mut device_batch = Device::new(&settings(&[]));
    device_batch.on_connected().unwrap();
    let batch = batches(&device_batch.tick(10_000)).pop().unwrap();

    for sample in &batch.samples {
        let sensor_id = sample.sensor_id.as_ref().unwrap();
        let declared = status
            .capabilities
            .sensors
            .iter()
            .find(|s| s.sensor_id.as_str() == sensor_id.as_str())
            .expect("a declared sensor");
        assert_eq!(
            declared.point.as_str(),
            sample.point.as_str(),
            "a sample's point must match the sensor's declared point, or a \
             policy bound to that point would never see it"
        );
    }
}

#[test]
fn the_device_id_is_not_derived_from_anything_the_declaration_says() {
    // A sanity check on the fixture, so the tests above are about
    // `plant-node-01` and not about whatever the default happened to be.
    let device = Device::new(&settings(&[]));
    assert_eq!(
        device.device_id(),
        &DeviceId::parse("plant-node-01").unwrap()
    );
}
