//! Broker-backed conformance tests for the reference device.
//!
//! Everything here runs against a real Mosquitto with the project's real
//! authentication and ACL files. See `support/mod.rs` for configuration and for
//! how a machine without a broker is handled.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod support;

use std::time::Duration;

use rhizo_mqtt_contract::{DeviceId, Topic};
use support::{RECEIVE_TIMEOUT, Received, SimulatedDevice, clear_retained, publish};

const DEVICE: &str = "plant-node-01";

fn topics(device_id: &str) -> [String; 7] {
    let id = DeviceId::parse(device_id).unwrap();
    Topic::device_subscriptions(&id)
}

fn status_topic(device_id: &str) -> String {
    Topic::Status(DeviceId::parse(device_id).unwrap()).as_string()
}

/// A minimal but structurally valid `edge.time`, used purely to prove that a
/// message published by the edge reaches the device's `time` subscription.
fn edge_time_payload(device_id: &str, edge_time_ms: i64) -> String {
    format!(
        r#"{{"v":1,"kind":"edge.time","message_id":"018fd8b2-0000-7000-8000-000000000001",
            "device_id":"{device_id}","data":{{"edge_time_ms":{edge_time_ms}}}}}"#
    )
}

// -------------------------------------------------------------- M2-002

#[tokio::test]
async fn connect_publishes_a_retained_online_status_a_fresh_subscriber_receives() {
    let Some(broker) = support::broker("connect_publishes_a_retained_online_status").await else {
        return;
    };
    let device = SimulatedDevice::start(&broker, DEVICE, &[]).await;

    // A *fresh* subscriber: what it receives before any live traffic is
    // exactly the broker's retained state.
    let mut fresh = broker
        .edge_subscriber("test-fresh-status", &status_topic(DEVICE))
        .await;
    let received = fresh
        .next_matching(RECEIVE_TIMEOUT, |m| m.topic == status_topic(DEVICE))
        .await
        .expect("a retained status must be waiting for a new subscriber");

    assert!(received.retain, "status must be published retained");
    let json = received.json();
    assert_eq!(json["kind"], "device.status");
    assert_eq!(json["device_id"], DEVICE);
    assert_eq!(json["data"]["status"], "online");
    assert_eq!(
        json["data"]["limits"]["max_ml_per_run"], 80.0,
        "status reports the compile-time hard limits"
    );

    device.stop_cleanly().await;
}

