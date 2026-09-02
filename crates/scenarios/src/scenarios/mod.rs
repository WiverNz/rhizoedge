//! Deterministic scenario catalogue.

use crate::harness::Harness;
use anyhow::{Context, Result, bail, ensure};
use reqwest::Method;
use serde_json::{Value, json};
use std::time::Duration;
use std::{future::Future, pin::Pin};

/// The dose the scenario profile configures, in millilitres.
///
/// A named constant because several scenarios assert that *exactly one dose of
/// this size* was delivered — an autonomous dose while isolated, for instance —
/// and a literal repeated beside the profile's own literal is a pair that drifts
/// apart the first time either is retuned. It has been retuned once already:
/// 40 ml into the simulator's 2500 ml pot could never satisfy the 3.0 VWC
/// recovery threshold, and the cycle was unsatisfiable.
const SCENARIO_DOSE_ML: f64 = 60.0;

/// Half a millilitre either side, for comparing a float against the dose.
const DOSE_EPSILON_ML: f64 = 0.01;

/// Boxed scenario future.
pub type ScenarioFuture<'a> = Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>>;

/// One independently runnable assembled-system scenario.
pub struct Scenario {
    /// Stable CLI name.
    pub name: &'static str,
    /// Numbered scenarios from `docs/testing/failure-scenarios.md` this covers.
    ///
    /// F-080-20 requires every `e2e` scenario assigned to M8 to be implemented,
    /// and this field is what makes that claim checkable rather than asserted:
    /// [`tests::every_m8_scenario_in_the_catalogue_is_implemented`] reads the
    /// document and fails if an id is unclaimed or claimed twice.
    pub covers: &'static [&'static str],
    /// Safety invariants re-verified by the scenario (F-080-22).
    pub proves: &'static [&'static str],
    /// Observable-state implementation.
    pub run: for<'a> fn(&'a Harness) -> ScenarioFuture<'a>,
}

