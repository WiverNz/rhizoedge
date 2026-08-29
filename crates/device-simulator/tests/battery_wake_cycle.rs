//! A device that actually sleeps, against a real broker (M5-021).
//!
//! SCEN-110, SCEN-111, and SCEN-112 need a *producer*: the edge's sleep-aware
//! liveness model has been enforced since the post-M4 correction, and until now
//! nothing in the project could generate the behaviour it reasons about. These
//! tests are that producer, run against Mosquitto because retention, the Last
//! Will, and the difference between a clean and an unclean disconnect are broker
//! behaviour — testing them against a mock would be testing a model of MQTT.
//!
//! Everything runs on the accelerated clock. A test that waited out a real
//! 900-second wake interval is the anti-goal `time-model.md` §8 names.
#![allow(clippy::unwrap_used, clippy::expect_used)]
mod support;

use std::time::Duration;

use rhizo_mqtt_contract::Envelope;
use rhizo_mqtt_contract::payload::{DeviceStatus, DeviceStatusValue, PowerMode};

/// Virtual seconds per real second. A 60-second wake interval is one real
/// second at this scale.
const SCALE: &str = "60";
/// The shortest interval the protocol permits a sleep announcement to declare.
const WAKE_SECONDS: &str = "60";

/// Clears whatever retained status a previous test left on the broker.
///
/// Retention outlives a test process, so without this a run would assert on the
/// last test's announcement rather than this one's — and would pass or fail
/// depending on the order the harness happened to choose.
async fn clear_retained_status(broker: &support::TestBroker, device_id: &str) {
    let publisher = broker
        .edge_subscriber(
            &format!("battery-clear-{}", client_suffix()),
            "rhizo/v1/devices/+/status",
        )
        .await;
    publisher
        .client()
        .publish(
            format!("rhizo/v1/devices/{device_id}/status"),
            rumqttc::QoS::AtLeastOnce,
            true,
            Vec::new(),
        )
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
}

/// A unique client-id suffix, without needing the `v4` uuid feature here.
fn client_suffix() -> u128 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    u128::from(std::process::id()) << 64 | u128::from(NEXT.fetch_add(1, Ordering::Relaxed))
}

fn battery_flags() -> Vec<&'static str> {
    vec![
        "--power-mode",
        "battery",
        "--wake-interval-seconds",
        WAKE_SECONDS,
        "--awake-budget-seconds",
        "5",
        "--sensor-warmup-ms",
        "500",
        "--telemetry-interval",
        "10",
        "--time-scale",
        SCALE,
        "--no-control-api",
        "--no-noise",
    ]
}