#[tokio::test]
async fn every_connect_restores_exactly_the_normative_subscriptions() {
    let Some(broker) = support::broker("every_connect_restores_the_subscriptions").await else {
        return;
    };
    let mut device = SimulatedDevice::start(&broker, DEVICE, &[]).await;
    let edge = broker
        .edge_subscriber("test-subs-edge", &status_topic(DEVICE))
        .await;
    // Every subscription is now an exact topic, so each can be published to
    // directly — there is no wildcard needing a concrete member chosen for it.
    let concrete = topics(DEVICE);

    for round in ["first connect", "after a reconnect"] {
        // Retained topics would be redelivered on reconnect and could make the
        // second round pass without a live subscription; clear them first.
        for retained in [&concrete[0], &concrete[1]] {
            clear_retained(&edge.client(), retained).await;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
        for topic in &concrete {
            publish(&edge.client(), topic, r#"{"probe":true}"#, false).await;
        }
        let seen = device.observe_inbound(&concrete, RECEIVE_TIMEOUT).await;
        assert_eq!(
            seen.len(),
            concrete.len(),
            "{round}: the device did not receive {:?}",
            concrete
                .iter()
                .filter(|t| !seen.contains(t))
                .collect::<Vec<_>>()
        );

        if round == "first connect" {
            // Force a disconnect the way the broker does when a second client
            // claims the same client id — the device_id is the client id.
            support::evict(&broker, DEVICE).await;
            device
                .next_step(RECEIVE_TIMEOUT, |s| {
                    *s == device_simulator::mqtt::Step::Connected
                })
                .await
                .expect("the device must reconnect after being kicked off");
        }
    }

    device.stop_cleanly().await;
}

/// Nothing the device publishes is ever delivered back to it.
///
/// The seam this replaces: the device used to subscribe to `commands/+`, which
/// matches `commands/result` — its own output. MQTT 3.1.1 has no "no local"
/// subscription option, so the only fix was to stop subscribing to a wildcard.
///
/// The assertion is about **delivery**, not about the router discarding the
/// message after receipt. A device that received its own results and then
/// ignored them would still be carrying the seam: one refactor of the dispatch
/// table away from acting on them, and burning bandwidth in the meantime.
#[tokio::test]
async fn nothing_the_device_publishes_is_delivered_back_to_it() {
    let Some(broker) = support::broker("nothing_the_device_publishes_comes_back").await else {
        return;
    };
    let mut device = SimulatedDevice::start(&broker, DEVICE, &[]).await;
    let edge = broker
        .edge_subscriber("test-nosub-edge", &status_topic(DEVICE))
        .await;
    let id = DeviceId::parse(DEVICE).unwrap();

    let published_by_the_device = [
        Topic::CommandResult(id.clone()).as_string(),
        Topic::Telemetry(id.clone()).as_string(),
        Topic::Actuator(id.clone()).as_string(),
        Topic::Events(id.clone()).as_string(),
        Topic::Status(id.clone()).as_string(),
    ];
    for topic in &published_by_the_device {
        publish(&edge.client(), topic, r#"{"probe":true}"#, false).await;
    }

    // Anything at all on one of those topics — dispatched or ignored — means the
    // broker delivered it, which means a subscription matched it.
    let leaked = device
        .next_step(Duration::from_secs(3), |step| match step {
            device_simulator::mqtt::Step::Inbound { topic }
            | device_simulator::mqtt::Step::Ignored { topic, .. } => {
                published_by_the_device.contains(topic)
            }
            _ => false,
        })
        .await;
    assert!(
        leaked.is_none(),
        "the broker delivered the device a topic it publishes: {leaked:?}"
    );

    // Non-vacuity: the same publisher on a topic the device *does* subscribe to
    // is delivered, so the negative above is about the subscription set rather
    // than about nothing having been published.
    let command_topic = Topic::CommandWater(id).as_string();
    publish(&edge.client(), &command_topic, r#"{"probe":true}"#, false).await;
    assert_eq!(
        device
            .observe_inbound(std::slice::from_ref(&command_topic), RECEIVE_TIMEOUT)
            .await,
        vec![command_topic],
        "a real command must still arrive, or this test proves only that the \
         broker is silent"
    );

    device.stop_cleanly().await;
}

/// The same property at the moment it matters: a *real* result the device
/// itself published must not come back.
#[tokio::test]
async fn a_result_the_device_published_does_not_re_enter_command_dispatch() {
    let Some(broker) = support::broker("a_published_result_does_not_re_enter_dispatch").await
    else {
        return;
    };
    support::clear_device_retained(&broker, DEVICE).await;
    let mut device = SimulatedDevice::start(&broker, DEVICE, &["--initial-moisture", "15"]).await;
    let id = DeviceId::parse(DEVICE).unwrap();
    let result_topic = Topic::CommandResult(id.clone()).as_string();
    let mut edge = broker
        .edge_subscriber("test-result-echo", &format!("rhizo/v1/devices/{DEVICE}/#"))
        .await;

    let now_ms = 1_756_121_400_000_i64;
    publish(
        &edge.client(),
        &Topic::Time(id.clone()).as_string(),
        &edge_time_payload(DEVICE, now_ms),
        false,
    )
    .await;
    tokio::time::sleep(Duration::from_millis(400)).await;

    // A command the device will refuse instantly, so a result is published
    // without waiting out a dose.
    publish(
        &edge.client(),
        &Topic::CommandWater(id).as_string(),
        &format!(
            r#"{{"v":1,"kind":"command.water",
                "message_id":"018fd7b1-0000-7000-8000-00000000ff01",
                "device_id":"{DEVICE}",
                "data":{{"command_id":"018fd7b1-4c2e-7f10-a3b8-9d1e2f304080",
                         "requested_ml":40.0,
                         "issued_at_ms":{},
                         "expires_at_ms":{}}}}}"#,
            now_ms - 300_000,
            now_ms - 60_000
        ),
        false,
    )
    .await;

    // The edge sees the result, so it really was published to the broker.
    let result = edge
        .next_matching(RECEIVE_TIMEOUT, |m| m.topic == result_topic)
        .await
        .expect("every command produces a result");
    assert_eq!(result.json()["data"]["status"], "rejected");

    // The device does not.
    let leaked = device
        .next_step(Duration::from_secs(3), |step| match step {
            device_simulator::mqtt::Step::Inbound { topic }
            | device_simulator::mqtt::Step::Ignored { topic, .. } => *topic == result_topic,
            _ => false,
        })
        .await;
    assert!(
        leaked.is_none(),
        "the device was delivered the result it had just published: {leaked:?}"
    );

    device.stop_cleanly().await;
}

#[tokio::test]
async fn killing_the_device_publishes_the_retained_will() {
    let Some(broker) = support::broker("killing_the_device_publishes_the_retained_will").await
    else {
        return;
    };
    let device = SimulatedDevice::start(&broker, DEVICE, &[]).await;
    let mut watcher = broker
        .edge_subscriber("test-lwt-watch", &status_topic(DEVICE))
        .await;
    // Drain the retained `online` the subscription just delivered.
    let _ = watcher
        .next_matching(RECEIVE_TIMEOUT, |m: &Received| m.retain)
        .await;

    device.kill();

    let will = watcher
        .next_matching(RECEIVE_TIMEOUT, |m| m.json()["data"]["status"] == "offline")
        .await
        .expect("the broker must publish the will when the socket dies");
    assert_eq!(
        will.json()["data"]["reason"],
        "connection_lost",
        "an unclean death is distinguishable from a deliberate stop"
    );
    assert_eq!(
        will.json()["sequence"],
        0,
        "the will is fixed at connect time"
    );
}

#[tokio::test]
async fn a_clean_stop_publishes_offline_with_reason_shutdown() {
    let Some(broker) = support::broker("a_clean_stop_publishes_offline_with_reason_shutdown").await
    else {
        return;
    };
    let device = SimulatedDevice::start(&broker, DEVICE, &[]).await;
    let mut watcher = broker
        .edge_subscriber("test-shutdown-watch", &status_topic(DEVICE))
        .await;
    let _ = watcher
        .next_matching(RECEIVE_TIMEOUT, |m: &Received| m.retain)
        .await;

    device.stop_cleanly().await;

    let offline = watcher
        .next_matching(RECEIVE_TIMEOUT, |m| m.json()["data"]["status"] == "offline")
        .await
        .expect("a clean stop must publish an offline status");
    assert_eq!(offline.json()["data"]["reason"], "shutdown");

    // A live delivery never carries the retain flag, whatever the publisher
    // set — MQTT sets it only on messages served *from* the broker's store. So
    // the assertion that the status really was retained has to come from a
    // fresh subscriber, which is where a retained message is distinguishable.
    let mut fresh = broker
        .edge_subscriber("test-shutdown-fresh", &status_topic(DEVICE))
        .await;
    let stored = fresh
        .next_matching(RECEIVE_TIMEOUT, |m| m.topic == status_topic(DEVICE))
        .await
        .expect("the broker must have stored the final status");
    assert!(stored.retain);
    assert_eq!(stored.json()["data"]["status"], "offline");
    assert_eq!(stored.json()["data"]["reason"], "shutdown");
}

#[tokio::test]
async fn invalid_credentials_are_refused_by_the_broker() {
    let Some(broker) = support::broker("invalid_credentials_are_refused_by_the_broker").await
    else {
        return;
    };
    let mut options = rumqttc::MqttOptions::new("test-bad-credentials", &broker.host, broker.port);
    options.set_credentials(DEVICE, "not-the-password");
    options.set_clean_session(true);
    let (_client, mut eventloop) = rumqttc::AsyncClient::new(options, 8);

    let outcome = tokio::time::timeout(RECEIVE_TIMEOUT, async {
        loop {
            match eventloop.poll().await {
                Ok(rumqttc::Event::Incoming(rumqttc::Packet::ConnAck(ack)))
                    if ack.code == rumqttc::ConnectReturnCode::Success =>
                {
                    return Err("the broker accepted a wrong password");
                }
                Ok(_) => {}
                Err(e) => return Ok(e.to_string()),
            }
        }
    })
    .await
    .expect("the broker must answer within the timeout");
    let reason = outcome.expect("anonymous or wrong credentials must be refused");
    assert!(
        reason.to_lowercase().contains("refus")
            || reason.to_lowercase().contains("denied")
            || reason.to_lowercase().contains("not authorized")
            || reason.to_lowercase().contains("bad"),
        "unexpected refusal reason: {reason}"
    );
}

#[tokio::test]
async fn anonymous_connections_are_refused() {
    let Some(broker) = support::broker("anonymous_connections_are_refused").await else {
        return;
    };
    let mut options = rumqttc::MqttOptions::new("test-anonymous", &broker.host, broker.port);
    options.set_clean_session(true);
    let (_client, mut eventloop) = rumqttc::AsyncClient::new(options, 8);

    let refused = tokio::time::timeout(RECEIVE_TIMEOUT, async {
        loop {
            match eventloop.poll().await {
                Ok(rumqttc::Event::Incoming(rumqttc::Packet::ConnAck(ack)))
                    if ack.code == rumqttc::ConnectReturnCode::Success =>
                {
                    return false;
                }
                Ok(_) => {}
                Err(_) => return true,
            }
        }
    })
    .await
    .expect("the broker must answer within the timeout");
    assert!(
        refused,
        "allow_anonymous false is the line that makes every ACL below it mean something"
    );
}

#[tokio::test]
async fn a_device_reconnects_after_the_broker_drops_it() {
    let Some(broker) = support::broker("a_device_reconnects_after_the_broker_drops_it").await
    else {
        return;
    };
    let mut device = SimulatedDevice::start(&broker, DEVICE, &[]).await;

    support::evict(&broker, DEVICE).await;

    device
        .next_step(RECEIVE_TIMEOUT, |s| {
            *s == device_simulator::mqtt::Step::Connected
        })
        .await
        .expect("a device reconnects indefinitely; it never gives up");
    assert!(
        support::eventually(RECEIVE_TIMEOUT, || device.core().is_connected()).await,
        "the device core must know it is connected again"
    );
    device.stop_cleanly().await;
}

// -------------------------------------------------------------- M2-006

#[tokio::test]
async fn telemetry_flows_on_the_configured_interval_and_is_never_retained() {
    let Some(broker) = support::broker("telemetry_flows_and_is_never_retained").await else {
        return;
    };
    // A sibling test's retained config would otherwise set the interval.
    support::clear_device_retained(&broker, DEVICE).await;
    let device = SimulatedDevice::start(
        &broker,
        DEVICE,
        // The protocol minimum, so the test waits ten real seconds. M2-014
        // adds `--time-scale`, which is what makes a five-minute interval
        // arrive in half a second.
        &["--telemetry-interval", "10"],
    )
    .await;
    let telemetry_topic = Topic::Telemetry(DeviceId::parse(DEVICE).unwrap()).as_string();
    let mut live = broker
        .edge_subscriber("test-telemetry-live", &telemetry_topic)
        .await;

    let batch = live
        .next_matching(Duration::from_secs(30), |m| m.topic == telemetry_topic)
        .await
        .expect("telemetry must arrive on the configured interval");
    let json = batch.json();
    assert_eq!(json["kind"], "telemetry.batch");
    assert!(!batch.retain, "telemetry is never published retained");
    let samples = json["data"]["samples"].as_array().unwrap();
    assert_eq!(samples.len(), 6, "one batch carrying the whole cycle");
    assert!(json["data"]["batch_id"].is_string());

    // A *fresh* subscriber is where a retained message would be visible.
    let mut fresh = broker
        .edge_subscriber("test-telemetry-fresh", &telemetry_topic)
        .await;
    let stored = fresh.drain_for(Duration::from_millis(400)).await;
    assert!(
        stored.iter().all(|m| !m.retain),
        "the broker must be storing no telemetry at all"
    );

    device.stop_cleanly().await;
}

// -------------------------------------------------------------- M2-003

fn config_payload(device_id: &str, version: u32, interval: u32) -> String {
    format!(
        r#"{{"v":1,"kind":"device.config",
            "message_id":"018fd7a0-0000-7000-8000-00000000000{version}",
            "device_id":"{device_id}",
            "data":{{"config_version":{version},
                     "telemetry_interval_seconds":{interval},
                     "pump":{{"ml_per_second":8.2,"enabled":true}},
                     "tank":{{"min_percent":15.0}},
                     "sensors":{{"soil":true,"weight":true,"tank":true,"leak":true}}}}}}"#
    )
}

