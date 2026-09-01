//! Deterministic scenario catalogue.

use crate::harness::Harness;
use anyhow::{Context, Result, bail, ensure};
use reqwest::Method;
use serde_json::{Value, json};
use std::time::Duration;
use std::{future::Future, pin::Pin};

/// Boxed scenario future.
pub type ScenarioFuture<'a> = Pin<Box<dyn Future<Output = Result<()>> + Send + 'a>>;

/// One independently runnable assembled-system scenario.
pub struct Scenario {
    /// Stable CLI name.
    pub name: &'static str,
    /// Safety invariants re-verified by the scenario.
    pub proves: &'static [&'static str],
    /// Observable-state implementation.
    pub run: for<'a> fn(&'a Harness) -> ScenarioFuture<'a>,
}

fn pending<'a>(_: &'a Harness) -> ScenarioFuture<'a> {
    Box::pin(async { bail!("scenario implementation is not yet present; refusing a false green") })
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
            json!({
                "profile_id":"scenario-profile", "name":"Scenario profile",
                "target_min_vwc":28.0, "target_max_vwc":45.0, "dose_ml":40.0,
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
    let mut bindings = vec![("soil-0", "default", "soil_moisture", "control")];
    if automatic {
        bindings.extend([
            ("tank-0", "reservoir", "tank_level", "required"),
            ("leak-0", "tray", "leak_state", "required"),
            ("weight-0", "default", "pot_weight", "advisory"),
        ]);
    }
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
    Ok(device)
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
                    .filter(|m| m.topic.contains("/commands/"))
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
            events >= 1 && events <= 3,
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
            h.clear_mqtt().await;
            tokio::time::sleep(Duration::from_millis(100)).await;
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
    bail!("expected MQTT publication was not captured")
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
        let before: i64 = sqlx::query_scalar("SELECT count(DISTINCT batch_id) FROM measurements")
            .fetch_one(&h.sqlite)
            .await?;
        h.stop_service("mosquitto")?;
        tokio::time::sleep(Duration::from_secs(2)).await;
        let during: i64 = sqlx::query_scalar("SELECT count(DISTINCT batch_id) FROM measurements")
            .fetch_one(&h.sqlite)
            .await?;
        ensure!(
            during <= before + 1,
            "telemetry advanced while broker was stopped: {before} -> {during}"
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
        fault(h, "stale-soil", true).await?;
        h.clear_mqtt().await;
        wait_lockout(h, Some("stale_data"), 200).await?;
        let commands = h
            .mqtt()
            .await
            .into_iter()
            .filter(|m| m.topic.contains("/commands/") && !m.topic.ends_with("/result/ack"))
            .count();
        ensure!(
            commands == 0,
            "stale sensor allowed {commands} command publications"
        );
        fault(h, "stale-soil", false).await?;
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
        // 300 ms at the suite's 600x scale is 180 virtual seconds: beyond the
        // command's two-minute lifetime while the clean-session subscriber is
        // absent.
        tokio::time::sleep(Duration::from_millis(300)).await;
        h.start_service("device-simulator")?;
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
                .any(|message| message.topic.contains("/commands/")),
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
        h.clear_mqtt().await;
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
            "restart published {publications} water commands"
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
            "SELECT count(*),count(DISTINCT event_id) FROM synced_events WHERE edge_id='home-01'",
        )
        .fetch_one(&h.postgres)
        .await?;
        ensure!(
            rows == emitted && distinct == emitted,
            "PostgreSQL ledger mismatch: rows={rows}, distinct={distinct}, emitted={emitted}"
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
        let after: i64 =
            sqlx::query_scalar("SELECT count(*) FROM synced_events WHERE edge_id='home-01'")
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
            if delivered >= 40.0 && status == "completed" {
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
    bail!("simulator state did not reach the expected condition")
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
            s["delivered_today_ml"].as_f64().unwrap_or_default() >= 39.99
        })
        .await?;
        ensure!(dosed["delivered_today_ml"].as_f64().unwrap_or_default() <= 40.01);
        h.simulator_post("/sim/state", json!({"moisture_vwc":46.0}))
            .await?;
        let reconnected = wait_simulator(h, 800, |s| s["connected"] == true).await?;
        ensure!(
            reconnected["delivered_today_ml"]
                .as_f64()
                .unwrap_or_default()
                <= 40.01,
            "more than one autonomous dose was delivered: {reconnected}"
        );
        for _ in 0..400 {
            let rows: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM watering_events WHERE plant_id='scenario-plant' AND origin='offline_autonomous' AND delivered_ml=40.0",
            )
            .fetch_one(&h.sqlite)
            .await?;
            if rows == 1 {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        bail!("exactly one autonomous 40 ml watering event was not reconciled")
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
        ensure!(completed["buffered_events"].as_u64().unwrap_or_default() > 0);
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
            s["delivered_today_ml"].as_f64().unwrap_or_default() >= 39.99
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
        // Start the edge shortly before the device's 48-hour isolation ends so
        // the reconnect and its replay are observed by a fresh edge process.
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
        wait_simulator(h, 400, |s| s["connected"] == true).await?;
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

macro_rules! scenarios {
    ($(($name:literal, [$($safety:literal),*] $(, $run:expr)?)),+ $(,)?) => {
        /// Complete M8 catalogue in deterministic execution order.
        pub fn catalogue() -> &'static [Scenario] {
            static SCENARIOS: &[Scenario] = &[$(Scenario {
                name: $name,
                proves: &[$($safety),*],
                run: scenario_run!($($run)?),
            }),+];
            SCENARIOS
        }
    };
}

macro_rules! scenario_run {
    () => {
        pending
    };
    ($run:expr) => {
        $run
    };
}

scenarios!(
    ("scenario_normal_telemetry", [], normal_telemetry),
    (
        "scenario_full_watering_cycle",
        ["SAFETY-006", "SAFETY-012"],
        full_watering_cycle
    ),
    (
        "scenario_recommendation_without_automation",
        [],
        recommendation_without_automation
    ),
    (
        "scenario_duplicate_command",
        ["SAFETY-001"],
        duplicate_command
    ),
    ("scenario_broker_restart", ["SAFETY-008"], broker_restart),
    ("scenario_stale_sensor", ["SAFETY-005"], stale_sensor),
    (
        "scenario_invalid_sensor",
        ["SAFETY-005", "SAFETY-012"],
        invalid_sensor
    ),
    (
        "scenario_clock_unsynced",
        ["SAFETY-002", "SAFETY-012"],
        clock_unsynced
    ),
    (
        "scenario_queued_command_expiry",
        ["SAFETY-002"],
        queued_command_expiry
    ),
    ("scenario_leak", ["SAFETY-003"], leak),
    ("scenario_tank_empty", ["SAFETY-004"], tank_empty),
    ("scenario_no_delivery", [], no_delivery),
    (
        "scenario_restart_mid_command",
        ["SAFETY-010"],
        restart_mid_command
    ),
    (
        "scenario_restart_mid_absorption",
        ["SAFETY-010"],
        restart_mid_absorption
    ),
    (
        "scenario_cloud_unavailable",
        ["SAFETY-008"],
        cloud_unavailable
    ),
    (
        "scenario_cloud_independence",
        ["SAFETY-009"],
        cloud_independence
    ),
    (
        "scenario_cloud_outage_recovery",
        ["SAFETY-008"],
        cloud_outage_recovery
    ),
    (
        "scenario_reconnect_fresh_sync",
        ["SAFETY-002", "SAFETY-015"],
        reconnect_fresh_sync
    ),
    (
        "scenario_isolation_no_policy",
        ["SAFETY-013"],
        isolation_no_policy
    ),
    (
        "scenario_isolation_automation",
        ["SAFETY-013", "SAFETY-014"],
        isolation_automation
    ),
    (
        "scenario_isolation_mid_dose",
        ["SAFETY-001", "SAFETY-016"],
        isolation_mid_dose
    ),
    (
        "scenario_isolation_corrupt_policy",
        ["SAFETY-013", "SAFETY-019"],
        isolation_corrupt_policy
    ),
    (
        "scenario_long_isolation",
        ["SAFETY-008", "SAFETY-013", "SAFETY-014"],
        long_isolation
    ),
    ("scenario_policy_activation", ["SAFETY-019"]),
    ("scenario_offline_budget", ["SAFETY-014"]),
    ("scenario_isolated_restart", ["SAFETY-014", "SAFETY-015"]),
    (
        "scenario_isolated_no_wall_clock",
        ["SAFETY-015", "SAFETY-002"]
    ),
    ("scenario_required_measurement", ["SAFETY-017"]),
    ("scenario_reconciliation", ["SAFETY-016"]),
    ("scenario_duplicate_replay", ["SAFETY-016"]),
    ("scenario_restart_mid_replay", ["SAFETY-016", "SAFETY-010"]),
    ("scenario_stale_policy", ["SAFETY-019"]),
    ("scenario_history_gap", ["SAFETY-020"]),
    ("scenario_advisory_measurement", ["SAFETY-017"]),
    (
        "scenario_sleeping_manual_water",
        ["SAFETY-001", "SAFETY-010"]
    ),
    (
        "scenario_sleeping_safety_refusal",
        ["SAFETY-003", "SAFETY-012"]
    ),
    (
        "scenario_sleep_budget_cooldown",
        ["SAFETY-014", "SAFETY-015"]
    ),
    ("scenario_sleeping_intent_expiry", ["SAFETY-002"]),
    ("scenario_battery_awake_cycle", ["SAFETY-001", "SAFETY-011"]),
    (
        "scenario_first_demo",
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

    #[test]
    fn catalogue_names_are_unique_and_selectable() {
        let mut names = catalogue().iter().map(|s| s.name).collect::<Vec<_>>();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), catalogue().len());
        assert!(select(&["scenario_first_demo".to_owned()]).is_ok());
        assert!(select(&["missing".to_owned()]).is_err());
    }
}