fn normal_telemetry<'a>(h: &'a Harness) -> ScenarioFuture<'a> {
    Box::pin(async move {
        for _ in 0..400 {
            let batches: i64 =
                sqlx::query_scalar("SELECT count(DISTINCT batch_id) FROM measurements")
                    .fetch_one(&h.sqlite)
                    .await?;
            if batches >= 10 {
                h.stop_service("device-simulator")?;
                ensure!(
                    batches == 10,
                    "expected exactly 10 telemetry batches, observed {batches}"
                );
                let rows: (i64, i64, i64) = sqlx::query_as(
                    "SELECT count(*), count(DISTINCT batch_id), count(DISTINCT received_at) FROM measurements",
                ).fetch_one(&h.sqlite).await?;
                ensure!(
                    rows.0 >= 10 && rows.1 == 10 && rows.2 == 10,
                    "telemetry counts are inconsistent: {rows:?}"
                );
                let regressions: i64 = sqlx::query_scalar(
                    "SELECT count(*) FROM (SELECT received_at, lag(received_at) OVER (ORDER BY id) previous FROM measurements) WHERE previous > received_at",
                ).fetch_one(&h.sqlite).await?;
                ensure!(
                    regressions == 0,
                    "received_at regressed {regressions} times"
                );
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        bail!("ten telemetry intervals did not arrive within eight seconds")
    })
}

async fn setup_plant(h: &Harness, automatic: bool) -> Result<String> {
    let state = h
        .get_json(&format!("{}/sim/state", h.simulator_url))
        .await?;
    let device = state["device_id"]
        .as_str()
        .context("simulator device_id")?
        .to_owned();
    let mut registered = false;
    for _ in 0..100 {
        let devices = h
            .get_json(&format!("{}/api/v1/devices", h.edge_url))
            .await?;
        if devices["devices"].as_array().is_some_and(|rows| {
            rows.iter().any(|row| {
                row["device_id"] == device
                    && row["connectivity"] == "connected"
                    && row["clock_synced"] == true
            })
        }) {
            registered = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    ensure!(
        registered,
        "simulator {device} did not become connected and clock-synchronised"
    );
    let mut reconciled = false;
    for _ in 0..100 {
        let reconciling: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM replay_progress p JOIN devices d ON d.device_id=p.device_id AND d.boot_id=p.boot_id WHERE p.device_id=? AND (p.complete=0 OR (p.through_device_seq IS NULL AND EXISTS (SELECT 1 FROM device_events e WHERE e.device_id=p.device_id AND e.boot_id=p.boot_id AND e.origin='offline_replay')))",
        )
        .bind(&device)
        .fetch_one(&h.sqlite)
        .await?;
        if reconciling == 0 {
            reconciled = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    ensure!(
        reconciled,
        "simulator {device} did not finish reconciliation"
    );
    let profile = if automatic {
        let created = api_ok(
            h,
            Method::POST,
            "/api/v1/profiles",
            // The numbers have to be coherent with the *modelled pot*, not
            // merely plausible on their own. The simulator's soil model holds
            // 2500 ml and converts volume to moisture at `100 / pot_volume`, so
            // one millilitre is 0.04 VWC and a 60 ml dose is +2.4. Recovery is
            // judged against the reading taken before the cycle's *first* dose,
            // so two doses clear the 3.0 VWC threshold and one does not:
            // deliberately, because SCEN-002's documented sequence and the
            // eighteen-step demo both require the recheck-and-dose-again path.
            //
            // An earlier 40 ml dose made the cycle unsatisfiable — +1.6 VWC per
            // dose against a 3.0 threshold, cumulative +4.8 over the three doses
            // `max_doses_per_cycle` allows, but the drying between them ate the
            // margin and the plant reached `max_doses_reached` every time. That
            // is the machine behaving correctly on an incoherent policy, which
            // is exactly the kind of thing a scenario should not be built on.
            json!({
                "profile_id":"scenario-profile", "name":"Scenario profile",
                "target_min_vwc":28.0, "target_max_vwc":45.0, "dose_ml":SCENARIO_DOSE_ML,
                "max_doses_per_cycle":3, "max_daily_ml":300.0,
                "dry_confirm_minutes":30, "cooldown_hours":6.0,
                "absorption_minutes":15, "recovery_delta_vwc":3.0,
                "tank_min_percent":15.0, "command_ttl_seconds":600
            }),
        )
        .await?;
        ensure!(created["profile_id"] == "scenario-profile");
        "scenario-profile"
    } else {
        "default"
    };
    api_ok(h, Method::POST, "/api/v1/plants", json!({
        "plant_id":"scenario-plant", "name":"Scenario plant", "profile_id":profile, "pot_volume_ml":2000.0
    })).await?;
    // Bound the same way whether or not automation is on. A binding says what
    // the plant *has*, not whether anybody has switched watering on, and the two
    // are independent: an operator wires the tray sensor once and decides about
    // automation afterwards.
    //
    // Binding them only in the automatic case made the non-automatic plant a
    // pot with a pump and no leak sensor, which the shared gate refuses outright
    // — `LeakState::Unknown` is `Uncertain`, fail-closed and correct. SCEN-003
    // then observed a locked-out plant rather than the advice-without-a-command
    // it exists to check, which is a scenario testing the wrong subject. A plant
    // that is genuinely under-provisioned is SCEN-106's business.
    let bindings = [
        ("soil-0", "default", "soil_moisture", "control"),
        ("tank-0", "reservoir", "tank_level", "required"),
        ("leak-0", "tray", "leak_state", "required"),
        ("weight-0", "default", "pot_weight", "advisory"),
    ];
    for (sensor_id, point, kind, role) in bindings {
        api_ok(
            h,
            Method::PUT,
            "/api/v1/plants/scenario-plant/bindings/sensors",
            json!({
                "device_id":device, "sensor_id":sensor_id, "point":point, "kind":kind, "role":role
            }),
        )
        .await?;
    }
    api_ok(
        h,
        Method::PUT,
        "/api/v1/plants/scenario-plant/bindings/actuator",
        json!({
            "device_id":device, "actuator_id":"pump-0"
        }),
    )
    .await?;
    api_ok(
        h,
        Method::PUT,
        "/api/v1/plants/scenario-plant/measurement-policies/soil_moisture",
        json!({
            "target_min":28.0, "target_max":45.0, "stale_after_ms":900000,
            "confirm_duration_ms":1800000, "hysteresis":1.0
        }),
    )
    .await?;
    // Every `required` binding needs a policy, not only the control stream.
    // `plant::analyse` treats a required binding with no policy as unhealthy:
    // declaring a measurement required is a claim that watering depends on it,
    // and there is no defensible default staleness for a claim like that
    // (SAFETY-017). Without these two the recommendation engine reported
    // `sensor_unhealthy` and blocked for ever, while the irrigation gate —
    // which derives its own freshness bound from the telemetry cadence — went
    // on watering. Two surfaces disagreeing about the same plant is exactly the
    // kind of thing the assembled suite exists to catch, and a scenario must
    // not bake it in.
    for (kind, policy) in [
        (
            "tank_level",
            json!({ "target_min":20.0, "stale_after_ms":900000 }),
        ),
        ("leak_state", json!({ "stale_after_ms":900000 })),
    ] {
        api_ok(
            h,
            Method::PUT,
            &format!("/api/v1/plants/scenario-plant/measurement-policies/{kind}"),
            policy,
        )
        .await?;
    }
    Ok(device)
}

async fn setup_battery_plant(h: &Harness) -> Result<String> {
    h.reset_battery_simulator().await?;
    let device = "battery-node-01".to_owned();
    for _ in 0..400 {
        let devices = h
            .get_json(&format!("{}/api/v1/devices", h.edge_url))
            .await?;
        if devices["devices"].as_array().is_some_and(|rows| {
            rows.iter().any(|row| {
                row["device_id"] == device
                    && row["power_mode"] == "battery"
                    && matches!(row["connectivity"].as_str(), Some("connected" | "sleeping"))
            })
        }) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    api_ok(
        h,
        Method::POST,
        "/api/v1/profiles",
        json!({
            "profile_id":"battery-profile", "name":"Battery profile",
            "target_min_vwc":28.0, "target_max_vwc":45.0, "dose_ml":SCENARIO_DOSE_ML,
            "max_doses_per_cycle":3, "max_daily_ml":300.0,
            "dry_confirm_minutes":30, "cooldown_hours":6.0,
            "absorption_minutes":15, "recovery_delta_vwc":3.0,
            "tank_min_percent":15.0, "command_ttl_seconds":600
        }),
    )
    .await?;
    api_ok(h, Method::POST, "/api/v1/plants", json!({
        "plant_id":"battery-plant", "name":"Battery plant", "profile_id":"battery-profile", "pot_volume_ml":2000.0
    })).await?;
    for (sensor_id, point, kind, role) in [
        ("soil-0", "default", "soil_moisture", "control"),
        ("tank-0", "reservoir", "tank_level", "required"),
        ("leak-0", "tray", "leak_state", "required"),
        ("weight-0", "default", "pot_weight", "advisory"),
    ] {
        api_ok(
            h,
            Method::PUT,
            "/api/v1/plants/battery-plant/bindings/sensors",
            json!({"device_id":device,"sensor_id":sensor_id,"point":point,"kind":kind,"role":role}),
        )
        .await?;
    }
    api_ok(
        h,
        Method::PUT,
        "/api/v1/plants/battery-plant/bindings/actuator",
        json!({"device_id":device,"actuator_id":"pump-0"}),
    )
    .await?;
    api_ok(
        h,
        Method::PUT,
        "/api/v1/plants/battery-plant/measurement-policies/soil_moisture",
        json!({"target_min":28.0,"target_max":45.0,"stale_after_ms":1800000,"confirm_duration_ms":1800000,"hysteresis":1.0}),
    )
    .await?;
    Ok(device)
}

async fn wait_battery_sleep(h: &Harness) -> Result<()> {
    for _ in 0..800 {
        let announced = h.mqtt().await.iter().any(|message| {
            message.topic == "rhizo/v1/devices/battery-node-01/status"
                && serde_json::from_slice::<Value>(&message.payload).is_ok_and(|body| {
                    body["data"]["status"] == "offline" && body["data"]["reason"] == "sleeping"
                })
        });
        let mode: Option<String> = sqlx::query_scalar(
            "SELECT connectivity_mode FROM devices WHERE device_id='battery-node-01'",
        )
        .fetch_optional(&h.sqlite)
        .await?;
        // The announcement is the wire-level observation and the current
        // database row proves Edge consumed it. Requiring both closes the race
        // where a request made just after the next wake would route directly.
        if announced && mode.as_deref() == Some("sleeping") {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    bail!("battery device did not announce sleep")
}

async fn request_battery_water(h: &Harness, ml: f64) -> Result<Value> {
    request_battery_water_mode(h, ml, "manual").await
}

async fn request_battery_water_mode(h: &Harness, ml: f64, mode: &str) -> Result<Value> {
    let (status, body) = h
        .json(
            Method::POST,
            &format!("{}/api/v1/plants/battery-plant/water", h.edge_url),
            json!({"ml":ml,"mode":mode}),
        )
        .await?;
    ensure!(status == reqwest::StatusCode::ACCEPTED, "{status}: {body}");
    ensure!(body["status"] == "pending_for_device_wake");
    ensure!(body.get("command_id").is_none());
    Ok(body)
}

/// Whether a captured topic is a command the **edge** issued to a device.
///
/// `topic.contains("/commands/")` is not that, and the difference is not
/// cosmetic: `commands/result` is a *device* publication, it lives under the
/// same path segment, and the device republishes it every retry interval until
/// an acknowledgement names it. Counting those as commands turns "the gate
/// refused to water" into "seven commands were published", which is a negative
/// safety assertion failing on the noise it was supposed to ignore.
///
/// The edge-to-device command topics are `commands/water`, `commands/tare`,
/// `commands/calibrate`, and `commands/result/ack`; only the first three can
/// actuate anything (protocol §3).
fn is_edge_command(topic: &str) -> bool {
    topic.ends_with("/commands/water")
        || topic.ends_with("/commands/tare")
        || topic.ends_with("/commands/calibrate")
}

async fn api_ok(h: &Harness, method: Method, path: &str, body: Value) -> Result<Value> {
    let url = format!("{}{}", h.edge_url, path);
    let (status, value) = h.json(method, &url, body).await?;
    ensure!(status.is_success(), "{url} returned {status}: {value}");
    Ok(value)
}

fn recommendation_without_automation<'a>(h: &'a Harness) -> ScenarioFuture<'a> {
    Box::pin(async move {
        setup_plant(h, false).await?;
        h.simulator_post("/sim/state", json!({"moisture_vwc":20.0}))
            .await?;
        for _ in 0..200 {
            let plant = h
                .get_json(&format!("{}/api/v1/plants/scenario-plant", h.edge_url))
                .await?;
            if plant["state"] == "water_recommended" {
                let recommendation = h
                    .get_json(&format!(
                        "{}/api/v1/plants/scenario-plant/recommendation",
                        h.edge_url
                    ))
                    .await?;
                let reasons = recommendation["reasons"]
                    .as_array()
                    .context("recommendation reasons")?;
                ensure!(reasons.iter().any(|v| v["code"] == "moisture_below_target"));
                ensure!(reasons.iter().any(|v| v["code"] == "dry_for"));
                let commands = h
                    .mqtt()
                    .await
                    .into_iter()
                    .filter(|m| is_edge_command(&m.topic))
                    .count();
                ensure!(
                    commands == 0,
                    "automation-off scenario published {commands} MQTT commands"
                );
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        bail!("plant did not reach water_recommended")
    })
}

fn full_watering_cycle<'a>(h: &'a Harness) -> ScenarioFuture<'a> {
    Box::pin(async move {
        setup_plant(h, true).await?;
        h.simulator_post(
            "/sim/state",
            json!({"moisture_vwc":42.0,"tank_percent":100.0,"leak":"clear"}),
        )
        .await?;
        tokio::time::sleep(Duration::from_millis(700)).await;
        api_ok(
            h,
            Method::POST,
            "/api/v1/plants/scenario-plant/auto-watering/enable",
            json!({}),
        )
        .await?;
        tokio::time::sleep(Duration::from_millis(200)).await;
        h.simulator_post("/sim/state", json!({"moisture_vwc":26.0}))
            .await?;
        for _ in 0..1000 {
            let events: i64 = sqlx::query_scalar("SELECT count(*) FROM watering_events WHERE plant_id='scenario-plant' AND status IN ('completed','accepted','success')")
                .fetch_one(&h.sqlite).await?;
            let state: Option<String> = sqlx::query_scalar(
                "SELECT state FROM irrigation_state WHERE plant_id='scenario-plant'",
            )
            .fetch_optional(&h.sqlite)
            .await?;
            if events > 0 && state.as_deref() == Some("normal") {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        let details: Vec<String> = sqlx::query_scalar(
            "SELECT detail_json FROM plant_events WHERE plant_id='scenario-plant' AND kind='irrigation_state_changed' ORDER BY occurred_at,event_id",
        )
        .fetch_all(&h.sqlite)
        .await?;
        let mut observed = vec!["normal".to_owned()];
        for detail in details {
            let value: Value = serde_json::from_str(&detail)?;
            observed.push(
                value["to"]
                    .as_str()
                    .context("transition target")?
                    .to_owned(),
            );
        }
        let required = [
            "normal",
            "drying",
            "dry_confirmed",
            "dose_issued",
            "wait_for_absorption",
            "recheck",
            "normal",
        ];
        let mut at = 0;
        for state in &observed {
            if at < required.len() && state == required[at] {
                at += 1;
            }
        }
        ensure!(
            at == required.len(),
            "irrigation sequence incomplete: {observed:?}"
        );
        let (events, delivered, unmatched): (i64, f64, i64) = sqlx::query_as(
            "SELECT count(*), coalesce(sum(w.delivered_ml),0), sum(CASE WHEN c.status NOT IN ('completed','accepted','success') OR c.command_id IS NULL THEN 1 ELSE 0 END) FROM watering_events w LEFT JOIN commands c ON c.command_id=w.command_id WHERE w.plant_id='scenario-plant'"
        ).fetch_one(&h.sqlite).await?;
        ensure!(
            (1..=3).contains(&events),
            "dose count {events} outside 1..=3"
        );
        ensure!(
            delivered <= 300.0,
            "delivered {delivered} ml exceeds daily maximum"
        );
        ensure!(
            unmatched == 0,
            "{unmatched} watering events lack a terminal command"
        );
        Ok(())
    })
}

fn duplicate_command<'a>(h: &'a Harness) -> ScenarioFuture<'a> {
    Box::pin(async move {
        setup_plant(h, true).await?;
        h.simulator_post(
            "/sim/state",
            json!({"moisture_vwc":40.0,"tank_percent":100.0,"leak":"clear"}),
        )
        .await?;
        tokio::time::sleep(Duration::from_millis(700)).await;
        api_ok(
            h,
            Method::POST,
            "/api/v1/plants/scenario-plant/water",
            json!({"ml":40.0,"mode":"manual"}),
        )
        .await?;
        let command = wait_mqtt(h, |m| m.topic.ends_with("/commands/water"), 200).await?;
        let first = wait_mqtt(
            h,
            |m| {
                m.topic.ends_with("/commands/result")
                    && serde_json::from_slice::<Value>(&m.payload)
                        .is_ok_and(|v| v["data"]["status"] == "completed")
            },
            400,
        )
        .await?;
        ensure!(serde_json::from_slice::<Value>(&first.payload)?["data"]["delivered_ml"] == 40.0);
        wait_mqtt(h, |m| m.topic.ends_with("/commands/result/ack"), 200).await?;
        for _ in 0..2 {
            h.clear_mqtt().await;
            h.publish(&command.topic, command.payload.clone()).await?;
            let result = wait_mqtt(h, |m| m.topic.ends_with("/commands/result"), 200).await?;
            let payload: Value = serde_json::from_slice(&result.payload)?;
            ensure!(
                payload["data"]["status"] == "completed" && payload["data"]["delivered_ml"] == 40.0,
                "duplicate did not replay stored terminal result: {payload}"
            );
            wait_mqtt(h, |m| m.topic.ends_with("/commands/result/ack"), 200).await?;
            // Wait for the *device* to report an empty ledger before opening
            // the quiet window. A `command.result` is retired when an
            // acknowledgement names it, but the device's republish timer is
            // free-running: seeing the acknowledgement on the wire says the
            // edge sent one, not that the device has consumed it, and a
            // retransmission queued a moment earlier still arrives afterwards.
            //
            // `pending_results` is the retirement itself, so this asserts the
            // property the acknowledgement exists for — once the device has
            // retired the result it stops publishing it — instead of sampling a
            // window and hoping the race fell the right way.
            wait_simulator(h, 400, |state| state["pending_results"].as_u64() == Some(0)).await?;
            // Drain, *then* open the window. A republish already handed to the
            // client before the acknowledgement was consumed is in flight and
            // will arrive; it was sent before the retirement and says nothing
            // about what follows it.
            tokio::time::sleep(Duration::from_millis(150)).await;
            h.clear_mqtt().await;
            tokio::time::sleep(Duration::from_millis(150)).await;
            let after_ack = h
                .mqtt()
                .await
                .into_iter()
                .filter(|m| m.topic.ends_with("/commands/result"))
                .count();
            ensure!(
                after_ack == 0,
                "device kept retransmitting a stored result after its acknowledgement"
            );
        }
        let events: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM watering_events WHERE plant_id='scenario-plant'",
        )
        .fetch_one(&h.sqlite)
        .await?;
        ensure!(
            events == 1,
            "duplicate command created {events} watering events"
        );
        let state = h
            .get_json(&format!("{}/sim/state", h.simulator_url))
            .await?;
        ensure!(
            state["delivered_today_ml"]
                .as_f64()
                .is_some_and(|v| (v - 40.0).abs() < 0.01),
            "device daily total was {}",
            state["delivered_today_ml"]
        );
        Ok(())
    })
}

async fn wait_mqtt(
    h: &Harness,
    predicate: impl Fn(&crate::harness::CapturedMqtt) -> bool,
    attempts: usize,
) -> Result<crate::harness::CapturedMqtt> {
    for _ in 0..attempts {
        if let Some(message) = h.mqtt().await.into_iter().find(&predicate) {
            return Ok(message);
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    // The topics that *were* seen, not a bare timeout: PRD 080 asks for the last
    // known state on a failure, and "an expected publication was not captured"
    // says nothing about whether the device was silent, busy with something
    // else, or publishing the right thing to the wrong topic.
    let mut topics: Vec<String> = h
        .mqtt()
        .await
        .into_iter()
        .map(|message| message.topic)
        .collect();
    topics.sort_unstable();
    topics.dedup();
    bail!("expected MQTT publication was not captured; topics seen: {topics:?}")
}

fn broker_restart<'a>(h: &'a Harness) -> ScenarioFuture<'a> {
    Box::pin(async move {
        setup_plant(h, true).await?;
        h.simulator_post(
            "/sim/state",
            json!({"moisture_vwc":40.0,"tank_percent":100.0,"leak":"clear"}),
        )
        .await?;
        tokio::time::sleep(Duration::from_millis(700)).await;
        api_ok(
            h,
            Method::POST,
            "/api/v1/plants/scenario-plant/auto-watering/enable",
            json!({}),
        )
        .await?;
        h.stop_service("mosquitto")?;
        // Let the edge finish what it had already read before sampling.
        // Stopping the broker ends *delivery*; the messages in the ingestion
        // channel are still the edge's to commit, and at this time scale the
        // few hundred milliseconds that takes is hours of virtual time. The
        // assertion below is an exact equality, so it has to start from a count
        // that is no longer moving for a reason unrelated to the outage.
        let during = wait_batches_settled(h).await?;
        // Well past `max_sample_age` at the overlay's scale: 500 ms is thirty
        // virtual minutes against a fifteen-minute threshold, which is also what
        // makes the lockout below inevitable rather than lucky.
        tokio::time::sleep(Duration::from_millis(500)).await;
        let still: i64 = sqlx::query_scalar("SELECT count(DISTINCT batch_id) FROM measurements")
            .fetch_one(&h.sqlite)
            .await?;
        ensure!(
            still == during,
            "telemetry advanced while the broker was stopped: {during} -> {still}"
        );
        let outage_lockout: Option<String> =
            sqlx::query_scalar("SELECT lockout_reason FROM plants WHERE plant_id='scenario-plant'")
                .fetch_one(&h.sqlite)
                .await?;
        h.start_service("mosquitto")?;
        ensure!(
            outage_lockout.is_some(),
            "outage beyond max_sample_age did not lock the plant"
        );
        for _ in 0..300 {
            let after: i64 =
                sqlx::query_scalar("SELECT count(DISTINCT batch_id) FROM measurements")
                    .fetch_one(&h.sqlite)
                    .await?;
            if after > during {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        let after: i64 = sqlx::query_scalar("SELECT count(DISTINCT batch_id) FROM measurements")
            .fetch_one(&h.sqlite)
            .await?;
        ensure!(
            after > during,
            "telemetry did not resume after broker restart"
        );
        let duplicates: i64 = sqlx::query_scalar("SELECT count(*) - count(DISTINCT device_id || ':' || batch_id || ':' || sample_index) FROM measurements WHERE sample_index IS NOT NULL").fetch_one(&h.sqlite).await?;
        ensure!(
            duplicates == 0,
            "broker restart corrupted telemetry uniqueness"
        );
        let retained_status = h
            .mqtt()
            .await
            .into_iter()
            .any(|m| m.retain && m.topic.ends_with("/status"));
        ensure!(
            retained_status,
            "retained device status was not redelivered after broker restart"
        );
        for _ in 0..200 {
            let lockout: Option<String> = sqlx::query_scalar(
                "SELECT lockout_reason FROM plants WHERE plant_id='scenario-plant'",
            )
            .fetch_one(&h.sqlite)
            .await?;
            if lockout.is_none() {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        bail!("outage lockout did not auto-clear after telemetry resumed")
    })
}

async fn fault(h: &Harness, name: &str, enabled: bool) -> Result<Value> {
    h.simulator_post("/sim/fault", json!({"fault":name,"enabled":enabled}))
        .await
}

async fn wait_lockout(h: &Harness, expected: Option<&str>, attempts: usize) -> Result<()> {
    for _ in 0..attempts {
        let reason: Option<String> =
            sqlx::query_scalar("SELECT lockout_reason FROM plants WHERE plant_id='scenario-plant'")
                .fetch_one(&h.sqlite)
                .await?;
        if reason.as_deref() == expected {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let actual: Option<String> =
        sqlx::query_scalar("SELECT lockout_reason FROM plants WHERE plant_id='scenario-plant'")
            .fetch_one(&h.sqlite)
            .await?;
    bail!("expected lockout {expected:?}, observed {actual:?}")
}

fn stale_sensor<'a>(h: &'a Harness) -> ScenarioFuture<'a> {
    Box::pin(async move {
        setup_plant(h, true).await?;
        h.simulator_post(
            "/sim/state",
            json!({"moisture_vwc":20.0,"tank_percent":100.0,"leak":"clear"}),
        )
        .await?;
        tokio::time::sleep(Duration::from_millis(700)).await;
        api_ok(
            h,
            Method::POST,
            "/api/v1/plants/scenario-plant/auto-watering/enable",
            json!({}),
        )
        .await?;
        // A device whose clock is a day ahead, still publishing. SAFETY-005 says
        // freshness is judged by the **edge's** `received_at` and never by a
        // timestamp the device chose, and this is what makes the two
        // distinguishable: withholding the stream alone stops both clocks
        // together, so the scenario would pass against a build that trusted
        // either one. M8-013's second mutation stayed green until this step
        // existed.
        //
        // A day, because the assertion below has to outlast the lie. The
        // staleness threshold is 900 logical seconds; a build dating samples by
        // `device_time_ms` would call this sample fresh for 86 400 more, which
        // is far past the window `wait_lockout` watches.
        fault(h, "clock-skew:86400", true).await?;
        let skewed = wait_measurement_newer_than(h, "soil_moisture", now_ms(h).await?).await?;
        ensure!(
            skewed,
            "no soil sample arrived while the device clock was skewed"
        );

        fault(h, "stale-soil", true).await?;
        wait_lockout(h, Some("stale_data"), 200).await?;
        // The quiet window opens *after* the lockout, which is what SCEN-022
        // actually claims: "after `max_sample_age`, the plant enters
        // `Lock(StaleData)`; no command is issued". Opening it at the moment
        // the fault is injected asserts something else and something false —
        // the last reading is still fresh then, the plant is still dry, and an
        // edge that issued a dose on it would be behaving correctly. SAFETY-005
        // is about acting on data that has *aged out*, not about the instant a
        // sensor fell silent.
        h.clear_mqtt().await;
        tokio::time::sleep(Duration::from_millis(250)).await;
        let commands = h
            .mqtt()
            .await
            .into_iter()
            .filter(|m| is_edge_command(&m.topic))
            .count();
        ensure!(
            commands == 0,
            "stale sensor allowed {commands} command publications"
        );
        fault(h, "stale-soil", false).await?;
        fault(h, "clock-skew:0", false).await?;
        wait_lockout(h, None, 200).await?;
        Ok(())
    })
}

fn invalid_sensor<'a>(h: &'a Harness) -> ScenarioFuture<'a> {
    Box::pin(async move {
        setup_plant(h, true).await?;
        h.simulator_post(
            "/sim/state",
            json!({"moisture_vwc":20.0,"tank_percent":100.0,"leak":"clear"}),
        )
        .await?;
        fault(h, "invalid-soil:1", true).await?;
        tokio::time::sleep(Duration::from_millis(100)).await;
        api_ok(
            h,
            Method::POST,
            "/api/v1/plants/scenario-plant/auto-watering/enable",
            json!({}),
        )
        .await?;
        h.clear_mqtt().await;
        wait_lockout(h, Some("sensor_fault"), 200).await?;
        for _ in 0..200 {
            let events: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM device_events WHERE kind='sensor_invalid'",
            )
            .fetch_one(&h.sqlite)
            .await?;
            if events > 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        let nulled: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM measurements WHERE kind='soil_moisture' AND value_num IS NULL",
        )
        .fetch_one(&h.sqlite)
        .await?;
        let events: i64 =
            sqlx::query_scalar("SELECT count(*) FROM device_events WHERE kind='sensor_invalid'")
                .fetch_one(&h.sqlite)
                .await?;
        ensure!(
            nulled > 0 && events > 0,
            "invalid soil was not nulled and surfaced: nulled={nulled}, events={events}"
        );
        let commands = h
            .mqtt()
            .await
            .into_iter()
            .filter(|m| m.topic.ends_with("/commands/water"))
            .count();
        ensure!(
            commands == 0,
            "invalid sensor allowed {commands} water commands"
        );
        Ok(())
    })
}

fn clock_unsynced<'a>(h: &'a Harness) -> ScenarioFuture<'a> {
    Box::pin(async move {
        setup_plant(h, true).await?;
        let before: i64 = sqlx::query_scalar("SELECT count(DISTINCT batch_id) FROM measurements")
            .fetch_one(&h.sqlite)
            .await?;
        fault(h, "clock-unsync", true).await?;
        h.simulator_post("/sim/restart", json!({})).await?;
        for _ in 0..200 {
            let state = h
                .get_json(&format!("{}/sim/state", h.simulator_url))
                .await?;
            if state["connected"] == true && state["clock_synced"] == false {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        h.simulator_post(
            "/sim/state",
            json!({"moisture_vwc":40.0,"tank_percent":100.0,"leak":"clear"}),
        )
        .await?;
        tokio::time::sleep(Duration::from_millis(600)).await;
        h.clear_mqtt().await;
        let _ = h
            .json(
                Method::POST,
                &format!("{}/api/v1/plants/scenario-plant/water", h.edge_url),
                json!({"ml":40.0,"mode":"manual"}),
            )
            .await?;
        wait_lockout(h, Some("clock_unsynced"), 200).await?;
        let rejected = wait_mqtt(
            h,
            |m| {
                m.topic.ends_with("/commands/result")
                    && serde_json::from_slice::<Value>(&m.payload)
                        .is_ok_and(|v| v["data"]["reason"] == "clock_unsynced")
            },
            200,
        )
        .await?;
        ensure!(!rejected.payload.is_empty());
        let after: i64 = sqlx::query_scalar("SELECT count(DISTINCT batch_id) FROM measurements")
            .fetch_one(&h.sqlite)
            .await?;
        ensure!(
            after > before,
            "telemetry stopped while the clock was unsynchronised"
        );
        Ok(())
    })
}

fn queued_command_expiry<'a>(h: &'a Harness) -> ScenarioFuture<'a> {
    Box::pin(async move {
        let device = setup_plant(h, true).await?;
        let mut issued_at = None;
        for _ in 0..200 {
            issued_at =
                sqlx::query_scalar("SELECT max(received_at) FROM measurements WHERE device_id=?")
                    .bind(&device)
                    .fetch_one(&h.sqlite)
                    .await?;
            if issued_at.is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        let issued_at: i64 = issued_at.context("no telemetry timestamp for expiry command")?;
        let expires_at = issued_at + 120_000;
        let command_id = uuid::Uuid::now_v7().to_string();
        let payload = serde_json::to_vec(&json!({
            "v": 1,
            "kind": "command.water",
            "message_id": uuid::Uuid::now_v7().to_string(),
            "device_id": device,
            "data": {
                "command_id": command_id,
                "requested_ml": 40.0,
                "issued_at_ms": issued_at,
                "expires_at_ms": expires_at
            }
        }))?;
        let topic = format!("rhizo/v1/devices/{device}/commands/water");
        let delivered_before = h
            .get_json(&format!("{}/sim/state", h.simulator_url))
            .await?["delivered_today_ml"]
            .as_f64()
            .context("simulator delivered_today_ml")?;

        h.stop_service("device-simulator")?;
        tokio::time::sleep(Duration::from_millis(150)).await;
        h.clear_mqtt().await;
        h.publish(&topic, payload.clone()).await?;
        // The wait only has to exceed the command's two-minute lifetime in
        // *virtual* time while the clean-session subscriber is absent, and the
        // overlay runs at 3600x — so 300 ms of real time is several hours of it.
        // Generous on purpose: the property is that an expired command is never
        // executed, and cutting the margin fine would test the margin instead.
        tokio::time::sleep(Duration::from_millis(300)).await;
        h.start_service("device-simulator")?;
        h.wait_simulator_ready().await?;
        for _ in 0..200 {
            let state = h
                .get_json(&format!("{}/sim/state", h.simulator_url))
                .await?;
            if state["connected"] == true && state["clock_synced"] == true {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
        let queued_result = h.mqtt().await.into_iter().any(|message| {
            message.topic.ends_with("/commands/result")
                && serde_json::from_slice::<Value>(&message.payload)
                    .is_ok_and(|value| value["data"]["command_id"] == command_id)
        });
        ensure!(
            !queued_result,
            "broker delivered a command to a clean session"
        );
        let delivered_after_reconnect = h
            .get_json(&format!("{}/sim/state", h.simulator_url))
            .await?["delivered_today_ml"]
            .as_f64()
            .context("simulator delivered_today_ml after reconnect")?;
        ensure!(
            (delivered_after_reconnect - delivered_before).abs() < f64::EPSILON,
            "queued command actuated after reconnect"
        );

        // Deliver the exact expired command explicitly: the independent device
        // gate must still reject it if broker session semantics ever regress.
        h.publish(&topic, payload).await?;
        wait_mqtt(
            h,
            |message| {
                message.topic.ends_with("/commands/result")
                    && serde_json::from_slice::<Value>(&message.payload).is_ok_and(|value| {
                        value["data"]["command_id"] == command_id
                            && value["data"]["status"] == "rejected"
                            && value["data"]["reason"] == "expired"
                    })
            },
            200,
        )
        .await?;
        let watering: i64 = sqlx::query_scalar("SELECT count(*) FROM watering_events")
            .fetch_one(&h.sqlite)
            .await?;
        ensure!(watering == 0, "expired command created a watering event");
        Ok(())
    })
}

fn leak<'a>(h: &'a Harness) -> ScenarioFuture<'a> {
    Box::pin(async move {
        setup_plant(h, true).await?;
        h.simulator_post(
            "/sim/state",
            json!({"moisture_vwc":20.0,"tank_percent":100.0,"leak":"detected"}),
        )
        .await?;
        api_ok(
            h,
            Method::POST,
            "/api/v1/plants/scenario-plant/auto-watering/enable",
            json!({}),
        )
        .await?;
        wait_lockout(h, Some("leak"), 300).await?;
        h.clear_mqtt().await;
        let (status, body) = h
            .json(
                Method::POST,
                &format!("{}/api/v1/plants/scenario-plant/water", h.edge_url),
                json!({"ml":40.0,"mode":"manual"}),
            )
            .await?;
        ensure!(
            status == reqwest::StatusCode::CONFLICT && body.to_string().contains("leak"),
            "manual leak refusal was {status}: {body}"
        );
        tokio::time::sleep(Duration::from_millis(250)).await;
        ensure!(
            !h.mqtt()
                .await
                .iter()
                .any(|message| is_edge_command(&message.topic)),
            "a command was published while the leak lockout was active"
        );
        let (wet_status, _) = h
            .json(
                Method::POST,
                &format!("{}/api/v1/plants/scenario-plant/lockout/clear", h.edge_url),
                json!({"reason":"leak"}),
            )
            .await?;
        ensure!(
            wet_status == reqwest::StatusCode::CONFLICT,
            "leak lockout cleared while the tray was wet"
        );
        h.simulator_post("/sim/state", json!({"leak":"clear"}))
            .await?;
        tokio::time::sleep(Duration::from_millis(600)).await;
        api_ok(
            h,
            Method::POST,
            "/api/v1/plants/scenario-plant/lockout/clear",
            json!({"reason":"leak"}),
        )
        .await?;
        wait_lockout(h, None, 100).await?;
        Ok(())
    })
}

async fn direct_water_payload(h: &Harness, device: &str) -> Result<(String, Vec<u8>)> {
    let now: Option<i64> =
        sqlx::query_scalar("SELECT max(received_at) FROM measurements WHERE device_id=?")
            .bind(device)
            .fetch_one(&h.sqlite)
            .await?;
    let now = now.context("no telemetry timestamp for direct water command")?;
    let command_id = uuid::Uuid::now_v7().to_string();
    Ok((
        command_id.clone(),
        serde_json::to_vec(&json!({
            "v":1, "kind":"command.water",
            "message_id":uuid::Uuid::now_v7().to_string(), "device_id":device,
            "data":{"command_id":command_id,"requested_ml":40.0,
                "issued_at_ms":now,"expires_at_ms":now + 600_000}
        }))?,
    ))
}

fn tank_empty<'a>(h: &'a Harness) -> ScenarioFuture<'a> {
    Box::pin(async move {
        let device = setup_plant(h, true).await?;
        h.simulator_post("/sim/state", json!({"tank_percent":0.0,"leak":"clear"}))
            .await?;
        api_ok(
            h,
            Method::POST,
            "/api/v1/plants/scenario-plant/auto-watering/enable",
            json!({}),
        )
        .await?;
        wait_lockout(h, Some("tank_low"), 300).await?;
        let (command_id, payload) = direct_water_payload(h, &device).await?;
        h.clear_mqtt().await;
        h.publish(
            &format!("rhizo/v1/devices/{device}/commands/water"),
            payload,
        )
        .await?;
        wait_mqtt(
            h,
            |message| {
                message.topic.ends_with("/commands/result")
                    && serde_json::from_slice::<Value>(&message.payload).is_ok_and(|value| {
                        value["data"]["command_id"] == command_id
                            && value["data"]["status"] == "rejected"
                            && value["data"]["reason"] == "tank_low"
                    })
            },
            200,
        )
        .await?;
        h.simulator_post("/sim/state", json!({"tank_percent":100.0}))
            .await?;
        wait_lockout(h, None, 300).await?;
        Ok(())
    })
}

fn no_delivery<'a>(h: &'a Harness) -> ScenarioFuture<'a> {
    Box::pin(async move {
        setup_plant(h, true).await?;
        h.simulator_post(
            "/sim/state",
            json!({"moisture_vwc":42.0,"tank_percent":100.0,"leak":"clear"}),
        )
        .await?;
        tokio::time::sleep(Duration::from_millis(700)).await;
        fault(h, "pump-no-delivery", true).await?;
        api_ok(
            h,
            Method::POST,
            "/api/v1/plants/scenario-plant/auto-watering/enable",
            json!({}),
        )
        .await?;
        h.clear_mqtt().await;
        h.simulator_post("/sim/state", json!({"moisture_vwc":20.0}))
            .await?;
        wait_lockout(h, Some("no_delivery_detected"), 1200).await?;
        let issued: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM commands WHERE plant_id='scenario-plant' AND kind='water'",
        )
        .fetch_one(&h.sqlite)
        .await?;
        ensure!(
            issued == 2,
            "expected two unresponsive doses, observed {issued}"
        );
        tokio::time::sleep(Duration::from_millis(750)).await;
        let after: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM commands WHERE plant_id='scenario-plant' AND kind='water'",
        )
        .fetch_one(&h.sqlite)
        .await?;
        ensure!(after == 2, "a third dose was issued after no delivery");
        fault(h, "pump-no-delivery", false).await?;
        tokio::time::sleep(Duration::from_millis(650)).await;
        wait_lockout(h, Some("no_delivery_detected"), 1).await?;
        api_ok(
            h,
            Method::POST,
            "/api/v1/plants/scenario-plant/lockout/clear",
            json!({"reason":"no_delivery_detected"}),
        )
        .await?;
        wait_lockout(h, None, 100).await?;
        Ok(())
    })
}