#[tokio::test]
async fn a_late_connecting_device_applies_the_retained_config() {
    let Some(broker) =
        support::broker("a_late_connecting_device_applies_the_retained_config").await
    else {
        return;
    };
    let config_topic = Topic::Config(DeviceId::parse(DEVICE).unwrap()).as_string();
    support::clear_device_retained(&broker, DEVICE).await;
    let mut watcher = broker
        .edge_subscriber("test-retained-config", &status_topic(DEVICE))
        .await;

    // Published *before* the device exists: the whole point of retention.
    publish(
        &watcher.client(),
        &config_topic,
        &config_payload(DEVICE, 7, 60),
        true,
    )
    .await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    let device = SimulatedDevice::start(&broker, DEVICE, &[]).await;
    // Both conditions together: a retained *offline* status left behind by an
    // earlier run can also carry an applied version, and matching on the
    // version alone would let that stale message satisfy the test.
    let echoed = watcher
        .next_matching(RECEIVE_TIMEOUT, |m| {
            m.json()["data"]["applied_config_version"] == 7
                && m.json()["data"]["status"] == "online"
        })
        .await
        .expect("the device must apply the retained config and echo its version");
    assert_eq!(echoed.json()["data"]["status"], "online");

    device.stop_cleanly().await;
    clear_retained(&watcher.client(), &config_topic).await;
}

