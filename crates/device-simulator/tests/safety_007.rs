//! SAFETY-007, end to end against a real broker.
//!
//! The verification PRD 020 says matters most: publish `requested_ml: 10000`
//! **directly to the broker**, bypassing any edge, and confirm the simulator
//! does not deliver it. Everything M6 later claims about the hard limit rests on
//! this being true, three milestones before hardware exists.
//!
//! Direct publication is the point. A test that went through the edge would be
//! testing the edge's own clamping, and the edge is exactly the component the
//! device is not allowed to trust.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::time::Duration;

use rhizo_mqtt_contract::safety::{FIRMWARE_MAX_ML_PER_RUN, FIRMWARE_MAX_RUN_SECONDS};
use rhizo_mqtt_contract::{DeviceId, Topic};
use support::{SimulatedDevice, publish};

/// Long enough for a real dose to run.
///
/// Without `--time-scale` (M2-014) a clamped 80 ml dose at 8.2 ml/s takes
/// nearly ten seconds of wall time, and the result only exists once the pump
/// stops. A shorter wait would fail on timing rather than on behaviour.
const DOSE_TIMEOUT: Duration = Duration::from_secs(45);

const DEVICE: &str = "plant-node-01";

fn topic(suffix: &str) -> String {
    let id = DeviceId::parse(DEVICE).unwrap();
    match suffix {
        "water" => Topic::CommandWater(id),
        "result" => Topic::CommandResult(id),
        "time" => Topic::Time(id),
        "status" => Topic::Status(id),
        other => panic!("no such topic: {other}"),
    }
    .as_string()
}

/// The device's clock must be synchronised before any command is accepted, so
/// the refusal under test is the hard limit and not `clock_unsynced`.
async fn synchronise(client: &rumqttc::AsyncClient, now_ms: i64) {
    publish(
        client,
        &topic("time"),
        &format!(
            r#"{{"v":1,"kind":"edge.time","message_id":"018fd8b2-0000-7000-8000-00000000aa01",
                "device_id":"{DEVICE}","data":{{"edge_time_ms":{now_ms}}}}}"#
        ),
        false,
    )
    .await;
}

fn water_command(command_id: &str, requested_ml: f64, now_ms: i64) -> String {
    format!(
        r#"{{"v":1,"kind":"command.water",
            "message_id":"018fd7b1-0000-7000-8000-00000000bb01",
            "device_id":"{DEVICE}",
            "data":{{"command_id":"{command_id}",
                     "requested_ml":{requested_ml},
                     "issued_at_ms":{now_ms},
                     "expires_at_ms":{}}}}}"#,
        now_ms + 120_000
    )
}

#[tokio::test]
async fn safety_007_simulator_refuses_like_hardware() {
    let Some(broker) = support::broker("safety_007_simulator_refuses_like_hardware").await else {
        return;
    };
    let device = SimulatedDevice::start(
        &broker,
        DEVICE,
        &["--initial-moisture", "15", "--ml-per-second", "8.2"],
    )
    .await;

    let mut edge = broker
        .edge_subscriber("test-safety-007", &topic("result"))
        .await;
    let now_ms = 1_756_121_400_000;
    synchronise(&edge.client(), now_ms).await;
    // Give the device a moment to apply the synchronisation before the command
    // arrives, or the refusal under test would be `clock_unsynced` instead.
    tokio::time::sleep(Duration::from_millis(500)).await;

    let tank_before = device.core().environment().tank.remaining_ml();

    publish(
        &edge.client(),
        &topic("water"),
        &water_command("018fd7b1-4c2e-7f10-a3b8-9d1e2f304050", 10_000.0, now_ms),
        false,
    )
    .await;

    let result = edge
        .next_matching(DOSE_TIMEOUT, |m| m.topic == topic("result"))
        .await
        .expect("the device must answer every command");

    let data = &result.json()["data"];
    assert_eq!(
        data["status"], "completed",
        "an oversized request is clamped, not refused; reason was {}",
        data["reason"]
    );
    assert_eq!(data["requested_ml"], 10_000.0);
    assert_eq!(
        data["clamped"], true,
        "the device must say a hard limit changed the request"
    );
    let delivered = data["delivered_ml"].as_f64().unwrap_or(0.0);
    assert!(
        delivered <= f64::from(FIRMWARE_MAX_ML_PER_RUN),
        "{delivered} ml was reported delivered against a hard limit of {FIRMWARE_MAX_ML_PER_RUN}"
    );
    let duration_ms = data["duration_ms"].as_u64().unwrap_or(0);
    assert!(
        duration_ms <= u64::from(FIRMWARE_MAX_RUN_SECONDS) * 1000,
        "the pump ran for {duration_ms} ms against a hard limit of {} ms",
        FIRMWARE_MAX_RUN_SECONDS * 1000
    );

    // The reported number could in principle be a polite fiction. The reservoir
    // cannot be: assert on the water that actually moved.
    let drawn = tank_before - device.core().environment().tank.remaining_ml();
    assert!(
        drawn <= f64::from(FIRMWARE_MAX_ML_PER_RUN) + 1e-6,
        "{drawn} ml actually left the reservoir"
    );

    device.stop_cleanly().await;
}

#[tokio::test]
async fn safety_001_the_same_command_published_three_times_actuates_once() {
    let Some(broker) = support::broker("safety_001_three_publications_one_actuation").await else {
        return;
    };
    let device = SimulatedDevice::start(&broker, DEVICE, &["--initial-moisture", "15"]).await;
    let mut edge = broker
        .edge_subscriber("test-safety-001", &topic("result"))
        .await;
    let now_ms = 1_756_121_400_000;
    synchronise(&edge.client(), now_ms).await;
    tokio::time::sleep(Duration::from_millis(500)).await;

    let tank_before = device.core().environment().tank.remaining_ml();
    let command_id = "018fd7b1-4c2e-7f10-a3b8-9d1e2f304051";
    let mut results = Vec::new();
    for _ in 0..3 {
        publish(
            &edge.client(),
            &topic("water"),
            &water_command(command_id, 40.0, now_ms),
            false,
        )
        .await;
        // Each publication waits for its own result, so the second and third
        // arrive *after* the first dose completed. That is the case PRD 020
        // names — a repeat of a finished command — rather than a repeat racing
        // a dose still in flight, which is refused for a different reason.
        results.push(
            edge.next_matching(DOSE_TIMEOUT, |m| m.topic == topic("result"))
                .await
                .expect("every publication must be answered"),
        );
    }
    assert_eq!(results.len(), 3, "three commands, three results");
    for result in &results {
        assert_eq!(result.json()["data"]["command_id"], command_id);
        assert_eq!(
            result.json()["data"]["status"],
            "completed",
            "reason was {}",
            result.json()["data"]["reason"]
        );
        assert!(!result.retain, "a command result is never retained");
    }

    let drawn = tank_before - device.core().environment().tank.remaining_ml();
    assert!(
        (drawn - 40.0).abs() < 1e-6,
        "exactly one dose of 40 ml should have left the reservoir, not {drawn}"
    );

    device.stop_cleanly().await;
}