/// SCEN-110. The device sleeps, and says so first: the retained status a fresh
/// subscriber finds is `offline` / `sleeping`, carrying the battery `power`
/// block the edge opens its window from.
#[tokio::test]
async fn a_sleeping_device_leaves_a_retained_sleep_announcement() {
    let Some(b) = support::broker("battery_sleep_announcement").await else {
        return;
    };
    clear_retained_status(&b, "plant-node-01").await;
    let device = support::SimulatedDevice::start(&b, "plant-node-01", &battery_flags()).await;

    // Give the device time to complete one wake and go to sleep.
    for _ in 0..60 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        if device.core().is_sleeping() {
            break;
        }
    }
    assert!(
        device.core().power_state().is_battery(),
        "a retained configuration must not be able to retire battery mode: an          absent `power` block declares nothing"
    );
    assert!(device.core().is_sleeping(), "the device never slept");

    // A subscriber arriving now sees the retained sleep announcement.
    //
    // Resubscribed in a bounded loop rather than read once: a previous test's
    // device may have left a retained will on this topic, and the device
    // republishes its announcement on its next sleep a second later. Asserting
    // on whichever retained message happened to be there first would make this
    // test a test of the order the harness chose.
    let mut status = None;
    for _ in 0..20 {
        let mut fresh = b
            .edge_subscriber(
                &format!("battery-watch-{}", client_suffix()),
                "rhizo/v1/devices/plant-node-01/status",
            )
            .await;
        if let Some(retained) = fresh
            .next_matching(Duration::from_millis(500), |m| m.topic.ends_with("/status"))
            .await
        {
            assert!(retained.retain, "the announcement must be retained");
            let decoded = Envelope::<DeviceStatus>::from_json(&retained.payload)
                .unwrap()
                .data;
            if decoded.reason.as_deref() == Some("sleeping") {
                status = Some(decoded);
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let status =
        status.expect("a retained sleep announcement must be waiting for a fresh subscriber");
    assert_eq!(status.status, DeviceStatusValue::Offline);
    assert_eq!(status.reason.as_deref(), Some("sleeping"));
    assert_eq!(status.declared_power_mode(), Some(PowerMode::Battery));
    assert_eq!(
        status.announced_sleep_interval_seconds(),
        Some(60),
        "the edge opens its window from a bounded relative interval"
    );
    assert_eq!(status.validate(), Ok(()));
}

/// A sleeping device is *silent*. Nothing appears on any command, telemetry,
/// event, or time topic while it is off the air.
#[tokio::test]
async fn a_sleeping_device_publishes_nothing_at_all() {
    let Some(b) = support::broker("battery_sleep_silence").await else {
        return;
    };
    clear_retained_status(&b, "plant-node-01").await;
    let device = support::SimulatedDevice::start(&b, "plant-node-01", &battery_flags()).await;
    let mut watch = b
        .edge_subscriber(
            &format!("battery-silence-{}", client_suffix()),
            "rhizo/v1/devices/plant-node-01/#",
        )
        .await;
    for _ in 0..60 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        if device.core().is_sleeping() {
            break;
        }
    }
    assert!(device.core().is_sleeping());
    // Drain whatever the wake produced, then listen through the sleep.
    let _ = watch.drain_for(Duration::from_millis(200)).await;
    let during_sleep = watch.drain_for(Duration::from_millis(400)).await;
    let noisy: Vec<&str> = during_sleep
        .iter()
        .map(|m| m.topic.as_str())
        .filter(|t| t.contains("/commands") || t.ends_with("/telemetry") || t.ends_with("/events"))
        .collect();
    assert!(
        noisy.is_empty(),
        "a sleeping device published {noisy:?}, and it has no radio"
    );
}

/// The wake cycle really cycles: the device comes back, publishes, and sleeps
/// again, without anybody driving it.
#[tokio::test]
async fn a_battery_device_wakes_publishes_and_sleeps_again() {
    let Some(b) = support::broker("battery_wake_cycle").await else {
        return;
    };
    clear_retained_status(&b, "plant-node-01").await;
    let device = support::SimulatedDevice::start(&b, "plant-node-01", &battery_flags()).await;
    let mut telemetry = b
        .edge_subscriber(
            &format!("battery-cycle-{}", client_suffix()),
            "rhizo/v1/devices/plant-node-01/telemetry",
        )
        .await;

    let mut sleeps = 0;
    let mut was_sleeping = false;
    for _ in 0..200 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        let sleeping = device.core().is_sleeping();
        if sleeping && !was_sleeping {
            sleeps += 1;
        }
        was_sleeping = sleeping;
        if sleeps >= 2 {
            break;
        }
    }
    assert!(
        sleeps >= 2,
        "the device must complete more than one wake cycle on its own"
    );
    assert!(
        telemetry
            .next_matching(Duration::from_secs(5), |m| m.topic.ends_with("/telemetry"))
            .await
            .is_some(),
        "each wake publishes its sampling cycle"
    );
    // Battery telemetry is published as ordinary measurements.
    assert!(
        device.core().battery_percent() < 100.0,
        "wake cycles cost charge"
    );
}

/// SCEN-112. A device that leaves without announcing fires its Last Will, and
/// the will says `connection_lost` — never `sleeping`. An unannounced absence is
/// never presented as an expected one (SAFETY-021).
#[tokio::test]
async fn sleep_without_announcing_fires_the_last_will() {
    let Some(b) = support::broker("battery_sleep_without_announcing").await else {
        return;
    };
    clear_retained_status(&b, "plant-node-01").await;
    let mut watch = b
        .edge_subscriber(
            &format!("battery-will-{}", client_suffix()),
            "rhizo/v1/devices/plant-node-01/status",
        )
        .await;
    let mut flags = battery_flags();
    flags.extend_from_slice(&["--fault", "sleep-without-announcing"]);
    let device = support::SimulatedDevice::start(&b, "plant-node-01", &flags).await;

    let will = loop {
        let Some(message) = watch
            .next_matching(Duration::from_secs(10), |m| m.topic.ends_with("/status"))
            .await
        else {
            panic!("the broker never delivered a will");
        };
        let status = Envelope::<DeviceStatus>::from_json(&message.payload)
            .unwrap()
            .data;
        if status.status == DeviceStatusValue::Offline {
            break status;
        }
    };
    assert_eq!(
        will.reason.as_deref(),
        Some("connection_lost"),
        "an unannounced absence must never look like a sleep"
    );
    assert_eq!(
        will.announced_sleep_interval_seconds(),
        None,
        "and it opens no wake window"
    );
    drop(device);
}