#[tokio::test]
async fn edge_time_over_the_broker_makes_the_device_report_clock_synced() {
    let Some(broker) = support::broker("edge_time_over_the_broker_reports_clock_synced").await
    else {
        return;
    };
    let device = SimulatedDevice::start(&broker, DEVICE, &[]).await;
    let mut watcher = broker
        .edge_subscriber("test-time-sync", &status_topic(DEVICE))
        .await;
    let retained = watcher
        .next_matching(RECEIVE_TIMEOUT, |m| m.retain)
        .await
        .expect("a retained status exists");
    assert_eq!(
        retained.json()["clock_synced"],
        false,
        "a device learns the time from the edge, never from its host"
    );

    let time_topic = Topic::Time(DeviceId::parse(DEVICE).unwrap()).as_string();
    publish(
        &watcher.client(),
        &time_topic,
        &edge_time_payload(DEVICE, 1_756_121_400_123),
        false,
    )
    .await;

    let synced = watcher
        .next_matching(RECEIVE_TIMEOUT, |m| m.json()["clock_synced"] == true)
        .await
        .expect("the device must announce that it is synchronised");
    assert_eq!(synced.json()["device_time_ms"], 1_756_121_400_123_i64);

    // A redelivery of the same timestamp changes nothing, so no further status
    // is published for it.
    publish(
        &watcher.client(),
        &time_topic,
        &edge_time_payload(DEVICE, 1_756_121_400_123),
        false,
    )
    .await;
    assert!(
        watcher
            .next_matching(Duration::from_secs(2), |m| m.topic == status_topic(DEVICE))
            .await
            .is_none(),
        "a duplicate edge.time must be ignored entirely"
    );

    device.stop_cleanly().await;
}

// -------------------------------------------------------------- M2-010