async fn wait_edge_ready(h: &Harness) -> Result<()> {
    for _ in 0..200 {
        if h.get_json(&format!("{}/health/ready", h.edge_url))
            .await
            .is_ok()
        {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    bail!("edge did not become ready after restart")
}

fn restart_mid_command<'a>(h: &'a Harness) -> ScenarioFuture<'a> {
    Box::pin(async move {
        setup_plant(h, true).await?;
        api_ok(
            h,
            Method::PUT,
            "/api/v1/profiles/scenario-profile",
            json!({
                "name":"Restart profile", "target_min_vwc":28.0,
                "target_max_vwc":45.0, "dose_ml":80.0,
                "max_doses_per_cycle":2, "max_daily_ml":500.0,
                "dry_confirm_minutes":30, "cooldown_hours":6.0,
                "absorption_minutes":15, "recovery_delta_vwc":3.0,
                "tank_min_percent":15.0, "command_ttl_seconds":600
            }),
        )
        .await?;
        h.simulator_post(
            "/sim/state",
            json!({"moisture_vwc":42.0,"tank_percent":100.0,"leak":"clear"}),
        )
        .await?;
        tokio::time::sleep(Duration::from_millis(700)).await;
        api_ok(
            h,
            Method::POST,
            "/api/v1/plants/scenario-plant/auto-watering/enable",
            json!({}),
        )
        .await?;
        h.arm_service_fault(
            "edge-controller",
            "/var/lib/rhizo/fault-exit-after-command-publish",
        )?;
        h.clear_mqtt().await;
        h.simulator_post("/sim/state", json!({"moisture_vwc":20.0}))
            .await?;
        let published = wait_mqtt(h, |m| m.topic.ends_with("/commands/water"), 500).await?;
        let command: Value = serde_json::from_slice(&published.payload)?;
        let command_id = command["data"]["command_id"]
            .as_str()
            .context("published command_id")?
            .to_owned();
        let requested_ml = command["data"]["requested_ml"]
            .as_f64()
            .context("published requested_ml")?;
        for _ in 0..200 {
            if !h.service_running("edge-controller")? {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        ensure!(
            !h.service_running("edge-controller")?,
            "edge fault hook did not terminate the process"
        );
        // The spy is deliberately *not* cleared here. SAFETY-010 is a claim
        // about both process lifetimes together — "only one command was ever
        // published" — and a window that starts after the restart cannot
        // distinguish an edge that republished from one that did not, because
        // the count it compares against would be the republished one.
        h.start_service("edge-controller")?;
        wait_edge_ready(h).await?;
        for _ in 0..400 {
            let status: Option<String> =
                sqlx::query_scalar("SELECT status FROM commands WHERE command_id=?")
                    .bind(&command_id)
                    .fetch_optional(&h.sqlite)
                    .await?;
            if status.as_deref() == Some("completed") {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        let publications = h
            .mqtt()
            .await
            .into_iter()
            .filter(|m| m.topic.ends_with("/commands/water"))
            .count();
        ensure!(
            publications == 1,
            "SAFETY-010: {publications} water commands were published across the crash and the \
             restart, and exactly one may ever be"
        );
        let row: (String, i64, f64) = sqlx::query_as(
            "SELECT c.status,count(w.watering_event_id),coalesce(sum(w.delivered_ml),0) FROM commands c LEFT JOIN watering_events w ON w.command_id=c.command_id WHERE c.command_id=? GROUP BY c.status",
        )
        .bind(&command_id)
        .fetch_one(&h.sqlite)
        .await?;
        ensure!(
            row.0 == "completed" && row.1 == 1 && (row.2 - requested_ml).abs() < 0.01,
            "late result did not settle exactly once: {row:?}"
        );
        let state: (String, Option<i64>) = sqlx::query_as(
            "SELECT state,wait_until FROM irrigation_state WHERE plant_id='scenario-plant'",
        )
        .fetch_one(&h.sqlite)
        .await?;
        ensure!(
            state.0 == "wait_for_absorption" && state.1.is_some(),
            "restart lost absorption state: {state:?}"
        );
        Ok(())
    })
}

fn restart_mid_absorption<'a>(h: &'a Harness) -> ScenarioFuture<'a> {
    Box::pin(async move {
        setup_plant(h, true).await?;
        h.simulator_post(
            "/sim/state",
            json!({"moisture_vwc":40.0,"tank_percent":100.0,"leak":"clear"}),
        )
        .await?;
        tokio::time::sleep(Duration::from_millis(700)).await;
        api_ok(
            h,
            Method::POST,
            "/api/v1/plants/scenario-plant/auto-watering/enable",
            json!({}),
        )
        .await?;
        api_ok(
            h,
            Method::POST,
            "/api/v1/plants/scenario-plant/water",
            json!({"ml":40.0,"mode":"manual"}),
        )
        .await?;
        let mut before = None;
        for _ in 0..300 {
            let state: Option<(String, Option<i64>, i64)> = sqlx::query_as(
                "SELECT state,wait_until,doses_this_cycle FROM irrigation_state WHERE plant_id='scenario-plant'",
            )
            .fetch_optional(&h.sqlite)
            .await?;
            if state
                .as_ref()
                .is_some_and(|row| row.0 == "wait_for_absorption")
            {
                before = state;
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        let before = before.context("plant never entered absorption wait")?;
        ensure!(before.1.is_some(), "absorption wait had no deadline");
        h.stop_service("edge-controller")?;
        let stopped: (String, Option<i64>, i64) = sqlx::query_as(
            "SELECT state,wait_until,doses_this_cycle FROM irrigation_state WHERE plant_id='scenario-plant'",
        )
        .fetch_one(&h.sqlite)
        .await?;
        ensure!(stopped == before, "stopping edge changed persisted state");
        h.start_service("edge-controller")?;
        wait_edge_ready(h).await?;
        let restored: (String, Option<i64>, i64) = sqlx::query_as(
            "SELECT state,wait_until,doses_this_cycle FROM irrigation_state WHERE plant_id='scenario-plant'",
        )
        .fetch_one(&h.sqlite)
        .await?;
        ensure!(
            restored == before,
            "restart did not restore exact wait state"
        );
        for _ in 0..400 {
            let state: String = sqlx::query_scalar(
                "SELECT state FROM irrigation_state WHERE plant_id='scenario-plant'",
            )
            .fetch_one(&h.sqlite)
            .await?;
            if state == "normal" {
                let events: i64 = sqlx::query_scalar(
                    "SELECT count(*) FROM watering_events WHERE plant_id='scenario-plant'",
                )
                .fetch_one(&h.sqlite)
                .await?;
                ensure!(events == 1, "restart changed the completed dose count");
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        bail!("restored absorption cycle did not complete normally")
    })
}

fn cloud_unavailable<'a>(h: &'a Harness) -> ScenarioFuture<'a> {
    Box::pin(async move {
        h.stop_service("cloud-api")?;
        full_watering_cycle(h).await?;
        let ready = h.get_json(&format!("{}/health/ready", h.edge_url)).await?;
        ensure!(
            ready["status"] == "ready",
            "edge became unready without cloud"
        );
        let pending: i64 = sqlx::query_scalar("SELECT count(*) FROM pending_cloud_events")
            .fetch_one(&h.sqlite)
            .await?;
        let measurements: i64 = sqlx::query_scalar("SELECT count(*) FROM measurements")
            .fetch_one(&h.sqlite)
            .await?;
        ensure!(
            pending > 0 && measurements > 0,
            "local ingestion/outbox did not operate with cloud down"
        );
        Ok(())
    })
}

async fn cloud_safety_run(
    h: &Harness,
) -> Result<(Vec<(f64, String, String, Option<String>)>, Vec<String>)> {
    setup_plant(h, true).await?;
    h.simulator_post(
        "/sim/state",
        json!({"moisture_vwc":42.0,"tank_percent":100.0,"leak":"clear"}),
    )
    .await?;
    tokio::time::sleep(Duration::from_millis(700)).await;
    api_ok(
        h,
        Method::POST,
        "/api/v1/plants/scenario-plant/auto-watering/enable",
        json!({}),
    )
    .await?;
    h.simulator_post("/sim/state", json!({"moisture_vwc":20.0}))
        .await?;
    for _ in 0..800 {
        let events: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM watering_events WHERE plant_id='scenario-plant'",
        )
        .fetch_one(&h.sqlite)
        .await?;
        if events > 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    h.simulator_post("/sim/state", json!({"leak":"detected"}))
        .await?;
    wait_lockout(h, Some("leak"), 300).await?;
    let commands = sqlx::query_as(
        "SELECT requested_ml,mode,status,reason FROM commands WHERE plant_id='scenario-plant' ORDER BY issued_at",
    )
    .fetch_all(&h.sqlite)
    .await?;
    Ok((commands, vec!["leak".to_owned()]))
}

fn cloud_independence<'a>(h: &'a Harness) -> ScenarioFuture<'a> {
    Box::pin(async move {
        let with_cloud = cloud_safety_run(h).await?;
        h.reset_scenario().await?;
        h.stop_service("cloud-api")?;
        let without_cloud = cloud_safety_run(h).await?;
        ensure!(
            with_cloud == without_cloud,
            "cloud availability changed commands or lockouts: up={with_cloud:?}, down={without_cloud:?}"
        );
        ensure!(
            !with_cloud.0.is_empty() && with_cloud.1 == vec!["leak"],
            "differential fixture exercised no command or lockout"
        );
        Ok(())
    })
}

fn cloud_outage_recovery<'a>(h: &'a Harness) -> ScenarioFuture<'a> {
    Box::pin(async move {
        h.stop_service("cloud-api")?;
        h.stop_service("device-simulator")?;
        sqlx::query("DELETE FROM pending_cloud_events")
            .execute(&h.sqlite)
            .await?;
        for index in 0..500 {
            api_ok(
                h,
                Method::POST,
                "/api/v1/plants",
                json!({
                    "plant_id":format!("cloud-{index:03}"),
                    "name":format!("Cloud load {index}"),
                    "profile_id":"default", "pot_volume_ml":2000.0
                }),
            )
            .await?;
        }
        h.stop_service("edge-controller")?;
        // Plant creation can legitimately trigger recommendation/state events
        // on the concurrent control loop. Freeze the documented outage at the
        // first 500 edge-emitted ledger entries so the recovery cardinality is
        // deterministic rather than scheduler-dependent.
        sqlx::query(
            "DELETE FROM pending_cloud_events WHERE event_id NOT IN (SELECT event_id FROM pending_cloud_events ORDER BY created_at,event_id LIMIT 500)",
        )
        .execute(&h.sqlite)
        .await?;
        // The plants were load generators, not recovery subjects. Remove them
        // while the edge is stopped so restarting the control loop cannot add
        // scheduler-dependent state-change events to the frozen outbox.
        sqlx::query("DELETE FROM plants").execute(&h.sqlite).await?;
        let emitted: i64 = sqlx::query_scalar("SELECT count(*) FROM pending_cloud_events")
            .fetch_one(&h.sqlite)
            .await?;
        ensure!(
            emitted == 500,
            "expected exactly 500 emitted events, got {emitted}"
        );
        let sample: (String, String, String, i64) = sqlx::query_as(
            "SELECT event_id,kind,payload_json,created_at FROM pending_cloud_events ORDER BY created_at,event_id LIMIT 1",
        )
        .fetch_one(&h.sqlite)
        .await?;
        // The exact identities being recovered, captured before the edge is
        // restarted. Counting *all* of `synced_events` afterwards would fold in
        // whatever the restarted edge legitimately emits about itself — device
        // status, config, connectivity — and turn "exactly once" into "exactly
        // 500", which is a different and false claim. Exactly-once is a
        // statement about these identities, so these are what get counted.
        let recovering: Vec<String> =
            sqlx::query_scalar("SELECT event_id FROM pending_cloud_events")
                .fetch_all(&h.sqlite)
                .await?;
        h.start_service("cloud-api")?;
        for _ in 0..400 {
            if h.get_json(&format!("{}/health/ready", h.cloud_url))
                .await
                .is_ok()
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        h.get_json(&format!("{}/health/ready", h.cloud_url))
            .await
            .context("cloud did not become ready for recovery")?;
        sqlx::query(
            "UPDATE pending_cloud_events SET status='pending',attempts=0,next_attempt_at=0,last_error=NULL",
        )
        .execute(&h.sqlite)
        .await?;
        h.start_service("edge-controller")?;
        wait_edge_ready(h).await?;
        for _ in 0..8000 {
            let pending: i64 = sqlx::query_scalar("SELECT count(*) FROM pending_cloud_events")
                .fetch_one(&h.sqlite)
                .await?;
            if pending == 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        let pending: i64 = sqlx::query_scalar("SELECT count(*) FROM pending_cloud_events")
            .fetch_one(&h.sqlite)
            .await?;
        ensure!(pending == 0, "cloud outbox did not drain: {pending} remain");
        let (rows, distinct): (i64, i64) = sqlx::query_as(
            "SELECT count(*),count(DISTINCT event_id) FROM synced_events WHERE edge_id='home-01' AND event_id::text = ANY($1)",
        )
        .bind(&recovering)
        .fetch_one(&h.postgres)
        .await?;
        ensure!(
            rows == emitted && distinct == emitted,
            "PostgreSQL ledger mismatch for the recovered events: rows={rows}, \
             distinct={distinct}, emitted={emitted}"
        );
        let occurred_at = chrono::DateTime::from_timestamp_millis(sample.3)
            .context("outbox timestamp outside chrono range")?
            .to_rfc3339();
        let payload: Value = serde_json::from_str(&sample.2)?;
        let (status, response) = h
            .json(
                Method::POST,
                &format!("{}/api/v1/edges/home-01/events", h.cloud_url),
                json!({"events":[{
                    "event_id":sample.0, "kind":sample.1, "occurred_at":occurred_at,
                    "device_id":payload.get("device_id"), "plant_id":payload.get("plant_id"),
                    "payload":payload
                }]}),
            )
            .await?;
        ensure!(
            status.is_success() && response["results"][0]["status"] == "duplicate",
            "re-POST was not duplicate: {status} {response}"
        );
        let after: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM synced_events WHERE edge_id='home-01' AND event_id::text = ANY($1)",
        )
        .bind(&recovering)
        .fetch_one(&h.postgres)
        .await?;
        ensure!(after == rows, "duplicate re-POST created a PostgreSQL row");
        Ok(())
    })
}

fn demo_step(number: u8, message: &str) {
    println!("[{number:02}/18] {message}");
}

fn first_demo<'a>(h: &'a Harness) -> ScenarioFuture<'a> {
    Box::pin(async move {
        let device = setup_plant(h, true).await?;
        let simulator = h
            .get_json(&format!("{}/sim/state", h.simulator_url))
            .await?;
        ensure!(simulator["connected"] == true);
        demo_step(1, "simulator connected to Mosquitto");

        let devices = h
            .get_json(&format!("{}/api/v1/devices", h.edge_url))
            .await?;
        ensure!(devices["devices"].as_array().is_some_and(|rows| {
            rows.iter()
                .any(|row| row["device_id"] == device && row["connectivity"] == "connected")
        }));
        demo_step(2, "edge reports the device online");

        for _ in 0..200 {
            let count: i64 = sqlx::query_scalar("SELECT count(*) FROM measurements")
                .fetch_one(&h.sqlite)
                .await?;
            if count > 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        let telemetry: i64 = sqlx::query_scalar("SELECT count(*) FROM measurements")
            .fetch_one(&h.sqlite)
            .await?;
        ensure!(telemetry > 0);
        demo_step(3, "telemetry is stored in SQLite");

        h.simulator_post(
            "/sim/state",
            json!({"moisture_vwc":42.0,"tank_percent":100.0,"leak":"clear"}),
        )
        .await?;
        tokio::time::sleep(Duration::from_millis(650)).await;
        api_ok(
            h,
            Method::POST,
            "/api/v1/plants/scenario-plant/auto-watering/enable",
            json!({}),
        )
        .await?;
        h.simulator_post("/sim/state", json!({"moisture_vwc":20.0}))
            .await?;
        for _ in 0..200 {
            let latest: Option<f64> = sqlx::query_scalar(
                "SELECT value_num FROM measurements WHERE kind='soil_moisture' ORDER BY received_at DESC,id DESC LIMIT 1",
            )
            .fetch_optional(&h.sqlite)
            .await?
            .flatten();
            if latest.is_some_and(|value| value < 28.0) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        demo_step(4, "simulated soil moisture decreases below target");

        for _ in 0..300 {
            let state: Option<String> = sqlx::query_scalar(
                "SELECT state FROM irrigation_state WHERE plant_id='scenario-plant'",
            )
            .fetch_optional(&h.sqlite)
            .await?;
            if matches!(
                state.as_deref(),
                Some("drying" | "dry_confirmed" | "dose_issued")
            ) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        let dry_events: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM plant_events WHERE plant_id='scenario-plant' AND kind='irrigation_state_changed' AND detail_json LIKE '%drying%'",
        )
        .fetch_one(&h.sqlite)
        .await?;
        ensure!(dry_events > 0, "dry soil transition was not persisted");
        demo_step(5, "edge detects sustained dry soil");

        for _ in 0..300 {
            let recommendation: Option<String> = sqlx::query_scalar(
                "SELECT reasons_json FROM plant_recommendations WHERE plant_id='scenario-plant' ORDER BY evaluated_at DESC,id DESC LIMIT 1",
            )
            .fetch_optional(&h.sqlite)
            .await?;
            if recommendation.is_some_and(|reasons| reasons.contains("dry_for")) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        let recommendations: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM plant_recommendations WHERE plant_id='scenario-plant'",
        )
        .fetch_one(&h.sqlite)
        .await?;
        ensure!(recommendations > 0);
        demo_step(6, "watering recommendation is generated");

        for _ in 0..500 {
            let commands: i64 =
                sqlx::query_scalar("SELECT count(*) FROM commands WHERE plant_id='scenario-plant'")
                    .fetch_one(&h.sqlite)
                    .await?;
            if commands >= 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        let first_command: (String, String) = sqlx::query_as(
            "SELECT command_id,status FROM commands WHERE plant_id='scenario-plant' ORDER BY issued_at LIMIT 1",
        )
        .fetch_one(&h.sqlite)
        .await?;
        demo_step(7, "edge issues the first automatic dose");

        for _ in 0..300 {
            let delivered = h
                .get_json(&format!("{}/sim/state", h.simulator_url))
                .await?["delivered_today_ml"]
                .as_f64()
                .unwrap_or_default();
            let status: String =
                sqlx::query_scalar("SELECT status FROM commands WHERE command_id=?")
                    .bind(&first_command.0)
                    .fetch_one(&h.sqlite)
                    .await?;
            if delivered >= SCENARIO_DOSE_ML - DOSE_EPSILON_ML && status == "completed" {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        let first_status: String =
            sqlx::query_scalar("SELECT status FROM commands WHERE command_id=?")
                .bind(&first_command.0)
                .fetch_one(&h.sqlite)
                .await?;
        ensure!(first_status == "completed");
        demo_step(8, "simulator applies the first water dose");

        let mut first_wait = false;
        for _ in 0..300 {
            let state: Option<(String, Option<i64>)> = sqlx::query_as(
                "SELECT state,wait_until FROM irrigation_state WHERE plant_id='scenario-plant'",
            )
            .fetch_optional(&h.sqlite)
            .await?;
            if state.is_some_and(|row| row.0 == "wait_for_absorption" && row.1.is_some()) {
                first_wait = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        ensure!(first_wait);
        demo_step(9, "plant enters the absorption wait");

        let mut second_seen = false;
        for _ in 0..500 {
            h.simulator_post("/sim/state", json!({"moisture_vwc":20.0}))
                .await?;
            let rechecks: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM plant_events WHERE plant_id='scenario-plant' AND kind='irrigation_state_changed' AND detail_json LIKE '%recheck%'",
            )
            .fetch_one(&h.sqlite)
            .await?;
            let commands: i64 =
                sqlx::query_scalar("SELECT count(*) FROM commands WHERE plant_id='scenario-plant'")
                    .fetch_one(&h.sqlite)
                    .await?;
            if rechecks > 0 && commands >= 2 {
                second_seen = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        ensure!(
            second_seen,
            "first recheck did not remain dry and issue dose two"
        );
        demo_step(10, "recheck confirms the soil is still dry");
        demo_step(11, "edge issues a second bounded dose");

        for _ in 0..300 {
            let completed: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM commands WHERE plant_id='scenario-plant' AND status='completed'",
            )
            .fetch_one(&h.sqlite)
            .await?;
            if completed >= 2 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        h.simulator_post("/sim/state", json!({"moisture_vwc":40.0}))
            .await?;
        tokio::time::sleep(Duration::from_millis(650)).await;
        demo_step(12, "moisture recovers into the healthy band");

        for _ in 0..500 {
            let state: String = sqlx::query_scalar(
                "SELECT state FROM irrigation_state WHERE plant_id='scenario-plant'",
            )
            .fetch_one(&h.sqlite)
            .await?;
            if state == "normal" {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        let healthy: (String, Option<String>, i64) = sqlx::query_as(
            "SELECT i.state,p.lockout_reason,(SELECT count(*) FROM watering_events WHERE plant_id=p.plant_id) FROM irrigation_state i JOIN plants p ON p.plant_id=i.plant_id WHERE p.plant_id='scenario-plant'",
        )
        .fetch_one(&h.sqlite)
        .await?;
        ensure!(healthy.0 == "normal" && healthy.1.is_none() && healthy.2 == 2);
        demo_step(13, "plant returns healthy after exactly two doses");

        h.stop_service("cloud-api")?;
        demo_step(14, "cloud API is stopped");
        let queued_before: i64 =
            sqlx::query_scalar("SELECT count(*) FROM pending_cloud_events WHERE status='pending'")
                .fetch_one(&h.sqlite)
                .await?;
        let batches_before: i64 =
            sqlx::query_scalar("SELECT count(DISTINCT batch_id) FROM measurements")
                .fetch_one(&h.sqlite)
                .await?;
        for _ in 0..200 {
            let batches: i64 =
                sqlx::query_scalar("SELECT count(DISTINCT batch_id) FROM measurements")
                    .fetch_one(&h.sqlite)
                    .await?;
            if batches > batches_before {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        h.get_json(&format!("{}/health/ready", h.edge_url)).await?;
        demo_step(15, "edge remains ready and continues ingesting locally");
        let queued_after: i64 =
            sqlx::query_scalar("SELECT count(*) FROM pending_cloud_events WHERE status='pending'")
                .fetch_one(&h.sqlite)
                .await?;
        ensure!(queued_after > queued_before);
        demo_step(16, "cloud events queue durably in SQLite");

        h.start_service("cloud-api")?;
        for _ in 0..400 {
            if h.get_json(&format!("{}/health/ready", h.cloud_url))
                .await
                .is_ok()
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        demo_step(17, "cloud API restarts");
        for _ in 0..4000 {
            let pending: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM pending_cloud_events WHERE status='pending'",
            )
            .fetch_one(&h.sqlite)
            .await?;
            if pending == 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        let pending: i64 =
            sqlx::query_scalar("SELECT count(*) FROM pending_cloud_events WHERE status='pending'")
                .fetch_one(&h.sqlite)
                .await?;
        let cloud_rows: i64 = sqlx::query_scalar("SELECT count(*) FROM synced_events")
            .fetch_one(&h.postgres)
            .await?;
        ensure!(pending == 0 && cloud_rows > 0);
        demo_step(18, "queued events synchronise exactly once to PostgreSQL");
        Ok(())
    })
}

async fn provision_offline_policy(h: &Harness) -> Result<u64> {
    let authored = api_ok(
        h,
        Method::PUT,
        "/api/v1/plants/scenario-plant/offline-policy",
        json!({}),
    )
    .await?;
    ensure!(authored["enabled"] == false);
    let enabled = api_ok(
        h,
        Method::POST,
        "/api/v1/plants/scenario-plant/offline-policy/enable",
        json!({}),
    )
    .await?;
    let version = enabled["policy_version"]
        .as_u64()
        .context("enabled offline policy version")?;
    for _ in 0..200 {
        let state = h
            .get_json(&format!("{}/sim/state", h.simulator_url))
            .await?;
        if state["applied_policy_versions"]["scenario-plant"] == version {
            return Ok(version);
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    bail!("device did not activate offline policy version {version}")
}

/// SCEN-034 end to end: the 24-hour cap is **rolling**, and a virtual midnight
/// does not refill it.
///
/// A calendar window is the tempting implementation and the dangerous one: it
/// hands a plant a fresh allowance at midnight, so a run that straddles it can
/// deliver twice the cap. `window_start` is `now - 24h` for exactly this
/// reason, and this is where that shows.
///
/// `recommended` is the mode under test, not `manual`: M6-007's budget query is
/// `mode IN ('automatic','recommended')`, so a manual dose does not spend the
/// budget and would prove nothing about it.
fn rolling_cap_across_midnight<'a>(h: &'a Harness) -> ScenarioFuture<'a> {
    Box::pin(async move {
        setup_plant(h, true).await?;
        h.simulator_post(
            "/sim/state",
            json!({"moisture_vwc":20.0,"tank_percent":100.0,"leak":"clear"}),
        )
        .await?;
        tokio::time::sleep(Duration::from_millis(700)).await;

        // Spend the cap. The profile allows 300 ml a day in 60 ml doses, so the
        // sixth request is the one that must be refused.
        let mut accepted = 0;
        let mut refusal = None;
        for _ in 0..10 {
            let (status, body) = h
                .json(
                    Method::POST,
                    &format!("{}/api/v1/plants/scenario-plant/water", h.edge_url),
                    json!({"ml": SCENARIO_DOSE_ML, "mode": "recommended"}),
                )
                .await?;
            if status == reqwest::StatusCode::CONFLICT {
                refusal = Some(body);
                break;
            }
            ensure!(status == reqwest::StatusCode::ACCEPTED, "{status}: {body}");
            accepted += 1;
            // One command in flight at a time: let it settle before asking again.
            wait_open_commands_drained(h).await?;
        }
        let refusal = refusal.context("the cap never refused a dose")?;
        ensure!(
            refusal["error"]["details"]["reason"] == "daily_limit",
            "expected a daily-limit refusal, got {refusal}"
        );
        ensure!(accepted >= 4, "only {accepted} doses fitted inside the cap");

        // Cross the next virtual midnight. At the overlay's scale a day passes
        // in about half a minute of real time, so this is a wait rather than a
        // simulation — and waiting is the point: the clock has to actually pass
        // the boundary for the boundary to be tested.
        let before = now_ms(h).await?;
        let day_ms = 24 * 60 * 60 * 1000;
        let midnight = before - before.rem_euclid(day_ms) + day_ms;
        let mut crossed = false;
        for _ in 0..4_000 {
            if now_ms(h).await? > midnight {
                crossed = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        ensure!(
            crossed,
            "the edge clock did not reach the next virtual midnight"
        );

        let (status, body) = h
            .json(
                Method::POST,
                &format!("{}/api/v1/plants/scenario-plant/water", h.edge_url),
                json!({"ml": SCENARIO_DOSE_ML, "mode": "recommended"}),
            )
            .await?;
        ensure!(
            status == reqwest::StatusCode::CONFLICT
                && body["error"]["details"]["reason"] == "daily_limit",
            "midnight refilled the daily allowance: {status} {body}"
        );
        Ok(())
    })
}

/// Waits until no command is open, so the next operator request is not refused
/// merely because one is already in flight.
async fn wait_open_commands_drained(h: &Harness) -> Result<()> {
    for _ in 0..800 {
        let open: i64 =
            sqlx::query_scalar("SELECT count(*) FROM commands WHERE settled_at IS NULL")
                .fetch_one(&h.sqlite)
                .await?;
        if open == 0 {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    bail!("a command never settled")
}

/// The batch count, once it has stopped moving.
///
/// Two equal reads a quarter of a second apart. Used where a scenario needs a
/// baseline that is not still absorbing a queue.
async fn wait_batches_settled(h: &Harness) -> Result<i64> {
    let mut previous = -1;
    for _ in 0..200 {
        let count: i64 = sqlx::query_scalar("SELECT count(DISTINCT batch_id) FROM measurements")
            .fetch_one(&h.sqlite)
            .await?;
        if count == previous {
            return Ok(count);
        }
        previous = count;
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    bail!("telemetry ingestion never settled")
}

/// The edge's own clock, as the newest measurement receipt reports it.
async fn now_ms(h: &Harness) -> Result<i64> {
    Ok(
        sqlx::query_scalar::<_, Option<i64>>("SELECT max(received_at) FROM measurements")
            .fetch_one(&h.sqlite)
            .await?
            .unwrap_or_default(),
    )
}

/// Waits for a sample of `kind` received after `after`.
async fn wait_measurement_newer_than(h: &Harness, kind: &str, after: i64) -> Result<bool> {
    for _ in 0..400 {
        let rows: i64 =
            sqlx::query_scalar("SELECT count(*) FROM measurements WHERE kind=? AND received_at>?")
                .bind(kind)
                .bind(after)
                .fetch_one(&h.sqlite)
                .await?;
        if rows > 0 {
            return Ok(true);
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    Ok(false)
}

async fn wait_simulator(
    h: &Harness,
    attempts: usize,
    predicate: impl Fn(&Value) -> bool,
) -> Result<Value> {
    for _ in 0..attempts {
        let state = h
            .get_json(&format!("{}/sim/state", h.simulator_url))
            .await?;
        if predicate(&state) {
            return Ok(state);
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    // The last state, not a bare timeout: PRD 080's failure table asks for the
    // last known state on a timeout, and "did not reach the expected condition"
    // on its own means a trip to the container to ask what it was.
    let last = h
        .get_json(&format!("{}/sim/state", h.simulator_url))
        .await
        .unwrap_or_else(|error| json!({ "unreadable": error.to_string() }));
    bail!("simulator state did not reach the expected condition; last observed: {last}")
}

fn isolation_no_policy<'a>(h: &'a Harness) -> ScenarioFuture<'a> {
    Box::pin(async move {
        let device = setup_plant(h, false).await?;
        h.simulator_post(
            "/sim/state",
            json!({"moisture_vwc":5.0,"tank_percent":100.0,"leak":"clear"}),
        )
        .await?;
        let before = h
            .get_json(&format!("{}/sim/state", h.simulator_url))
            .await?;
        let delivered_before = before["delivered_today_ml"].as_f64().unwrap_or_default();
        fault(h, "disconnect:21600", true).await?;
        let isolated = wait_simulator(h, 400, |s| {
            s["connected"] == false && s["buffered_cycles"].as_u64().unwrap_or_default() > 0
        })
        .await?;
        ensure!(isolated["buffered_events"].as_u64().unwrap_or_default() > 0);
        let during = wait_simulator(h, 1600, |s| s["connected"] == true).await?;
        ensure!(
            (during["delivered_today_ml"].as_f64().unwrap_or_default() - delivered_before).abs()
                < 0.01,
            "an unprovisioned isolated device actuated: {during}"
        );
        for _ in 0..400 {
            let refusal: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM device_events WHERE device_id=? AND kind='offline.refused' AND detail_json LIKE '%no_valid_policy%'",
            )
            .bind(&device)
            .fetch_one(&h.sqlite)
            .await?;
            if refusal > 0 {
                let doses: i64 = sqlx::query_scalar(
                    "SELECT count(*) FROM watering_events WHERE plant_id='scenario-plant'",
                )
                .fetch_one(&h.sqlite)
                .await?;
                ensure!(
                    doses == 0,
                    "unprovisioned plant has {doses} watering events"
                );
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        bail!("replayed history did not expose the no_valid_policy audit refusal")
    })
}

fn isolation_automation<'a>(h: &'a Harness) -> ScenarioFuture<'a> {
    Box::pin(async move {
        setup_plant(h, true).await?;
        h.simulator_post(
            "/sim/state",
            json!({"moisture_vwc":40.0,"tank_percent":100.0,"leak":"clear"}),
        )
        .await?;
        provision_offline_policy(h).await?;
        fault(h, "disconnect:7200", true).await?;
        wait_simulator(h, 200, |s| s["connected"] == false).await?;
        h.simulator_post("/sim/state", json!({"moisture_vwc":5.0}))
            .await?;
        let dosed = wait_simulator(h, 1000, |s| {
            s["delivered_today_ml"].as_f64().unwrap_or_default()
                >= SCENARIO_DOSE_ML - DOSE_EPSILON_ML
        })
        .await?;
        ensure!(
            dosed["delivered_today_ml"].as_f64().unwrap_or_default()
                <= SCENARIO_DOSE_ML + DOSE_EPSILON_ML,
            "the isolated device delivered more than one bounded dose"
        );
        h.simulator_post("/sim/state", json!({"moisture_vwc":46.0}))
            .await?;
        let reconnected = wait_simulator(h, 800, |s| s["connected"] == true).await?;
        ensure!(
            reconnected["delivered_today_ml"]
                .as_f64()
                .unwrap_or_default()
                <= SCENARIO_DOSE_ML + DOSE_EPSILON_ML,
            "more than one autonomous dose was delivered: {reconnected}"
        );
        for _ in 0..400 {
            let rows: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM watering_events WHERE plant_id='scenario-plant' AND origin='offline_autonomous' AND abs(delivered_ml - ?) < ?",
            )
            .bind(SCENARIO_DOSE_ML)
            .bind(DOSE_EPSILON_ML)
            .fetch_one(&h.sqlite)
            .await?;
            if rows == 1 {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        bail!("exactly one autonomous {SCENARIO_DOSE_ML} ml watering event was not reconciled")
    })
}

fn isolation_mid_dose<'a>(h: &'a Harness) -> ScenarioFuture<'a> {
    Box::pin(async move {
        setup_plant(h, true).await?;
        h.simulator_post(
            "/sim/state",
            json!({"moisture_vwc":40.0,"tank_percent":100.0,"leak":"clear"}),
        )
        .await?;
        tokio::time::sleep(Duration::from_millis(500)).await;
        fault(h, "disconnect-mid-dose", true).await?;
        h.clear_mqtt().await;
        let response = api_ok(
            h,
            Method::POST,
            "/api/v1/plants/scenario-plant/water",
            json!({"ml":80.0,"mode":"manual"}),
        )
        .await?;
        let command_id = response["command_id"]
            .as_str()
            .context("manual water command_id")?
            .to_owned();
        wait_simulator(h, 300, |s| s["connected"] == false).await?;
        let completed = wait_simulator(h, 400, |s| {
            s["delivered_today_ml"].as_f64().unwrap_or_default() >= 79.99
        })
        .await?;
        ensure!(completed["pending_results"].as_u64().unwrap_or_default() > 0);
        let command_publications = h
            .mqtt()
            .await
            .iter()
            .filter(|m| m.topic.ends_with("/commands/water"))
            .count();
        ensure!(
            command_publications == 1,
            "edge re-issued the command during isolation"
        );
        wait_simulator(h, 500, |s| s["connected"] == true).await?;
        for _ in 0..400 {
            let settled: Option<(String, f64)> = sqlx::query_as(
                "SELECT status,delivered_ml FROM watering_events WHERE command_id=?",
            )
            .bind(&command_id)
            .fetch_optional(&h.sqlite)
            .await?;
            if settled
                .as_ref()
                .is_some_and(|(status, ml)| status == "completed" && (*ml - 80.0).abs() < 0.01)
            {
                let count: i64 =
                    sqlx::query_scalar("SELECT count(*) FROM watering_events WHERE command_id=?")
                        .bind(&command_id)
                        .fetch_one(&h.sqlite)
                        .await?;
                ensure!(
                    count == 1,
                    "command reconciled into {count} watering events"
                );
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        bail!("buffered result did not settle original command {command_id}")
    })
}

fn reconnect_fresh_sync<'a>(h: &'a Harness) -> ScenarioFuture<'a> {
    Box::pin(async move {
        let device = setup_plant(h, true).await?;
        provision_offline_policy(h).await?;
        h.simulator_post(
            "/sim/state",
            json!({"moisture_vwc":5.0,"tank_percent":100.0,"leak":"clear"}),
        )
        .await?;
        fault(h, "clock-unsync", true).await?;
        fault(h, "disconnect:3600", true).await?;
        let autonomous = wait_simulator(h, 500, |s| {
            s["delivered_today_ml"].as_f64().unwrap_or_default()
                >= SCENARIO_DOSE_ML - DOSE_EPSILON_ML
        })
        .await?;
        ensure!(
            autonomous["clock_synced"] == false,
            "autonomy depended on wall time"
        );
        wait_simulator(h, 500, |s| s["connected"] == true).await?;
        let (first_id, first) = direct_water_payload(h, &device).await?;
        h.clear_mqtt().await;
        h.publish(&format!("rhizo/v1/devices/{device}/commands/water"), first)
            .await?;
        wait_mqtt(
            h,
            |m| {
                m.topic.ends_with("/commands/result")
                    && serde_json::from_slice::<Value>(&m.payload).is_ok_and(|v| {
                        v["data"]["command_id"] == first_id
                            && v["data"]["status"] == "rejected"
                            && v["data"]["reason"] == "clock_unsynced"
                    })
            },
            300,
        )
        .await?;
        fault(h, "clock-unsync", false).await?;
        wait_simulator(h, 400, |s| s["clock_synced"] == true).await?;
        let (second_id, second) = direct_water_payload(h, &device).await?;
        h.clear_mqtt().await;
        h.publish(&format!("rhizo/v1/devices/{device}/commands/water"), second)
            .await?;
        wait_mqtt(
            h,
            |m| {
                m.topic.ends_with("/commands/result")
                    && serde_json::from_slice::<Value>(&m.payload).is_ok_and(|v| {
                        v["data"]["command_id"] == second_id && v["data"]["status"] == "completed"
                    })
            },
            400,
        )
        .await?;
        Ok(())
    })
}

fn isolation_corrupt_policy<'a>(h: &'a Harness) -> ScenarioFuture<'a> {
    Box::pin(async move {
        let device = setup_plant(h, true).await?;
        provision_offline_policy(h).await?;
        h.stop_service("device-simulator")?;
        h.clear_retained(&format!("rhizo/v1/devices/{device}/policy"))
            .await?;
        tokio::time::sleep(Duration::from_millis(100)).await;
        h.corrupt_simulator_policy()?;
        h.start_service("device-simulator")?;
        h.wait_simulator_ready().await?;
        wait_http_simulator(h).await?;
        let state = h
            .get_json(&format!("{}/sim/state", h.simulator_url))
            .await?;
        ensure!(
            state["applied_policy_versions"]
                .as_object()
                .is_some_and(|v| v.is_empty())
        );
        ensure!(state["persistent_state_fault"].is_null());
        ensure!(state["last_policy_rejection"] == "malformed");
        h.simulator_post("/sim/state", json!({"moisture_vwc":5.0}))
            .await?;
        fault(h, "disconnect:3600", true).await?;
        let reconnected = wait_simulator(h, 500, |s| s["connected"] == true).await?;
        ensure!(
            reconnected["delivered_today_ml"]
                .as_f64()
                .unwrap_or_default()
                == 0.0
        );
        ensure!(reconnected["persistent_state_fault"].is_null());
        for _ in 0..400 {
            let invalid: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM device_events WHERE kind='offline.refused' AND detail_json LIKE '%policy_invalid%'",
            )
            .fetch_one(&h.sqlite)
            .await?;
            if invalid > 0 {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        bail!("policy_invalid audit event was not visible after reconnect")
    })
}

async fn wait_http_simulator(h: &Harness) -> Result<()> {
    for _ in 0..200 {
        if h.get_json(&format!("{}/sim/state", h.simulator_url))
            .await
            .is_ok()
        {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    bail!("simulator control API did not return after restart")
}

fn long_isolation<'a>(h: &'a Harness) -> ScenarioFuture<'a> {
    Box::pin(async move {
        let device = setup_plant(h, true).await?;
        provision_offline_policy(h).await?;
        api_ok(
            h,
            Method::POST,
            "/api/v1/plants",
            json!({"plant_id":"unprovisioned-plant","name":"Unprovisioned plant","profile_id":"default","pot_volume_ml":2000.0}),
        )
        .await?;
        api_ok(
            h,
            Method::PUT,
            "/api/v1/plants/unprovisioned-plant/bindings/sensors",
            json!({"device_id":device,"sensor_id":"soil-0","point":"default","kind":"soil_moisture","role":"control"}),
        )
        .await?;
        h.simulator_post(
            "/sim/state",
            json!({"moisture_vwc":5.0,"tank_percent":100.0,"leak":"clear"}),
        )
        .await?;
        let started = h
            .get_json(&format!("{}/sim/state", h.simulator_url))
            .await?["uptime_ms"]
            .as_u64()
            .context("simulator uptime")?;
        h.stop_service("edge-controller")?;
        fault(h, "disconnect:172800", true).await?;
        // Start the edge during the final eight virtual hours so its MQTT
        // subscription is established before the device reconnects and replays.
        wait_simulator(h, 14_000, |s| {
            s["uptime_ms"]
                .as_u64()
                .is_some_and(|now| now.saturating_sub(started) >= 144_000_000)
        })
        .await?;
        let isolated = h
            .get_json(&format!("{}/sim/state", h.simulator_url))
            .await?;
        ensure!(isolated["connected"] == false);
        ensure!(isolated["delivered_today_ml"].as_f64().unwrap_or_default() > 0.0);
        ensure!(isolated["delivered_today_ml"].as_f64().unwrap_or_default() <= 500.0);
        ensure!(isolated["buffered_cycles"].as_u64().unwrap_or_default() > 0);
        ensure!(isolated["buffered_events"].as_u64().unwrap_or_default() > 0);
        h.clear_mqtt().await;
        h.start_service("edge-controller")?;
        wait_edge_ready(h).await?;
        wait_mqtt(h, |m| m.topic == "rhizo/v1/health/broker", 1_200).await?;
        wait_simulator(h, 3_000, |s| s["connected"] == true).await?;
        for _ in 0..800 {
            let provisioned: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM watering_events WHERE plant_id='scenario-plant' AND origin='offline_autonomous'",
            )
            .fetch_one(&h.sqlite)
            .await?;
            let unprovisioned: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM watering_events WHERE plant_id='unprovisioned-plant'",
            )
            .fetch_one(&h.sqlite)
            .await?;
            if provisioned > 0 {
                ensure!(unprovisioned == 0, "unprovisioned plant was watered");
                let duplicates: i64 = sqlx::query_scalar(
                    "SELECT count(*) FROM (SELECT watering_event_id FROM watering_events GROUP BY watering_event_id HAVING count(*) > 1)",
                )
                .fetch_one(&h.sqlite)
                .await?;
                ensure!(
                    duplicates == 0,
                    "isolation replay created duplicate watering rows"
                );
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        bail!("48-hour autonomous history did not reconcile after edge restart")
    })
}

fn policy_activation<'a>(h: &'a Harness) -> ScenarioFuture<'a> {
    Box::pin(async move {
        setup_plant(h, true).await?;
        let mut active = provision_offline_policy(h).await?;
        for step in ["validate", "stage", "verify", "activate", "acknowledge"] {
            let before = h
                .get_json(&format!("{}/sim/state", h.simulator_url))
                .await?;
            let boot = before["boot_count"].as_u64().context("boot_count")?;
            fault(h, &format!("policy-interrupt:{step}"), true).await?;
            let authored = api_ok(
                h,
                Method::PUT,
                "/api/v1/plants/scenario-plant/offline-policy",
                json!({}),
            )
            .await?;
            let offered = authored["policy_version"]
                .as_u64()
                .context("policy version")?;
            let state = wait_simulator(h, 400, |s| {
                s["boot_count"].as_u64().unwrap_or_default() > boot
                    && s["applied_policy_versions"]["scenario-plant"]
                        .as_u64()
                        .is_some_and(|v| v == active || v == offered)
            })
            .await?;
            let held = state["applied_policy_versions"]["scenario-plant"]
                .as_u64()
                .context("held policy version")?;
            ensure!(
                held == active || held == offered,
                "mixed policy state after {step}"
            );
            let enabled = api_ok(
                h,
                Method::POST,
                "/api/v1/plants/scenario-plant/offline-policy/enable",
                json!({}),
            )
            .await?;
            active = enabled["policy_version"]
                .as_u64()
                .context("enabled version")?;
            wait_simulator(h, 400, |s| {
                s["applied_policy_versions"]["scenario-plant"] == active
            })
            .await?;
        }
        Ok(())
    })
}

fn offline_budget<'a>(h: &'a Harness) -> ScenarioFuture<'a> {
    Box::pin(async move {
        setup_plant(h, true).await?;
        provision_offline_policy(h).await?;
        h.simulator_post(
            "/sim/state",
            json!({"moisture_vwc":5.0,"tank_percent":100.0,"leak":"clear"}),
        )
        .await?;
        let started = h
            .get_json(&format!("{}/sim/state", h.simulator_url))
            .await?["uptime_ms"]
            .as_u64()
            .context("uptime")?;
        fault(h, "disconnect:259200", true).await?;
        let mut first_used = 0.0_f64;
        let mut first_window_complete = false;
        for _ in 0..5_000 {
            let state = h
                .get_json(&format!("{}/sim/state", h.simulator_url))
                .await?;
            first_used =
                first_used.max(state["offline_budget_used_ml"].as_f64().unwrap_or_default());
            if state["uptime_ms"]
                .as_u64()
                .is_some_and(|v| v.saturating_sub(started) >= 86_400_000)
            {
                first_window_complete = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        ensure!(
            first_window_complete,
            "first rolling budget window did not complete"
        );
        ensure!(
            first_used > 0.0 && first_used <= 300.01,
            "first rolling budget maximum was {first_used}"
        );
        let final_state = wait_simulator(h, 8_000, |s| s["connected"] == true).await?;
        ensure!(
            final_state["offline_budget_used_ml"]
                .as_f64()
                .unwrap_or_default()
                <= 300.01
        );
        for _ in 0..800 {
            let edge_sum: f64 = sqlx::query_scalar(
                "SELECT CAST(coalesce(sum(delivered_ml),0) AS REAL) FROM watering_events WHERE origin='offline_autonomous'",
            ).fetch_one(&h.sqlite).await?;
            if edge_sum > 0.0 {
                let refusals: i64 = sqlx::query_scalar(
                    "SELECT count(*) FROM device_events WHERE kind='offline.refused' AND detail_json LIKE '%budget_exhausted%'",
                )
                .fetch_one(&h.sqlite)
                .await?;
                ensure!(
                    edge_sum <= 900.01,
                    "72-hour replay exceeded three rolling budgets: {edge_sum}"
                );
                ensure!(
                    refusals > 0,
                    "budget exhaustion produced no durable audit event"
                );
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        bail!("72-hour offline budget history did not reconcile")
    })
}

fn isolated_restart<'a>(h: &'a Harness) -> ScenarioFuture<'a> {
    Box::pin(async move {
        setup_plant(h, true).await?;
        api_ok(
            h,
            Method::PUT,
            "/api/v1/profiles/scenario-profile",
            json!({
                "name":"Scenario profile", "target_min_vwc":28.0, "target_max_vwc":45.0,
                "dose_ml":SCENARIO_DOSE_ML, "max_doses_per_cycle":1, "max_daily_ml":300.0,
                "dry_confirm_minutes":30, "cooldown_hours":6.0,
                "absorption_minutes":15, "recovery_delta_vwc":3.0,
                "tank_min_percent":15.0, "command_ttl_seconds":600
            }),
        )
        .await?;
        provision_offline_policy(h).await?;
        h.simulator_post(
            "/sim/state",
            json!({"moisture_vwc":5.0,"tank_percent":100.0,"leak":"clear"}),
        )
        .await?;
        fault(h, "disconnect:43200", true).await?;
        let dosed = wait_simulator(h, 1_000, |s| {
            s["delivered_today_ml"].as_f64().unwrap_or_default()
                >= SCENARIO_DOSE_ML - DOSE_EPSILON_ML
        })
        .await?;
        let delivered = dosed["delivered_today_ml"].as_f64().unwrap_or_default();
        let cooling = wait_simulator(h, 1_000, |s| {
            s["offline_cooldown_remaining_ms"]
                .as_u64()
                .unwrap_or_default()
                > 0
        })
        .await?;
        let mut remaining = cooling["offline_cooldown_remaining_ms"]
            .as_u64()
            .context("cooldown")?;
        ensure!(remaining > 0 && cooling["buffered_events"].as_u64().unwrap_or_default() > 0);
        for _ in 0..3 {
            h.simulator_post("/sim/restart", json!({})).await?;
            let restarted = h
                .get_json(&format!("{}/sim/state", h.simulator_url))
                .await?;
            let now = restarted["offline_cooldown_remaining_ms"]
                .as_u64()
                .context("cooldown")?;
            ensure!(
                now <= remaining && now > 0,
                "restart reset or erased cooldown"
            );
            ensure!(
                (restarted["delivered_today_ml"].as_f64().unwrap_or_default() - delivered).abs()
                    < 0.01
            );
            ensure!(restarted["buffered_events"].as_u64().unwrap_or_default() > 0);
            remaining = now;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
        let before_cooldown = h
            .get_json(&format!("{}/sim/state", h.simulator_url))
            .await?;
        ensure!(
            (before_cooldown["delivered_today_ml"]
                .as_f64()
                .unwrap_or_default()
                - delivered)
                .abs()
                < 0.01
        );
        Ok(())
    })
}

fn isolated_no_wall_clock<'a>(h: &'a Harness) -> ScenarioFuture<'a> {
    Box::pin(async move {
        let device = setup_plant(h, true).await?;
        provision_offline_policy(h).await?;
        fault(h, "clock-unsync", true).await?;
        h.simulator_post(
            "/sim/state",
            json!({"moisture_vwc":5.0,"tank_percent":100.0,"leak":"clear"}),
        )
        .await?;
        h.clear_mqtt().await;
        fault(h, "disconnect:7200", true).await?;
        let dosed = wait_simulator(h, 1_200, |s| {
            s["delivered_today_ml"].as_f64().unwrap_or_default()
                >= SCENARIO_DOSE_ML - DOSE_EPSILON_ML
        })
        .await?;
        ensure!(dosed["clock_synced"] == false);
        fault(h, "disconnect:1", false).await?;
        wait_simulator(h, 500, |s| s["connected"] == true).await?;
        let replay_has_monotonic_without_wall = h
            .mqtt()
            .await
            .iter()
            .filter(|m| m.topic.ends_with("/events"))
            .any(|m| {
                serde_json::from_slice::<Value>(&m.payload).is_ok_and(|v| {
                    v["data"]["events"].as_array().is_some_and(|events| {
                        events.iter().any(|e| {
                            e["kind"] == "watering.offline_autonomous"
                                && e["device_time_ms"].is_null()
                                && e["monotonic_ms"].as_u64().is_some()
                        })
                    })
                })
            });
        ensure!(
            replay_has_monotonic_without_wall,
            "offline event did not preserve monotonic-only time"
        );
        let (id, payload) = direct_water_payload(h, &device).await?;
        h.publish(
            &format!("rhizo/v1/devices/{device}/commands/water"),
            payload,
        )
        .await?;
        wait_mqtt(
            h,
            |m| {
                m.topic.ends_with("/commands/result")
                    && serde_json::from_slice::<Value>(&m.payload).is_ok_and(|v| {
                        v["data"]["command_id"] == id && v["data"]["reason"] == "clock_unsynced"
                    })
            },
            400,
        )
        .await?;
        Ok(())
    })
}

fn required_measurement<'a>(h: &'a Harness) -> ScenarioFuture<'a> {
    Box::pin(async move {
        setup_plant(h, true).await?;
        provision_offline_policy(h).await?;
        fault(h, "stale-tank", true).await?;
        fault(h, "stale-leak", true).await?;
        h.simulator_post(
            "/sim/state",
            json!({"moisture_vwc":5.0,"tank_percent":100.0,"leak":"clear"}),
        )
        .await?;
        fault(h, "disconnect:10800", true).await?;
        let refused = wait_simulator(h, 1_500, |s| {
            s["buffered_events"].as_u64().unwrap_or_default() >= 2
        })
        .await?;
        ensure!(
            refused["delivered_today_ml"].as_f64().unwrap_or_default() < 0.01,
            "missing required measurement actuated"
        );
        fault(h, "stale-tank", false).await?;
        fault(h, "stale-leak", false).await?;
        let allowed = wait_simulator(h, 1_500, |s| {
            s["delivered_today_ml"].as_f64().unwrap_or_default()
                >= SCENARIO_DOSE_ML - DOSE_EPSILON_ML
        })
        .await?;
        ensure!(
            allowed["delivered_today_ml"].as_f64().unwrap_or_default()
                <= SCENARIO_DOSE_ML + DOSE_EPSILON_ML,
            "the restored measurement allowed more than one bounded dose"
        );
        Ok(())
    })
}

fn reconciliation<'a>(h: &'a Harness) -> ScenarioFuture<'a> {
    Box::pin(async move {
        setup_plant(h, true).await?;
        api_ok(
            h,
            Method::POST,
            "/api/v1/plants/scenario-plant/auto-watering/enable",
            json!({}),
        )
        .await?;
        provision_offline_policy(h).await?;
        h.simulator_post(
            "/sim/state",
            json!({"moisture_vwc":5.0,"tank_percent":100.0,"leak":"clear"}),
        )
        .await?;
        fault(h, "disconnect:50400", true).await?;
        wait_simulator(h, 3_000, |s| {
            s["delivered_today_ml"].as_f64().unwrap_or_default() >= 79.99
        })
        .await?;
        h.clear_mqtt().await;
        fault(h, "disconnect:1", false).await?;
        wait_simulator(h, 600, |s| s["connected"] == true).await?;
        for _ in 0..800 {
            let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM watering_events WHERE plant_id='scenario-plant' AND origin='offline_autonomous'").fetch_one(&h.sqlite).await?;
            if rows >= 2 {
                let sum: f64 = sqlx::query_scalar("SELECT coalesce(sum(delivered_ml),0) FROM watering_events WHERE plant_id='scenario-plant' AND origin='offline_autonomous'").fetch_one(&h.sqlite).await?;
                ensure!(sum >= 79.99, "replayed budget lost a dose: {sum}");
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        let mqtt = h.mqtt().await;
        let commands = mqtt
            .iter()
            .filter(|m| m.topic.ends_with("/commands/water"))
            .count();
        ensure!(
            commands == 0,
            "edge published {commands} commands during reconciliation"
        );
        let mut saw_complete = false;
        for message in mqtt.iter().filter(|m| m.topic.ends_with("/events")) {
            let value: Value = serde_json::from_slice(&message.payload)?;
            let seqs = value["data"]["events"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|e| e["device_seq"].as_u64())
                .collect::<Vec<_>>();
            ensure!(
                seqs.windows(2).all(|w| w[0] < w[1]),
                "replay sequence was not ordered"
            );
            saw_complete |= value["data"]["complete"] == true;
        }
        ensure!(saw_complete, "replay never published complete=true");
        Ok(())
    })
}

async fn captured_replay_batches(h: &Harness) -> Result<Vec<Vec<u8>>> {
    let batches = h
        .mqtt()
        .await
        .into_iter()
        .filter(|m| m.topic.ends_with("/events"))
        .filter_map(|m| {
            serde_json::from_slice::<Value>(&m.payload)
                .ok()
                .filter(|v| v["data"]["replay"] == true)
                .map(|_| m.payload)
        })
        .collect::<Vec<_>>();
    ensure!(!batches.is_empty(), "no replay batches were captured");
    Ok(batches)
}

fn duplicate_replay<'a>(h: &'a Harness) -> ScenarioFuture<'a> {
    Box::pin(async move {
        let device = setup_plant(h, true).await?;
        // One autonomous dose is sufficient to prove replay idempotency and
        // avoids manufacturing an unnecessary reconnect burst while the Edge
        // is deliberately stopped.
        api_ok(
            h,
            Method::PUT,
            "/api/v1/profiles/scenario-profile",
            json!({
                "name":"Scenario profile", "target_min_vwc":28.0, "target_max_vwc":45.0,
                "dose_ml":SCENARIO_DOSE_ML, "max_doses_per_cycle":1, "max_daily_ml":300.0,
                "dry_confirm_minutes":30, "cooldown_hours":6.0,
                "absorption_minutes":15, "recovery_delta_vwc":3.0,
                "tank_min_percent":15.0, "command_ttl_seconds":600
            }),
        )
        .await?;
        provision_offline_policy(h).await?;
        h.stop_service("edge-controller")?;
        h.simulator_post(
            "/sim/state",
            json!({"moisture_vwc":5.0,"tank_percent":100.0,"leak":"clear"}),
        )
        .await?;
        fault(h, "disconnect:7200", true).await?;
        wait_simulator(h, 1_200, |s| {
            s["delivered_today_ml"].as_f64().unwrap_or_default()
                >= SCENARIO_DOSE_ML - DOSE_EPSILON_ML
        })
        .await?;
        h.clear_mqtt().await;
        fault(h, "disconnect:1", false).await?;
        wait_simulator(h, 100, |s| s["isolation_remaining_ms"] == 0).await?;
        wait_simulator(h, 2_000, |s| s["connected"] == true).await?;
        let batches = captured_replay_batches(h).await?;
        h.start_service("edge-controller")?;
        wait_edge_ready(h).await?;
        h.clear_mqtt().await;
        for _ in 0..3 {
            for payload in batches.iter().rev() {
                h.publish(
                    &format!("rhizo/v1/devices/{device}/events"),
                    payload.clone(),
                )
                .await?;
            }
        }
        for _ in 0..600 {
            let distinct: i64 = sqlx::query_scalar("SELECT count(DISTINCT watering_event_id) FROM watering_events WHERE origin='offline_autonomous'").fetch_one(&h.sqlite).await?;
            let total: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM watering_events WHERE origin='offline_autonomous'",
            )
            .fetch_one(&h.sqlite)
            .await?;
            if distinct > 0 {
                ensure!(total == distinct, "triple replay duplicated watering rows");
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        bail!("triple replay did not persist")
    })
}

fn restart_mid_replay<'a>(h: &'a Harness) -> ScenarioFuture<'a> {
    Box::pin(async move {
        let device = setup_plant(h, true).await?;
        provision_offline_policy(h).await?;
        h.stop_service("edge-controller")?;
        h.simulator_post("/sim/state", json!({"moisture_vwc":35.0}))
            .await?;
        fault(h, "disconnect:86400", true).await?;
        let mut expected = 0;
        for _ in 0..17 {
            fault(h, "leak", true).await?;
            expected += 1;
            wait_simulator(h, 100, |s| {
                s["buffered_events"].as_u64().unwrap_or_default() >= expected
            })
            .await?;
            fault(h, "leak", false).await?;
            fault(h, "tank-empty", true).await?;
            expected += 1;
            wait_simulator(h, 100, |s| {
                s["buffered_events"].as_u64().unwrap_or_default() >= expected
            })
            .await?;
            fault(h, "tank-empty", false).await?;
            h.simulator_post("/sim/state", json!({"tank_percent":100.0}))
                .await?;
        }
        wait_simulator(h, 2_000, |s| {
            s["buffered_events"].as_u64().unwrap_or_default() > 32
        })
        .await?;
        h.clear_mqtt().await;
        fault(h, "disconnect:1", false).await?;
        wait_simulator(h, 500, |s| s["connected"] == true).await?;
        let batches = captured_replay_batches(h).await?;
        ensure!(
            batches.len() > 1,
            "mid-replay scenario needs multiple batches"
        );
        h.start_service("edge-controller")?;
        wait_edge_ready(h).await?;
        h.arm_service_fault("edge-controller", "/var/lib/rhizo/fault-exit-mid-replay")?;
        h.clear_mqtt().await;
        h.publish(
            &format!("rhizo/v1/devices/{device}/events"),
            batches[0].clone(),
        )
        .await?;
        for _ in 0..200 {
            if !h.service_running("edge-controller")? {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        ensure!(
            !h.service_running("edge-controller")?,
            "edge did not exit at the mid-replay hook"
        );
        let commands_before = h
            .mqtt()
            .await
            .iter()
            .filter(|m| m.topic.ends_with("/commands/water"))
            .count();
        ensure!(commands_before == 0);
        h.start_service("edge-controller")?;
        wait_edge_ready(h).await?;
        for payload in &batches {
            h.publish(
                &format!("rhizo/v1/devices/{device}/events"),
                payload.clone(),
            )
            .await?;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
        let duplicates: i64 = sqlx::query_scalar("SELECT count(*) FROM (SELECT event_id FROM device_events GROUP BY event_id HAVING count(*)>1)").fetch_one(&h.sqlite).await?;
        ensure!(duplicates == 0, "restart mid-replay duplicated events");
        let complete: i64 =
            sqlx::query_scalar("SELECT count(*) FROM replay_progress WHERE complete=1")
                .fetch_one(&h.sqlite)
                .await?;
        ensure!(complete > 0, "replay did not complete after edge restart");
        let commands = h
            .mqtt()
            .await
            .iter()
            .filter(|m| m.topic.ends_with("/commands/water"))
            .count();
        ensure!(commands == 0, "command published across mid-replay restart");
        Ok(())
    })
}

fn stale_policy<'a>(h: &'a Harness) -> ScenarioFuture<'a> {
    Box::pin(async move {
        let device = setup_plant(h, true).await?;
        let old_version = provision_offline_policy(h).await?;
        let old_payload = h
            .mqtt()
            .await
            .into_iter()
            .rev()
            .find(|m| {
                m.topic.ends_with("/policy")
                    && serde_json::from_slice::<Value>(&m.payload)
                        .is_ok_and(|v| v["data"]["policies"][0]["policy_version"] == old_version)
            })
            .context("old retained policy payload")?
            .payload;
        fault(h, "disconnect:7200", true).await?;
        let mut latest = old_version;
        for _ in 0..2 {
            api_ok(
                h,
                Method::PUT,
                "/api/v1/plants/scenario-plant/offline-policy",
                json!({}),
            )
            .await?;
            let enabled = api_ok(
                h,
                Method::POST,
                "/api/v1/plants/scenario-plant/offline-policy/enable",
                json!({}),
            )
            .await?;
            latest = enabled["policy_version"]
                .as_u64()
                .context("latest policy version")?;
        }
        fault(h, "disconnect:1", false).await?;
        wait_simulator(h, 600, |s| {
            s["applied_policy_versions"]["scenario-plant"] == latest
        })
        .await?;
        h.publish(&format!("rhizo/v1/devices/{device}/policy"), old_payload)
            .await?;
        tokio::time::sleep(Duration::from_millis(300)).await;
        let state = h
            .get_json(&format!("{}/sim/state", h.simulator_url))
            .await?;
        ensure!(
            state["applied_policy_versions"]["scenario-plant"] == latest,
            "stale policy replaced v{latest}"
        );
        let db_version: i64 = sqlx::query_scalar(
            "SELECT policy_version FROM offline_policies WHERE plant_id='scenario-plant'",
        )
        .fetch_one(&h.sqlite)
        .await?;
        ensure!(
            db_version == latest as i64,
            "edge policy drifted from device"
        );
        Ok(())
    })
}

fn history_gap<'a>(h: &'a Harness) -> ScenarioFuture<'a> {
    Box::pin(async move {
        setup_plant(h, false).await?;
        h.simulator_post("/sim/state", json!({"moisture_vwc":5.0}))
            .await?;
        fault(h, "disconnect:259200", true).await?;
        wait_simulator(h, 8_000, |s| {
            s["buffered_cycles"].as_u64().unwrap_or_default() >= 16
                && s["buffered_events"].as_u64().unwrap_or_default() >= 257
        })
        .await?;
        fault(h, "disconnect:1", false).await?;
        wait_simulator(h, 600, |s| s["connected"] == true).await?;
        for _ in 0..1_000 {
            let gaps: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM history_gaps WHERE lost_count>0 AND from_seq<=to_seq",
            )
            .fetch_one(&h.sqlite)
            .await?;
            let audits: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM device_events WHERE kind='offline.refused'",
            )
            .fetch_one(&h.sqlite)
            .await?;
            if gaps > 0 {
                ensure!(
                    audits > 0,
                    "audit events did not survive telemetry eviction"
                );
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        bail!("overflow replay produced no explicit history gap")
    })
}

fn advisory_measurement<'a>(h: &'a Harness) -> ScenarioFuture<'a> {
    Box::pin(async move {
        setup_plant(h, true).await?;
        provision_offline_policy(h).await?;
        fault(h, "stale-weight", true).await?;
        h.simulator_post(
            "/sim/state",
            json!({"moisture_vwc":5.0,"tank_percent":100.0,"leak":"clear"}),
        )
        .await?;
        fault(h, "disconnect:7200", true).await?;
        let state = wait_simulator(h, 1_500, |s| {
            s["delivered_today_ml"].as_f64().unwrap_or_default()
                >= SCENARIO_DOSE_ML - DOSE_EPSILON_ML
        })
        .await?;
        ensure!(
            state["delivered_today_ml"].as_f64().unwrap_or_default()
                <= SCENARIO_DOSE_ML + DOSE_EPSILON_ML,
            "missing advisory measurement blocked or over-dosed"
        );
        Ok(())
    })
}

fn sleeping_manual_water<'a>(h: &'a Harness) -> ScenarioFuture<'a> {
    Box::pin(async move {
        setup_battery_plant(h).await?;
        wait_battery_sleep(h).await?;
        h.clear_mqtt().await;
        let held = request_battery_water(h, 30.0).await?;
        ensure!(held["expected_delivery_after"].as_str().is_some());
        ensure!(held["intent_expires_at"].as_str().is_some());
        tokio::time::sleep(Duration::from_millis(50)).await;
        ensure!(
            h.mqtt().await.iter().all(|m| !is_edge_command(&m.topic)),
            "a command was published while the battery device slept"
        );
        // Stop the accelerated Edge clock immediately: waiting through a
        // graceful container timeout would itself consume hours of virtual
        // intent lifetime and turn this crash-recovery case into an expiry
        // case. The sleeping simulator remains alive and wakes normally.
        h.kill_service("edge-controller")?;
        h.start_service("edge-controller")?;
        wait_edge_ready(h).await?;
        h.clear_mqtt().await;
        wait_mqtt(h, |m| m.topic.ends_with("/commands/water"), 800).await?;
        for _ in 0..800 {
            let rows: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM watering_events WHERE plant_id='battery-plant'",
            )
            .fetch_one(&h.sqlite)
            .await?;
            if rows == 1 {
                let published = h
                    .mqtt()
                    .await
                    .iter()
                    .filter(|m| m.topic.ends_with("/commands/water"))
                    .count();
                ensure!(published == 1, "intent delivered {published} times");
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        bail!("held battery intent produced no watering event")
    })
}

fn sleeping_safety_refusal<'a>(h: &'a Harness) -> ScenarioFuture<'a> {
    Box::pin(async move {
        for variant in ["leak", "tank", "budget"] {
            if variant != "leak" {
                h.reset_scenario().await?;
            }
            setup_battery_plant(h).await?;
            wait_battery_sleep(h).await?;
            match variant {
                "leak" => {
                    h.battery_post("/sim/fault", json!({"fault":"leak","enabled":true}))
                        .await?;
                }
                "tank" => {
                    h.battery_post("/sim/fault", json!({"fault":"tank-empty","enabled":true}))
                        .await?;
                }
                _ => {}
            }
            h.pause_service("battery-simulator")?;
            let held = if variant == "budget" {
                request_battery_water_mode(h, 30.0, "recommended").await?
            } else {
                request_battery_water(h, 30.0).await?
            };
            let intent = held["intent_id"].as_str().context("intent_id")?;
            if variant == "budget" {
                let now: i64 = sqlx::query_scalar("SELECT max(received_at) FROM measurements")
                    .fetch_one(&h.sqlite)
                    .await?;
                sqlx::query("INSERT INTO watering_events(watering_event_id,plant_id,device_id,mode,origin,started_at,completed_at,requested_ml,delivered_ml,status) VALUES('battery-cap','battery-plant','battery-node-01','recommended','edge_command',?,?,300,300,'completed')")
                    .bind(now).bind(now).execute(&h.sqlite).await?;
            }
            h.clear_mqtt().await;
            h.unpause_service("battery-simulator")?;
            let mut refusal = None;
            for _ in 0..800 {
                refusal = sqlx::query_as::<_, (String, Option<String>)>(
                    "SELECT state,refusal_reason FROM command_intents WHERE intent_id=?",
                )
                .bind(intent)
                .fetch_optional(&h.sqlite)
                .await?;
                if refusal.as_ref().is_some_and(|row| row.0 == "refused") {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            let refusal = refusal.context("intent row")?;
            ensure!(refusal.0 == "refused", "{variant} intent was not refused");
            let expected = match variant {
                "leak" => "leak",
                "tank" => "tank_low",
                _ => "daily_limit",
            };
            ensure!(
                refusal.1.as_deref() == Some(expected),
                "{variant} refusal was {:?}, expected {expected}",
                refusal.1
            );
            ensure!(
                h.mqtt()
                    .await
                    .iter()
                    .all(|m| !m.topic.ends_with("/commands/water")),
                "{variant} refusal still published a water command"
            );
        }
        Ok(())
    })
}

fn sleeping_intent_expiry<'a>(h: &'a Harness) -> ScenarioFuture<'a> {
    Box::pin(async move {
        setup_battery_plant(h).await?;
        wait_battery_sleep(h).await?;
        let held = request_battery_water(h, 30.0).await?;
        let intent = held["intent_id"].as_str().context("intent_id")?;
        h.stop_service("battery-simulator")?;
        h.clear_mqtt().await;
        for _ in 0..1_000 {
            let state: Option<String> =
                sqlx::query_scalar("SELECT state FROM command_intents WHERE intent_id=?")
                    .bind(intent)
                    .fetch_optional(&h.sqlite)
                    .await?;
            if state.as_deref() == Some("expired_before_wake") {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        let expired: String =
            sqlx::query_scalar("SELECT state FROM command_intents WHERE intent_id=?")
                .bind(intent)
                .fetch_one(&h.sqlite)
                .await?;
        ensure!(expired == "expired_before_wake");
        ensure!(
            h.mqtt().await.iter().all(|m| !is_edge_command(&m.topic)),
            "a command was published for an intent that expired before the wake"
        );

        h.reset_scenario().await?;
        setup_battery_plant(h).await?;
        wait_battery_sleep(h).await?;
        request_battery_water(h, 30.0).await?;
        h.clear_mqtt().await;
        let command = wait_mqtt(h, |m| m.topic.ends_with("/commands/water"), 800).await?;
        let value: Value = serde_json::from_slice(&command.payload)?;
        let issued = value["data"]["issued_at_ms"]
            .as_i64()
            .context("issued_at")?;
        let expires = value["data"]["expires_at_ms"]
            .as_i64()
            .context("expires_at")?;
        ensure!(
            expires - issued == 600_000,
            "wire TTL was not minted at wake"
        );
        let time = h
            .mqtt()
            .await
            .iter()
            .position(|m| m.topic.ends_with("/time"))
            .context("fresh edge.time")?;
        let water = h
            .mqtt()
            .await
            .iter()
            .position(|m| m.topic.ends_with("/commands/water"))
            .context("wake command")?;
        ensure!(time < water, "command preceded fresh edge.time");
        Ok(())
    })
}

fn battery_awake_cycle<'a>(h: &'a Harness) -> ScenarioFuture<'a> {
    Box::pin(async move {
        setup_battery_plant(h).await?;
        wait_battery_sleep(h).await?;
        // Cleared *before* the request, not after. The battery node wakes every
        // 900 logical seconds, which at 3600x is every quarter of a real
        // second — so the intent can be minted, delivered, and answered inside
        // the gap between the request returning and a clear that follows it,
        // and the result the scenario is about would be thrown away. This is
        // the flake that failed one run in two.
        h.clear_mqtt().await;
        request_battery_water(h, 40.0).await?;
        wait_mqtt(h, |m| m.topic.ends_with("/commands/result"), 800).await?;
        wait_mqtt(
            h,
            |m| {
                m.topic.ends_with("/status")
                    && serde_json::from_slice::<Value>(&m.payload)
                        .is_ok_and(|v| v["data"]["reason"] == "sleeping")
            },
            800,
        )
        .await?;
        let mqtt = h.mqtt().await;
        let result = mqtt
            .iter()
            .position(|m| m.topic.ends_with("/commands/result"))
            .context("command result")?;
        let sleep = mqtt
            .iter()
            .position(|m| {
                m.topic.ends_with("/status")
                    && serde_json::from_slice::<Value>(&m.payload)
                        .is_ok_and(|v| v["data"]["reason"] == "sleeping")
            })
            .context("sleep announcement")?;
        ensure!(result < sleep, "device slept before publishing its result");

        // Repeat the cycle with a power cut after the in-flight marker is
        // durable. The simulator's one-shot restart fault models loss of power,
        // and the next boot must converge pump-off and report uncertainty.
        h.reset_scenario().await?;
        setup_battery_plant(h).await?;
        wait_battery_sleep(h).await?;
        h.battery_post(
            "/sim/fault",
            json!({"fault":"restart-mid-dose","enabled":true}),
        )
        .await?;
        h.clear_mqtt().await;
        request_battery_water(h, 40.0).await?;
        let interrupted = wait_mqtt(
            h,
            |m| {
                m.topic.ends_with("/commands/result")
                    && serde_json::from_slice::<Value>(&m.payload).is_ok_and(|v| {
                        v["data"]["status"] == "interrupted" && v["data"]["delivered_ml"].is_null()
                    })
            },
            800,
        )
        .await?;
        let body: Value = serde_json::from_slice(&interrupted.payload)?;
        ensure!(body["data"]["status"] == "interrupted");
        let state = h.get_json(&format!("{}/sim/state", h.battery_url)).await?;
        ensure!(
            !state["pump_running"].as_bool().unwrap_or(false),
            "battery rebooted with the pump running"
        );
        Ok(())
    })
}

fn sleep_budget_cooldown<'a>(h: &'a Harness) -> ScenarioFuture<'a> {
    Box::pin(async move {
        setup_battery_plant(h).await?;
        let authored = api_ok(
            h,
            Method::PUT,
            "/api/v1/plants/battery-plant/offline-policy",
            json!({}),
        )
        .await?;
        ensure!(authored["enabled"] == false);
        let enabled = api_ok(
            h,
            Method::POST,
            "/api/v1/plants/battery-plant/offline-policy/enable",
            json!({}),
        )
        .await?;
        let version = enabled["policy_version"]
            .as_u64()
            .context("policy version")?;
        for _ in 0..400 {
            let state = h.get_json(&format!("{}/sim/state", h.battery_url)).await?;
            if state["applied_policy_versions"]["battery-plant"] == version {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        h.battery_post(
            "/sim/state",
            json!({"moisture_vwc":5.0,"tank_percent":100.0,"leak":"clear"}),
        )
        .await?;
        h.battery_post(
            "/sim/fault",
            json!({"fault":"disconnect:172800","enabled":true}),
        )
        .await?;
        let cooling = loop {
            let state = h.get_json(&format!("{}/sim/state", h.battery_url)).await?;
            if state["offline_cooldown_remaining_ms"]
                .as_u64()
                .unwrap_or_default()
                > 0
            {
                break state;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        };
        let before = cooling["offline_cooldown_remaining_ms"]
            .as_u64()
            .context("cooldown")?;
        let cold = h.battery_post("/sim/restart", json!({})).await?;
        ensure!(cold["zero_credit_resets"].as_u64().unwrap_or_default() > 0);
        ensure!(
            cold["offline_cooldown_remaining_ms"]
                .as_u64()
                .unwrap_or_default()
                >= before.saturating_sub(60_000),
            "cold reset shortened cooldown"
        );
        let corrupt = h.battery_post("/sim/rtc-corrupt", json!({})).await?;
        ensure!(
            corrupt["zero_credit_checksum_failures"]
                .as_u64()
                .unwrap_or_default()
                > 0
        );
        let final_state = wait_simulator_url(h, &h.battery_url, 8_000, |s| {
            s["credited_timer_wakes"].as_u64().unwrap_or_default() >= 190
        })
        .await?;
        ensure!(
            final_state["offline_budget_used_ml"]
                .as_f64()
                .unwrap_or_default()
                <= 300.01
        );
        ensure!(
            final_state["delivered_today_ml"]
                .as_f64()
                .unwrap_or_default()
                <= 600.01
        );
        Ok(())
    })
}

async fn wait_simulator_url(
    h: &Harness,
    url: &str,
    attempts: usize,
    predicate: impl Fn(&Value) -> bool,
) -> Result<Value> {
    for _ in 0..attempts {
        let state = h.get_json(&format!("{url}/sim/state")).await?;
        if predicate(&state) {
            return Ok(state);
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    bail!("simulator state did not reach the expected condition")
}

macro_rules! scenarios {
    ($(($name:literal, [$($scen:literal),*], [$($safety:literal),*], $run:expr)),+ $(,)?) => {
        /// Complete M8 catalogue in deterministic execution order.
        pub fn catalogue() -> &'static [Scenario] {
            static SCENARIOS: &[Scenario] = &[$(Scenario {
                name: $name,
                covers: &[$($scen),*],
                proves: &[$($safety),*],
                run: $run,
            }),+];
            SCENARIOS
        }
    };
}

scenarios!(
    (
        "scenario_normal_telemetry",
        ["SCEN-001"],
        [],
        normal_telemetry
    ),
    (
        "scenario_full_watering_cycle",
        ["SCEN-002"],
        ["SAFETY-006", "SAFETY-012"],
        full_watering_cycle
    ),
    (
        "scenario_rolling_cap_across_midnight",
        ["SCEN-034"],
        ["SAFETY-006"],
        rolling_cap_across_midnight
    ),
    (
        "scenario_recommendation_without_automation",
        ["SCEN-003"],
        [],
        recommendation_without_automation
    ),
    (
        "scenario_duplicate_command",
        ["SCEN-011"],
        ["SAFETY-001"],
        duplicate_command
    ),
    (
        "scenario_broker_restart",
        ["SCEN-012"],
        ["SAFETY-008"],
        broker_restart
    ),
    (
        "scenario_stale_sensor",
        ["SCEN-022"],
        ["SAFETY-005"],
        stale_sensor
    ),
    (
        "scenario_invalid_sensor",
        ["SCEN-023"],
        ["SAFETY-005", "SAFETY-012"],
        invalid_sensor
    ),
    (
        "scenario_clock_unsynced",
        ["SCEN-025"],
        ["SAFETY-002", "SAFETY-012"],
        clock_unsynced
    ),
    (
        "scenario_queued_command_expiry",
        ["SCEN-031"],
        ["SAFETY-002"],
        queued_command_expiry
    ),
    ("scenario_leak", ["SCEN-040"], ["SAFETY-003"], leak),
    (
        "scenario_tank_empty",
        ["SCEN-042"],
        ["SAFETY-004"],
        tank_empty
    ),
    ("scenario_no_delivery", ["SCEN-044"], [], no_delivery),
    (
        "scenario_restart_mid_command",
        ["SCEN-051"],
        ["SAFETY-010"],
        restart_mid_command
    ),
    (
        "scenario_restart_mid_absorption",
        ["SCEN-052"],
        ["SAFETY-010"],
        restart_mid_absorption
    ),
    (
        "scenario_cloud_unavailable",
        ["SCEN-060"],
        ["SAFETY-008"],
        cloud_unavailable
    ),
    (
        "scenario_cloud_independence",
        ["SCEN-061"],
        ["SAFETY-009"],
        cloud_independence
    ),
    (
        "scenario_cloud_outage_recovery",
        ["SCEN-062"],
        ["SAFETY-008"],
        cloud_outage_recovery
    ),
    (
        "scenario_reconnect_fresh_sync",
        ["SCEN-077"],
        ["SAFETY-002", "SAFETY-015"],
        reconnect_fresh_sync
    ),
    (
        "scenario_isolation_no_policy",
        ["SCEN-090", "SCEN-093"],
        ["SAFETY-013"],
        isolation_no_policy
    ),
    (
        "scenario_isolation_automation",
        ["SCEN-091"],
        ["SAFETY-013", "SAFETY-014"],
        isolation_automation
    ),
    (
        "scenario_isolation_mid_dose",
        ["SCEN-092"],
        ["SAFETY-001", "SAFETY-016"],
        isolation_mid_dose
    ),
    (
        "scenario_isolation_corrupt_policy",
        ["SCEN-094"],
        ["SAFETY-013", "SAFETY-019"],
        isolation_corrupt_policy
    ),
    (
        "scenario_long_isolation",
        ["SCEN-107"],
        ["SAFETY-008", "SAFETY-013", "SAFETY-014"],
        long_isolation
    ),
    (
        "scenario_policy_activation",
        ["SCEN-095"],
        ["SAFETY-019"],
        policy_activation
    ),
    (
        "scenario_offline_budget",
        ["SCEN-096"],
        ["SAFETY-014"],
        offline_budget
    ),
    (
        "scenario_isolated_restart",
        ["SCEN-097"],
        ["SAFETY-014", "SAFETY-015"],
        isolated_restart
    ),
    (
        "scenario_isolated_no_wall_clock",
        ["SCEN-098"],
        ["SAFETY-015", "SAFETY-002"],
        isolated_no_wall_clock
    ),
    (
        "scenario_required_measurement",
        ["SCEN-099"],
        ["SAFETY-017"],
        required_measurement
    ),
    (
        "scenario_reconciliation",
        ["SCEN-100"],
        ["SAFETY-016"],
        reconciliation
    ),
    (
        "scenario_duplicate_replay",
        ["SCEN-101"],
        ["SAFETY-016"],
        duplicate_replay
    ),
    (
        "scenario_restart_mid_replay",
        ["SCEN-102"],
        ["SAFETY-016", "SAFETY-010"],
        restart_mid_replay
    ),
    (
        "scenario_stale_policy",
        ["SCEN-103"],
        ["SAFETY-019"],
        stale_policy
    ),
    (
        "scenario_history_gap",
        ["SCEN-104"],
        ["SAFETY-020"],
        history_gap
    ),
    (
        "scenario_advisory_measurement",
        ["SCEN-105", "SCEN-106"],
        ["SAFETY-017"],
        advisory_measurement
    ),
    (
        "scenario_sleeping_manual_water",
        ["SCEN-113"],
        ["SAFETY-001", "SAFETY-010"],
        sleeping_manual_water
    ),
    (
        "scenario_sleeping_safety_refusal",
        ["SCEN-114"],
        ["SAFETY-003", "SAFETY-012"],
        sleeping_safety_refusal
    ),
    (
        "scenario_sleep_budget_cooldown",
        ["SCEN-115"],
        ["SAFETY-014", "SAFETY-015"],
        sleep_budget_cooldown
    ),
    (
        "scenario_sleeping_intent_expiry",
        ["SCEN-116"],
        ["SAFETY-002"],
        sleeping_intent_expiry
    ),
    (
        "scenario_battery_awake_cycle",
        ["SCEN-117"],
        ["SAFETY-001", "SAFETY-011"],
        battery_awake_cycle
    ),
    (
        "scenario_first_demo",
        [],
        ["SAFETY-006", "SAFETY-008"],
        first_demo
    ),
);

/// Resolves CLI names and rejects unknown or duplicate requests.
pub fn select(names: &[String]) -> Result<Vec<&'static Scenario>> {
    if names.is_empty() {
        return Ok(catalogue().iter().collect());
    }
    let mut selected = Vec::new();
    for name in names {
        if name == "scenario_battery" {
            for scenario in catalogue().iter().filter(|scenario| {
                matches!(
                    scenario.name,
                    "scenario_sleeping_manual_water"
                        | "scenario_sleeping_safety_refusal"
                        | "scenario_sleep_budget_cooldown"
                        | "scenario_sleeping_intent_expiry"
                        | "scenario_battery_awake_cycle"
                )
            }) {
                selected.push(scenario);
            }
            continue;
        }
        // `scenario_reconciliation` is the documented aggregate command for
        // M8-016. Keep a private selector for focused verification of the
        // underlying SCEN-100 case without duplicating it in the catalogue.
        if name == "scenario_reconciliation_core" {
            let scenario = catalogue()
                .iter()
                .find(|scenario| scenario.name == "scenario_reconciliation")
                .context("the reconciliation scenario is missing from the catalogue")?;
            selected.push(scenario);
            continue;
        }
        if name == "scenario_isolation" {
            for scenario in catalogue().iter().filter(|scenario| {
                matches!(
                    scenario.name,
                    "scenario_reconnect_fresh_sync"
                        | "scenario_isolation_no_policy"
                        | "scenario_isolation_automation"
                        | "scenario_isolation_mid_dose"
                        | "scenario_isolation_corrupt_policy"
                        | "scenario_long_isolation"
                )
            }) {
                if selected
                    .iter()
                    .any(|existing: &&Scenario| existing.name == scenario.name)
                {
                    bail!("scenario `{}` was requested more than once", scenario.name);
                }
                selected.push(scenario);
            }
            continue;
        }
        if name == "scenario_reconciliation" {
            for scenario in catalogue().iter().filter(|scenario| {
                matches!(
                    scenario.name,
                    "scenario_policy_activation"
                        | "scenario_offline_budget"
                        | "scenario_isolated_restart"
                        | "scenario_isolated_no_wall_clock"
                        | "scenario_required_measurement"
                        | "scenario_reconciliation"
                        | "scenario_duplicate_replay"
                        | "scenario_restart_mid_replay"
                        | "scenario_stale_policy"
                        | "scenario_history_gap"
                        | "scenario_advisory_measurement"
                )
            }) {
                selected.push(scenario);
            }
            continue;
        }
        let scenario = catalogue()
            .iter()
            .find(|scenario| scenario.name == name)
            .ok_or_else(|| anyhow::anyhow!("unknown scenario `{name}`; use --list"))?;
        if selected
            .iter()
            .any(|existing: &&Scenario| existing.name == scenario.name)
        {
            bail!("scenario `{name}` was requested more than once");
        }
        selected.push(scenario);
    }
    Ok(selected)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every `e2e` scenario the failure catalogue assigns to M8 is implemented,
    /// exactly once, and no entry claims an id the catalogue does not define.
    ///
    /// Read out of `docs/testing/failure-scenarios.md` rather than duplicated
    /// here, for the same reason the ADR-005 kind tests read their ADR: two
    /// hand-maintained lists that are never compared will disagree, and the one
    /// that disagrees silently is the coverage claim.
    #[test]
    fn every_m8_scenario_in_the_catalogue_is_implemented() {
        let document = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../docs/testing/failure-scenarios.md"),
        )
        .expect("the failure-scenario catalogue is part of the repository");

        // A scenario is `### SCEN-NNN <title>` followed, within its section, by
        // a bullet naming its level and milestone.
        let mut required: Vec<String> = Vec::new();
        let mut current: Option<String> = None;
        for line in document.lines() {
            if let Some(rest) = line.strip_prefix("### SCEN-") {
                let id: String = rest.chars().take_while(char::is_ascii_digit).collect();
                current = (id.len() == 3).then(|| format!("SCEN-{id}"));
            } else if line.starts_with("- **Level**")
                && line.contains("e2e")
                && line.contains("**Milestone** M8")
                && let Some(id) = current.take()
            {
                required.push(id);
            }
        }
        assert!(
            required.len() >= 26,
            "the catalogue parser found only {} M8 e2e scenarios, which means the document \
             shape changed and this test stopped checking anything",
            required.len()
        );

        let mut claimed: Vec<&str> = catalogue().iter().flat_map(|s| s.covers).copied().collect();
        claimed.sort_unstable();
        let mut deduped = claimed.clone();
        deduped.dedup();
        assert_eq!(
            claimed, deduped,
            "two catalogue entries claim the same numbered scenario"
        );

        let missing: Vec<&String> = required
            .iter()
            .filter(|id| !claimed.contains(&id.as_str()))
            .collect();
        assert!(
            missing.is_empty(),
            "M8 e2e scenarios not implemented: {missing:?}"
        );

        for id in &claimed {
            assert!(
                document.contains(&format!("### {id} ")),
                "{id} is claimed by a scenario but is not in the failure catalogue"
            );
        }
    }

    /// F-080-22: every scenario says what it proves, in metadata rather than in
    /// a comment, so the claim can be read back by a person or a report.
    #[test]
    fn every_scenario_declares_its_coverage() {
        for scenario in catalogue() {
            assert!(
                !scenario.covers.is_empty() || scenario.name == "scenario_first_demo",
                "{} names no numbered scenario",
                scenario.name
            );
            for invariant in scenario.proves {
                assert!(
                    invariant.starts_with("SAFETY-") && invariant.len() == 10,
                    "{}: `{invariant}` is not a safety-invariant id",
                    scenario.name
                );
            }
        }
    }

    #[test]
    fn catalogue_names_are_unique_and_selectable() {
        let mut names = catalogue().iter().map(|s| s.name).collect::<Vec<_>>();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), catalogue().len());
        assert!(select(&["scenario_first_demo".to_owned()]).is_ok());
        assert_eq!(select(&["scenario_isolation".to_owned()]).unwrap().len(), 6);
        assert_eq!(
            select(&["scenario_reconciliation".to_owned()])
                .unwrap()
                .len(),
            11
        );
        assert!(select(&["missing".to_owned()]).is_err());
    }
}