/// The retention matrix, after a full command cycle (SCEN-015).
///
/// ADR-002 calls a retained command topic the single most damaging mistake
/// available in this protocol: the broker would redeliver it on every reconnect,
/// indefinitely, watering the plant each time. Retained `time` is the same trap
/// in a subtler form — it would set every reconnecting device's clock back to
/// the moment of publication, making long-expired commands look valid and
/// quietly defeating SAFETY-002.
///
/// The check is a **fresh subscriber**: what a new subscriber receives before
/// any live traffic is exactly the broker's stored state, and the `retain` flag
/// on delivery is what distinguishes stored from live.
#[tokio::test]
async fn retained_topics_are_exactly_status_config_and_policy() {
    let Some(broker) = support::broker("retained_topics").await else {
        return;
    };
    let id = DeviceId::parse(DEVICE).unwrap();
    support::clear_device_retained(&broker, DEVICE).await;
    let device = SimulatedDevice::start(
        &broker,
        DEVICE,
        &["--telemetry-interval", "10", "--initial-moisture", "15"],
    )
    .await;

    let everything = format!("rhizo/v1/devices/{DEVICE}/#");
    let mut live = broker
        .edge_subscriber("test-retained-live", &everything)
        .await;
    let edge = live.client();

    // A full cycle: config, time, a command, telemetry, an actuator change,
    // and a result.
    let config_topic = Topic::Config(id.clone()).as_string();
    publish(&edge, &config_topic, &config_payload(DEVICE, 11, 10), true).await;

    // The third retained topic (M2-016). Publishing it here is what makes the
    // positive half of the matrix complete rather than partial.
    let policy_topic = Topic::Policy(id.clone()).as_string();
    publish(
        &edge,
        &policy_topic,
        &policy_payload(DEVICE, "monstera-01", 11),
        true,
    )
    .await;

    let time_topic = Topic::Time(id.clone()).as_string();
    let now_ms = 1_756_121_400_000_i64;
    publish(
        &edge,
        &time_topic,
        &edge_time_payload(DEVICE, now_ms),
        false,
    )
    .await;
    // Non-vacuity: the `time` assertion below is meaningless unless an
    // `edge.time` really was published during this cycle. Prove it arrived.
    live.next_matching(RECEIVE_TIMEOUT, |m| m.topic == time_topic)
        .await
        .expect("an edge.time must have been published, or the time assertion proves nothing");

    let command_topic = Topic::CommandWater(id.clone()).as_string();
    let result_topic = Topic::CommandResult(id.clone()).as_string();
    publish(
        &edge,
        &command_topic,
        &format!(
            r#"{{"v":1,"kind":"command.water",
                "message_id":"018fd7b1-0000-7000-8000-00000000cc01",
                "device_id":"{DEVICE}",
                "data":{{"command_id":"018fd7b1-4c2e-7f10-a3b8-9d1e2f304060",
                         "requested_ml":40.0,
                         "issued_at_ms":{now_ms},
                         "expires_at_ms":{}}}}}"#,
            now_ms + 120_000
        ),
        false,
    )
    .await;
    live.next_matching(Duration::from_secs(45), |m| m.topic == result_topic)
        .await
        .expect("the command cycle must complete, or there is nothing to check retention of");

    // Everything the cycle produced is now in the broker's hands. Ask a client
    // that has never seen any of it what the broker kept.
    let mut fresh = broker
        .edge_subscriber("test-retained-fresh", &everything)
        .await;
    let delivered = fresh.drain_for(Duration::from_millis(800)).await;
    let stored: Vec<&Received> = delivered.iter().filter(|m| m.retain).collect();
    assert!(
        !stored.is_empty(),
        "the broker kept nothing at all, so this test would pass vacuously"
    );

    let status_topic = status_topic(DEVICE);
    let must_be_retained = [
        status_topic.clone(),
        config_topic.clone(),
        policy_topic.clone(),
    ];
    for topic in &must_be_retained {
        assert!(
            stored.iter().any(|m| m.topic == *topic),
            "{topic} must be retained; the broker kept {:?}",
            stored.iter().map(|m| &m.topic).collect::<Vec<_>>()
        );
    }

    let must_never_be_retained = [
        Topic::Telemetry(id.clone()).as_string(),
        Topic::Actuator(id.clone()).as_string(),
        Topic::Events(id.clone()).as_string(),
        time_topic.clone(),
        command_topic.clone(),
        Topic::CommandTare(id.clone()).as_string(),
        Topic::CommandCalibrate(id.clone()).as_string(),
        result_topic.clone(),
    ];
    for topic in &must_never_be_retained {
        assert!(
            !stored.iter().any(|m| m.topic == *topic),
            "{topic} was retained; the broker would redeliver it to every new subscriber"
        );
    }
    // ...and nothing outside the permitted set was retained either, including
    // topics this test did not think to name.
    for message in &stored {
        assert!(
            must_be_retained.contains(&message.topic),
            "{} was retained and is not in the permitted set",
            message.topic
        );
    }

    device.stop_cleanly().await;
    clear_retained(&edge, &config_topic).await;
    clear_retained(&edge, &policy_topic).await;
}

// -------------------------------------------------------------- M2-012

/// Device identity is a real boundary, enforced by the broker (SCEN-016).
///
/// ADR-012 makes the Mosquitto `%u` pattern the mechanism that turns a
/// `device_id` into a boundary; ADR-002 lists "ACL misconfiguration silently
/// granting broad access" as a risk. An untested ACL is an assumption.
///
/// # The assertion has to be about delivery, not about the publish call
///
/// Mosquitto's response to an ACL denial on publish is to **drop the message
/// silently** — under MQTT 3.1.1 it still sends the PUBACK. So `publish()`
/// returns `Ok`, and a test that asserted on its result would pass whether the
/// ACL existed or not. What must be asserted is that a subscriber never receives
/// the message.
#[tokio::test]
async fn acl_isolation_between_devices() {
    let Some(broker) = support::broker("acl_isolation_between_devices").await else {
        return;
    };
    const OTHER: &str = "plant-node-02";
    let own = Topic::Status(DeviceId::parse(DEVICE).unwrap()).as_string();
    let theirs = Topic::Status(DeviceId::parse(OTHER).unwrap()).as_string();

    // The watcher uses the edge account, which is the only one with a
    // fleet-wide view — asserting the edge can subscribe broadly at the same
    // time.
    let mut watcher = broker
        .edge_subscriber("test-acl-watcher", "rhizo/v1/devices/+/#")
        .await;

    // `plant-node-01`, authenticated as itself.
    let one = support::Subscriber::connect(
        &broker,
        "test-acl-one",
        DEVICE,
        &broker.device_password(DEVICE),
        &own,
    )
    .await;

    // Its own subtree: permitted.
    publish(&one.client(), &own, r#"{"probe":"own"}"#, false).await;
    let mine = watcher
        .next_matching(RECEIVE_TIMEOUT, |m| {
            m.topic == own && m.json()["probe"] == "own"
        })
        .await;
    assert!(
        mine.is_some(),
        "a device must be able to publish into its own subtree"
    );

    // Its neighbour's subtree: denied, silently.
    publish(&one.client(), &theirs, r#"{"probe":"intruder"}"#, false).await;
    let leaked = watcher
        .next_matching(Duration::from_secs(3), |m| m.json()["probe"] == "intruder")
        .await;
    assert!(
        leaked.is_none(),
        "a device published into another device's subtree and it was delivered: \
         the ACL is not confining `{DEVICE}`"
    );

    // ...and the same in the other direction, so the test is not passing
    // because of something specific to one account.
    let two = support::Subscriber::connect(
        &broker,
        "test-acl-two",
        OTHER,
        &broker.device_password(OTHER),
        &theirs,
    )
    .await;
    publish(&two.client(), &theirs, r#"{"probe":"own-two"}"#, false).await;
    assert!(
        watcher
            .next_matching(RECEIVE_TIMEOUT, |m| m.json()["probe"] == "own-two")
            .await
            .is_some()
    );
    publish(&two.client(), &own, r#"{"probe":"intruder-two"}"#, false).await;
    assert!(
        watcher
            .next_matching(Duration::from_secs(3), |m| m.json()["probe"]
                == "intruder-two")
            .await
            .is_none(),
        "the ACL is not confining `{OTHER}` either"
    );
}

/// A device cannot *read* its neighbour's plant either.
///
/// The pattern grants `readwrite` on its own subtree only, so a compromised or
/// simply buggy device has no view of the fleet. Publishing is the damaging
/// direction; subscribing is the private one, and both matter.
#[tokio::test]
async fn a_device_cannot_subscribe_to_another_devices_telemetry() {
    let Some(broker) = support::broker("a_device_cannot_subscribe_to_another_device").await else {
        return;
    };
    const OTHER: &str = "plant-node-02";
    let theirs = Topic::Status(DeviceId::parse(OTHER).unwrap()).as_string();

    // Subscribing to a forbidden filter is not refused at connect time; the
    // subscription simply never delivers anything.
    let mut nosy = support::Subscriber::connect(
        &broker,
        "test-acl-nosy",
        DEVICE,
        &broker.device_password(DEVICE),
        &theirs,
    )
    .await;

    let edge = broker.edge_subscriber("test-acl-publisher", &theirs).await;
    publish(&edge.client(), &theirs, r#"{"probe":"private"}"#, false).await;
    // The edge sees it, so the message really was published.
    let mut confirming = broker.edge_subscriber("test-acl-confirm", &theirs).await;
    publish(&edge.client(), &theirs, r#"{"probe":"private"}"#, false).await;
    assert!(
        confirming
            .next_matching(RECEIVE_TIMEOUT, |m| m.json()["probe"] == "private")
            .await
            .is_some(),
        "the message must really be published, or the negative below is vacuous"
    );

    assert!(
        nosy.next_matching(Duration::from_secs(3), |m| m.json()["probe"] == "private")
            .await
            .is_none(),
        "`{DEVICE}` received `{OTHER}`'s traffic; a device has no view of the fleet"
    );
}

// -------------------------------------------------------------- M2-014

/// A full dose, absorption wait, and recheck in under ten seconds of wall time.
///
/// ADR-013's whole justification: without acceleration the end-to-end suite is
/// not something anyone runs, and a suite nobody runs is one that fails
/// silently. At `--time-scale 600` ten simulated minutes pass per real second,
/// so a fifteen-minute absorption wait is a second and a half.
#[tokio::test]
async fn a_full_cycle_completes_in_under_ten_seconds_at_scale_six_hundred() {
    let Some(broker) = support::broker("a_full_cycle_at_scale_six_hundred").await else {
        return;
    };
    support::clear_device_retained(&broker, DEVICE).await;
    let device = SimulatedDevice::start(
        &broker,
        DEVICE,
        &[
            "--time-scale",
            "600",
            // Five simulated minutes between cycles: at this scale, half a
            // second of wall time.
            "--telemetry-interval",
            "300",
            "--initial-moisture",
            "20",
        ],
    )
    .await;

    let id = DeviceId::parse(DEVICE).unwrap();
    let telemetry_topic = Topic::Telemetry(id.clone()).as_string();
    let result_topic = Topic::CommandResult(id.clone()).as_string();
    let mut edge = broker
        .edge_subscriber("test-scale-edge", &format!("rhizo/v1/devices/{DEVICE}/#"))
        .await;

    let started = std::time::Instant::now();
    let now_ms = 1_756_121_400_000_i64;
    publish(
        &edge.client(),
        &Topic::Time(id.clone()).as_string(),
        &edge_time_payload(DEVICE, now_ms),
        false,
    )
    .await;

    // A sample before the dose, so "recheck" is a comparison rather than a
    // single reading.
    edge.next_matching(Duration::from_secs(10), |m| m.topic == telemetry_topic)
        .await
        .expect("a sampling cycle must arrive within a second or two at this scale");

    publish(
        &edge.client(),
        &Topic::CommandWater(id).as_string(),
        &format!(
            r#"{{"v":1,"kind":"command.water",
                "message_id":"018fd7b1-0000-7000-8000-00000000ee01",
                "device_id":"{DEVICE}",
                "data":{{"command_id":"018fd7b1-4c2e-7f10-a3b8-9d1e2f304070",
                         "requested_ml":60.0,
                         "issued_at_ms":{now_ms},
                         "expires_at_ms":{}}}}}"#,
            now_ms + 600_000
        ),
        false,
    )
    .await;

    let result = edge
        .next_matching(Duration::from_secs(10), |m| m.topic == result_topic)
        .await
        .expect("the dose must complete");
    assert_eq!(result.json()["data"]["status"], "completed");

    // The absorption wait and the recheck: another sampling cycle after the
    // dose, which at real time would be five minutes away.
    let recheck = edge
        .next_matching(Duration::from_secs(10), |m| m.topic == telemetry_topic)
        .await
        .expect("a recheck cycle must follow the dose");
    assert_eq!(recheck.json()["kind"], "telemetry.batch");

    let wall = started.elapsed();
    assert!(
        wall < Duration::from_secs(10),
        "dose, absorption, and recheck took {wall:?}; acceleration is not working"
    );

    // The published timestamps must still be plausible UTC values: the clock is
    // anchored to the Edge's real instant, so acceleration moves it fast but
    // never makes it nonsense.
    let device_time = recheck.json()["device_time_ms"].as_i64().unwrap();
    assert!(
        device_time >= now_ms,
        "a timestamp before the synchronisation it came from"
    );
    assert!(
        device_time < now_ms + 86_400_000,
        "{device_time} is more than a simulated day past the sync in a few seconds of wall time"
    );

    device.stop_cleanly().await;
}

/// `GET /sim/scale` reports the configured factor, so M8-004 can assert the edge
/// and the simulator agree.
#[tokio::test]
async fn the_control_api_reports_the_configured_time_scale() {
    use device_simulator::control::{ControlState, router};
    use tower::ServiceExt;

    let settings = {
        use clap::Parser;
        let state_file = support::scratch_state_file().display().to_string();
        let cli = device_simulator::Cli::try_parse_from([
            "device-simulator",
            "--device-id",
            DEVICE,
            "--time-scale",
            "600",
            "--state-file",
            &state_file,
        ])
        .unwrap();
        cli.validate().unwrap();
        cli
    };
    let state = ControlState::new(
        std::sync::Arc::new(std::sync::Mutex::new(device_simulator::Device::new(
            &settings,
        ))),
        std::sync::Arc::new(settings),
    );
    let response = router(state)
        .oneshot(
            axum::http::Request::builder()
                .uri("/sim/scale")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = axum::body::to_bytes(response.into_body(), 4096)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["time_scale"], 600.0);
}

// -------------------------------------------------------------- M2-016

fn policy_payload(device_id: &str, plant: &str, version: u32) -> String {
    format!(
        r#"{{"v":1,"kind":"device.policy",
            "message_id":"018fd7a1-0000-7000-8000-00000000{version:04}",
            "device_id":"{device_id}",
            "data":{{"policies":[{{
                "plant_id":"{plant}",
                "policy_version":{version},
                "enabled":true,
                "actuator":{{"actuator_id":"pump-0","kind":"irrigation_pump",
                             "dose_ml":35.0,"max_doses_per_cycle":3,
                             "absorption_wait_ms":900000}},
                "control_measurement":{{"kind":"soil_moisture","point":"default",
                                        "trigger_below":28.0,"resume_above":34.0,
                                        "confirm_duration_ms":1800000,"max_age_ms":900000}},
                "required_measurements":[
                    {{"kind":"tank_level","point":"reservoir","max_age_ms":1800000}},
                    {{"kind":"leak_state","point":"tray","max_age_ms":1800000}}],
                "advisory_measurements":[{{"kind":"soil_temperature","point":"default"}}],
                "limits":{{"cooldown_ms":21600000,"max_volume_per_window_ml":300.0,
                           "window_ms":86400000}},
                "safety":{{"require_leak_clear":true,"require_tank_above_percent":15.0,
                           "require_pump_healthy":true}}
            }}]}}}}"#
    )
}

/// `policy` completes the positive-retention set alongside `status` and
/// `config` (mqtt-v1.md §3), and a late-connecting device applies what the
/// broker kept.
/// `event.ack` end to end: the edge acknowledges a replay over the broker and
/// the device releases exactly the covered prefix — and the acknowledgement
/// leaves nothing retained behind it.
///
/// The retention half is the one worth having a real broker for. A retained
/// acknowledgement would be redelivered to this device on every future
/// reconnect, telling it to discard history the edge may no longer hold; a
/// unit test can assert the flag we pass, but only the broker can show that
/// nothing was stored.
#[tokio::test]
async fn an_acknowledgement_over_the_broker_releases_history_and_is_never_retained() {
    let Some(broker) = support::broker("event_ack_over_the_broker").await else {
        return;
    };
    let id = DeviceId::parse(DEVICE).unwrap();
    let policy_topic = Topic::Policy(id.clone()).as_string();
    let ack_topic = Topic::EventsAck(id.clone()).as_string();
    support::clear_device_retained(&broker, DEVICE).await;

    let edge = broker
        .edge_subscriber("test-ack-edge", &format!("rhizo/v1/devices/{DEVICE}/#"))
        .await;
    let device = SimulatedDevice::start(&broker, DEVICE, &[]).await;

    // Activating a policy buffers one audit event, which is the history to
    // acknowledge.
    publish(
        &edge.client(),
        &policy_topic,
        &policy_payload(DEVICE, "monstera-01", 7),
        true,
    )
    .await;
    assert!(
        support::eventually(RECEIVE_TIMEOUT, || device.core().buffered_events() > 0).await,
        "policy activation is a buffered audit event"
    );

    let (boot, through) = {
        let core = device.core();
        (
            core.boot_id(),
            core.highest_allocated_seq()
                .expect("a sequence has been allocated"),
        )
    };

    publish(
        &edge.client(),
        &ack_topic,
        &format!(
            r#"{{"v":1,"kind":"event.ack",
                "message_id":"018fd8c0-0000-7000-8000-0000000000e1",
                "device_id":"{DEVICE}",
                "data":{{"boot_id":"{boot}","through_device_seq":{through}}}}}"#
        ),
        // Derived from the topic, never chosen here: a hard-coded `false` would
        // make the retention assertion below a test of this line rather than of
        // the contract's rule.
        Topic::EventsAck(DeviceId::parse(DEVICE).unwrap())
            .metadata()
            .retained,
    )
    .await;

    assert!(
        support::eventually(RECEIVE_TIMEOUT, || device.core().buffered_events() == 0).await,
        "the device must release history the edge has acknowledged"
    );
    assert_eq!(device.core().acknowledged_through(), Some(through));

    // Nothing is waiting on `events/ack` for the next subscriber. A retained
    // acknowledgement is delivered on connect, so a fresh subscriber is exactly
    // the device's own position after a reconnect.
    let mut fresh = broker.edge_subscriber("test-ack-fresh", &ack_topic).await;
    assert!(
        fresh
            .next_matching(Duration::from_secs(2), |m| m.topic == ack_topic)
            .await
            .is_none(),
        "an acknowledgement must never be retained"
    );

    device.stop_cleanly().await;
    clear_retained(&edge.client(), &policy_topic).await;
}

#[tokio::test]
async fn retained_policy_reaches_a_late_connecting_device_and_is_acknowledged() {
    let Some(broker) = support::broker("retained_policy").await else {
        return;
    };
    let id = DeviceId::parse(DEVICE).unwrap();
    let policy_topic = Topic::Policy(id.clone()).as_string();
    support::clear_device_retained(&broker, DEVICE).await;

    let mut watcher = broker
        .edge_subscriber(
            "test-retained-policy",
            &format!("rhizo/v1/devices/{DEVICE}/#"),
        )
        .await;

    // Published retained *before* the device exists.
    publish(
        &watcher.client(),
        &policy_topic,
        &policy_payload(DEVICE, "monstera-01", 7),
        true,
    )
    .await;
    tokio::time::sleep(Duration::from_millis(250)).await;

    let device = SimulatedDevice::start(&broker, DEVICE, &[]).await;

    let acknowledged = watcher
        .next_matching(RECEIVE_TIMEOUT, |m| {
            // Both conditions: a retained `offline` status left by an earlier
            // test can carry an applied version too, and matching the version
            // alone lets that stale message satisfy the test.
            m.topic == status_topic(DEVICE)
                && m.json()["data"]["status"] == "online"
                && m.json()["data"]["applied_policy_versions"]["monstera-01"] == 7
        })
        .await
        .expect("the device must activate the retained policy and acknowledge it");
    assert_eq!(acknowledged.json()["data"]["status"], "online");
    assert!(
        device.core().active_policy().is_some(),
        "and hold it as the input the M6 evaluator will read"
    );

    // A fresh subscriber sees the policy retained, which is what makes it
    // reach a device that connects later.
    let mut fresh = broker
        .edge_subscriber("test-retained-policy-fresh", &policy_topic)
        .await;
    let stored = fresh
        .next_matching(RECEIVE_TIMEOUT, |m| m.topic == policy_topic)
        .await
        .expect("policy is one of the three retained topics");
    assert!(stored.retain);

    // ...and an enabled, activated policy still waters nothing in M2.
    assert!(!device.core().pump_running());

    device.stop_cleanly().await;
    clear_retained(&watcher.client(), &policy_topic).await;
}
