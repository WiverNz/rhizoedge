//! The periodic plant-evaluation loop (M5-012).
//!
//! M6 extends this into the control loop. In M5 it evaluates, records, and
//! measures — and **publishes nothing**. There is no MQTT client in this
//! module's signature at all, which is the strongest form the promise can take:
//! the loop could not publish a command if it wanted to.
//!
//! Two economies matter and are both about not drowning the operator:
//!
//! - A `plant_recommendations` row is written **only when the decision or the
//!   reason set changes**. Writing one per tick would produce 2 880 rows per
//!   plant per day recording that nothing happened.
//! - INFO is logged only when a recommendation **changes**. A tick reaching the
//!   same conclusion is not news (ADR-010).
use std::sync::Arc;
use std::time::Duration as StdDuration;

use rhizo_domain::Clock;
use rhizo_domain::recommend::{Decision, Reason, Recommendation, recommend};
use rhizo_storage::EdgeDb;
use rhizo_storage::repo::plant as plant_repo;
use tokio::sync::watch;

use crate::metrics::Metrics;
use crate::plant;

/// How often the loop evaluates every plant.
pub const DEFAULT_TICK_INTERVAL: StdDuration = StdDuration::from_secs(30);

/// Renders a structured reason for the wire.
///
/// **The one place reasons become prose.** Everything else — storage, the
/// engine, the tests — carries the typed value, so a wording change is one edit
/// and an assertion never depends on English.
#[must_use]
pub fn reason_json(reason: &Reason) -> serde_json::Value {
    let mut value = serde_json::json!({ "code": reason.code() });
    let detail = match *reason {
        Reason::MoistureBelowTarget { vwc, target_min }
        | Reason::MoistureAtOrAboveTarget { vwc, target_min } => {
            serde_json::json!({ "vwc": vwc, "target_min": target_min })
        }
        Reason::DryFor { minutes, required } | Reason::NotDryLongEnough { minutes, required } => {
            serde_json::json!({ "minutes": minutes, "required": required })
        }
        Reason::LastWatering { hours_ago } => serde_json::json!({ "hours_ago": hours_ago }),
        Reason::CooldownActive {
            hours_ago,
            required_hours,
        } => serde_json::json!({ "hours_ago": hours_ago, "required_hours": required_hours }),
        Reason::SampleStale {
            age_seconds,
            max_age_seconds,
        } => serde_json::json!({ "age_seconds": age_seconds, "max_age_seconds": max_age_seconds }),
        Reason::LockedOut { reason } => {
            serde_json::json!({ "lockout": plant::lockout_name(reason) })
        }
        Reason::Trend { vwc_per_hour } => serde_json::json!({ "vwc_per_hour": vwc_per_hour }),
        Reason::NeverWatered
        | Reason::SampleMissing
        | Reason::SampleInvalid
        | Reason::SensorUnhealthy
        | Reason::NoActuator
        | Reason::TrendUnavailable => serde_json::json!({}),
    };
    if let (Some(object), Some(detail)) = (value.as_object_mut(), detail.as_object()) {
        for (key, v) in detail {
            object.insert(key.clone(), v.clone());
        }
    }
    if let Some(object) = value.as_object_mut() {
        // Prose is produced by exactly one function, called from exactly one
        // place. Storing it alongside the code means a persisted recommendation
        // keeps the sentence it actually showed, and no reader has to
        // reconstruct English from a code and a number.
        object.insert(
            "message".to_owned(),
            serde_json::Value::String(reason_text(reason)),
        );
    }
    value
}

/// The prose an operator reads. Rendered here and nowhere else.
#[must_use]
pub fn reason_text(reason: &Reason) -> String {
    match *reason {
        Reason::MoistureBelowTarget { vwc, target_min } => {
            format!("moisture {vwc:.1}% is below the target minimum of {target_min:.1}%")
        }
        Reason::MoistureAtOrAboveTarget { vwc, target_min } => {
            format!("moisture {vwc:.1}% is at or above the target minimum of {target_min:.1}%")
        }
        Reason::DryFor { minutes, required } => {
            format!("dry for {minutes} minutes, past the {required}-minute confirmation")
        }
        Reason::NotDryLongEnough { minutes, required } => {
            format!("dry for only {minutes} of the {required} minutes required")
        }
        Reason::LastWatering { hours_ago } => {
            format!("last watered {hours_ago:.1} hours ago")
        }
        Reason::CooldownActive {
            hours_ago,
            required_hours,
        } => format!("watered {hours_ago:.1} hours ago; the cooldown is {required_hours:.1} hours"),
        Reason::NeverWatered => "no watering has been recorded for this plant".to_owned(),
        Reason::SampleMissing => "there is no usable moisture reading".to_owned(),
        Reason::SampleInvalid => "the latest moisture reading failed validation".to_owned(),
        Reason::SampleStale {
            age_seconds,
            max_age_seconds,
        } => format!(
            "the latest reading is {age_seconds} seconds old; {max_age_seconds} is the limit"
        ),
        Reason::SensorUnhealthy => "a sensor this plant depends on is unhealthy".to_owned(),
        Reason::NoActuator => {
            "this plant has no pump, so watering is something a person does".to_owned()
        }
        Reason::LockedOut { reason } => {
            format!("watering is locked out: {}", plant::lockout_name(reason))
        }
        Reason::TrendUnavailable => "not enough recent data to fit a trend".to_owned(),
        Reason::Trend { vwc_per_hour } => format!("moisture is changing by {vwc_per_hour:.2} %/h"),
    }
}

/// The full recommendation, as the API returns it.
#[must_use]
pub fn recommendation_json(
    recommendation: &Recommendation,
    evaluated_at: Option<i64>,
) -> serde_json::Value {
    serde_json::json!({
        "recommendation": recommendation.decision.as_str(),
        "recommended_ml": recommendation.recommended_ml,
        "confidence": recommendation.confidence,
        "reasons": recommendation.reasons.iter().map(reason_json).collect::<Vec<_>>(),
        "blocked_by": recommendation.blocked_by.map(plant::lockout_name),
        "evaluated_at": evaluated_at.and_then(crate::api::support::timestamp),
    })
}

/// Evaluates one plant and records whatever changed.
///
/// Returns the recommendation, so a test can assert on it without waiting for a
/// tick.
pub async fn evaluate_plant(
    db: &EdgeDb,
    plant_id: &str,
    clock: &dyn Clock,
    metrics: &Metrics,
) -> Result<Option<Recommendation>, rhizo_storage::StorageError> {
    let Some(loaded) = plant::load(db, plant_id).await? else {
        return Ok(None);
    };
    let now = clock.now();
    let analysis = plant::analyse(db, &loaded, now).await?;
    let recommendation = recommend(&analysis.inputs);

    // Detection and stuck-sensor tracking run on the same pass, so what the
    // recommendation saw and what the ledger records come from one reading.
    let detection = plant::detect::run(db, &loaded, now).await?;
    if detection.recorded {
        metrics.manual_watering_detected.inc();
    }
    plant::detect::stuck(db, &loaded, now).await?;

    // Thresholds inform; they never water. Evaluated for every plant, actuator
    // or not.
    for (kind, crossing) in crate::control::threshold::run(db, &loaded, now).await? {
        metrics
            .threshold_crossings
            .with_label_values(&[&kind, crossing.to.as_str()])
            .inc();
        if crossing.to == rhizo_domain::threshold::Severity::Normal {
            tracing::info!(
                plant_id = %plant_id,
                measurement_kind = %kind,
                old_state = %crossing.from.as_str(),
                new_state = %crossing.to.as_str(),
                value = ?crossing.value,
                "measurement threshold recovered"
            );
        } else {
            tracing::warn!(
                plant_id = %plant_id,
                measurement_kind = %kind,
                old_state = %crossing.from.as_str(),
                new_state = %crossing.to.as_str(),
                value = ?crossing.value,
                "measurement threshold changed"
            );
        }
    }

    // EC is recorded and trended; a rise raises a warning and no more.
    if let Some(warning) = rhizo_domain::ec::ec_warning(
        analysis.latest_ec,
        rhizo_domain::ec::DEFAULT_WARNING_HIGH_US_CM,
    ) {
        plant_repo::record_plant_event(
            db,
            Some(plant_id),
            &format!(
                "plant:{plant_id}:ec_high:{}",
                now.timestamp_millis() / 3_600_000
            ),
            "ec_high",
            "warning",
            Some(&serde_json::json!({
                "us_cm": warning.us_cm,
                "warning_high_us_cm": warning.warning_high_us_cm,
                "trend_us_cm_per_hour": analysis.ec_trend.map(|t| t.0),
            })),
            now.timestamp_millis(),
        )
        .await?;
    }

    // Persist on change only.
    let previous = plant_repo::latest_recommendation(db, plant_id).await?;
    let previous_codes: Vec<String> = previous
        .as_ref()
        .and_then(|row| serde_json::from_str::<Vec<serde_json::Value>>(&row.reasons_json).ok())
        .map(|list| {
            list.iter()
                .filter_map(|v| {
                    v.get("code")
                        .and_then(|c| c.as_str())
                        .map(ToOwned::to_owned)
                })
                .collect()
        })
        .unwrap_or_default();
    let codes: Vec<String> = recommendation
        .reasons
        .iter()
        .map(|r| r.code().to_owned())
        .collect();
    let blocked = recommendation.blocked_by.map(plant::lockout_name);
    let changed = previous.as_ref().is_none_or(|row| {
        row.decision != recommendation.decision.as_str()
            || row.blocked_by != blocked
            || previous_codes != codes
    });
    if changed {
        let reasons = recommendation
            .reasons
            .iter()
            .map(reason_json)
            .collect::<Vec<_>>();
        plant_repo::insert_recommendation(
            db,
            plant_id,
            &plant_repo::RecommendationRow {
                decision: recommendation.decision.as_str().to_owned(),
                recommended_ml: recommendation.recommended_ml.map(f64::from),
                confidence: f64::from(recommendation.confidence),
                reasons_json: serde_json::Value::Array(reasons).to_string(),
                blocked_by: blocked,
                evaluated_at: now.timestamp_millis(),
            },
        )
        .await?;
        metrics
            .recommendations
            .with_label_values(&[recommendation.decision.as_str()])
            .inc();
        tracing::info!(
            plant_id = %plant_id,
            old_decision = %previous.as_ref().map_or("none", |row| row.decision.as_str()),
            new_decision = %recommendation.decision.as_str(),
            recommended_ml = ?recommendation.recommended_ml,
            reasons = ?codes,
            "recommendation changed"
        );
    }

    let (_state, transition) = plant::state::apply(db, plant_id, &recommendation, now).await?;
    if let Some(transition) = transition {
        tracing::info!(
            plant_id = %plant_id,
            old_state = %transition
                .from
                .map_or_else(|| "none".to_owned(), plant::state::state_name),
            new_state = %plant::state::state_name(transition.to),
            "plant state changed"
        );
    }
    Ok(Some(recommendation))
}

/// One pass over every plant, plus the gauges.
pub async fn tick(
    db: &EdgeDb,
    clock: &dyn Clock,
    metrics: &Metrics,
) -> Result<(), rhizo_storage::StorageError> {
    let plants = plant_repo::list(db, None, 500).await?;
    metrics
        .plants_total
        .set(i64::try_from(plants.len()).unwrap_or(i64::MAX));
    let mut by_state: std::collections::BTreeMap<&'static str, i64> = [
        ("healthy", 0),
        ("drying", 0),
        ("water_recommended", 0),
        ("sensor_fault", 0),
        ("watering_locked", 0),
        ("other", 0),
    ]
    .into_iter()
    .collect();
    for row in plants {
        evaluate_plant(db, &row.plant_id, clock, metrics).await?;
        let state = plant_repo::plant_state(db, &row.plant_id)
            .await?
            .unwrap_or_else(|| "other".to_owned());
        let bucket = match state.as_str() {
            "healthy" => "healthy",
            "drying" => "drying",
            "water_recommended" => "water_recommended",
            "sensor_fault" => "sensor_fault",
            "watering_locked" => "watering_locked",
            _ => "other",
        };
        *by_state.entry(bucket).or_insert(0) += 1;
    }
    for (state, count) in by_state {
        metrics.plant_state.with_label_values(&[state]).set(count);
    }
    Ok(())
}

/// Runs the evaluation loop until shutdown.
///
/// Note the absence of an MQTT client: this loop cannot publish (M5-012).
pub async fn run(
    db: EdgeDb,
    clock: Arc<dyn Clock>,
    metrics: Metrics,
    interval: StdDuration,
    mut shutdown: watch::Receiver<bool>,
) -> Result<(), String> {
    let mut ticker = tokio::time::interval(interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            changed = shutdown.changed() => if changed.is_err() || *shutdown.borrow() { return Ok(()); },
            _ = ticker.tick() => {
                if let Err(error) = tick(&db, clock.as_ref(), &metrics).await {
                    // A failed pass is not fatal: the next one re-reads
                    // everything from storage, which is the only state there is.
                    tracing::warn!(%error, "a plant evaluation pass failed");
                }
            }
        }
    }
}

/// A decision rendered for a log line or a badge.
#[must_use]
pub const fn decision_name(decision: Decision) -> &'static str {
    decision.as_str()
}

/// The evaluation loop's own tests.
///
/// The module is named `recommend` so `cargo test -p edge-controller recommend::`
/// reaches it, which is the filter M5-012's verification section names.
#[cfg(test)]
mod recommend {
    use crate::api::testsupport::{TestApi, base};
    use chrono::Duration;
    use rhizo_domain::recommend::Decision;
    use rhizo_storage::repo::plant as plant_repo;

    /// A plant with a control binding, a pump, a moisture policy, and a drying
    /// series at the device's real 300-second cadence.
    async fn drying(api: &TestApi) {
        api.with_device().await;
        api.plant("monstera-01").await;
        api.bind_control("monstera-01").await;
        api.moisture_policy("monstera-01").await;
        api.json(
            "PUT",
            "/api/v1/plants/monstera-01/bindings/actuator",
            serde_json::json!({ "device_id": "plant-node-01", "actuator_id": "pump-0" }),
        )
        .await;
        // Six hours of five-minute readings falling from 40 % through the 28 %
        // target: the cadence a `telemetry_interval_seconds` of 300 produces.
        for i in 0i64..72 {
            api.sample(
                base() - Duration::minutes((71 - i) * 5),
                40.0 - i as f64 * 0.25,
            )
            .await;
        }
    }

    /// The milestone's headline behaviour: drying produces `water_recommended`
    /// with a non-empty structured reason list.
    #[tokio::test]
    async fn drying_produces_water_recommended_with_reasons() {
        let api = TestApi::start().await;
        drying(&api).await;
        let answer = api.evaluate("monstera-01").await;
        assert_eq!(answer.decision, Decision::Water, "{answer:?}");
        assert!(!answer.reasons.is_empty());
        let codes: Vec<&str> = answer.reasons.iter().map(|r| r.code()).collect();
        assert!(codes.contains(&"moisture_below_target"), "{codes:?}");
        assert!(codes.contains(&"dry_for"), "{codes:?}");
        assert_eq!(answer.recommended_ml, Some(40.0));

        let (status, body) = api.get("/api/v1/plants/monstera-01/recommendation").await;
        assert_eq!(status, axum::http::StatusCode::OK);
        assert_eq!(body["recommendation"], "water");
        assert_eq!(body["recommended_ml"], 40.0);
        assert!(body["reasons"].as_array().unwrap().len() >= 3);
        assert!(
            body["reasons"][0]["message"].is_string(),
            "reasons render to prose in exactly one place, and this is where it shows"
        );
        assert!(body["evaluated_at"].is_string());

        let (_, plant) = api.get("/api/v1/plants/monstera-01").await;
        assert_eq!(plant["state"], "water_recommended");
        assert_eq!(
            plant["auto_watering_enabled"], false,
            "a plant may sit in water_recommended indefinitely with automation off"
        );
    }

    /// A row is written only when the decision or the reason set changes.
    #[tokio::test]
    async fn a_recommendation_row_is_written_only_on_change() {
        let api = TestApi::start().await;
        drying(&api).await;
        api.evaluate("monstera-01").await;
        let after_first = plant_repo::recommendation_count(&api.db, "monstera-01")
            .await
            .unwrap();
        assert_eq!(after_first, 1);
        for _ in 0..10 {
            api.clock.advance(Duration::seconds(30));
            api.evaluate("monstera-01").await;
        }
        assert_eq!(
            plant_repo::recommendation_count(&api.db, "monstera-01")
                .await
                .unwrap(),
            after_first,
            "ten ticks reaching the same conclusion are not ten rows"
        );

        // A real change writes exactly one more.
        api.clock.advance(Duration::minutes(5));
        api.sample(api.clock.now(), 44.0).await;
        api.evaluate("monstera-01").await;
        assert_eq!(
            plant_repo::recommendation_count(&api.db, "monstera-01")
                .await
                .unwrap(),
            after_first + 1
        );
    }

    /// State transitions are persisted; steady state is not.
    #[tokio::test]
    async fn plant_state_transitions_are_persisted_once() {
        let api = TestApi::start().await;
        drying(&api).await;
        api.evaluate("monstera-01").await;
        let events = plant_repo::plant_events(&api.db, "monstera-01", 100)
            .await
            .unwrap();
        let transitions = events
            .iter()
            .filter(|(kind, ..)| kind == "plant_state_changed")
            .count();
        assert_eq!(transitions, 1);
        for _ in 0..5 {
            api.clock.advance(Duration::seconds(30));
            api.evaluate("monstera-01").await;
        }
        let events = plant_repo::plant_events(&api.db, "monstera-01", 100)
            .await
            .unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|(kind, ..)| kind == "plant_state_changed")
                .count(),
            transitions,
            "a tick that changes nothing writes nothing"
        );
    }

    /// A plant with no readings at all is blocked, not silently healthy.
    #[tokio::test]
    async fn a_plant_with_no_readings_is_blocked_rather_than_healthy() {
        let api = TestApi::start().await;
        api.with_device().await;
        api.plant("monstera-01").await;
        api.bind_control("monstera-01").await;
        api.moisture_policy("monstera-01").await;
        api.json(
            "PUT",
            "/api/v1/plants/monstera-01/bindings/actuator",
            serde_json::json!({ "device_id": "plant-node-01", "actuator_id": "pump-0" }),
        )
        .await;
        let answer = api.evaluate("monstera-01").await;
        assert_eq!(answer.decision, Decision::Blocked);
        assert_eq!(
            answer.blocked_by,
            Some(rhizo_domain::LockoutReason::SensorFault)
        );
        let (_, plant) = api.get("/api/v1/plants/monstera-01").await;
        assert_eq!(plant["state"], "sensor_fault");
    }

    /// A binding owns the complete `(device, sensor, point, kind)` identity.
    /// Another probe on the same device may report the same kind and point, but
    /// its data is not evidence about this plant. Omitting `sensor_id` from the
    /// storage lookup used to let this unbound stream build dry duration and
    /// produce a positive watering recommendation.
    #[tokio::test]
    async fn an_unbound_same_kind_stream_cannot_recommend_water() {
        let api = TestApi::start().await;
        api.with_device().await;
        api.plant("monstera-01").await;
        api.bind_control("monstera-01").await;
        api.moisture_policy("monstera-01").await;
        api.json(
            "PUT",
            "/api/v1/plants/monstera-01/bindings/actuator",
            serde_json::json!({ "device_id": "plant-node-01", "actuator_id": "pump-0" }),
        )
        .await;

        for i in 0i64..72 {
            let at = base() - Duration::minutes((71 - i) * 5);
            sqlx::query(
                "INSERT INTO measurements(device_id,sensor_id,point,kind,value_num,unit,quality,received_at,batch_id,origin) \
                 VALUES('plant-node-01','soil-unbound','default','soil_moisture',20.0,'percent','ok',?,?, 'live')",
            )
            .bind(at.timestamp_millis())
            .bind(format!("unbound-{i}"))
            .execute(api.db.pool())
            .await
            .unwrap();
        }

        let answer = api.evaluate("monstera-01").await;
        assert_eq!(answer.decision, Decision::Blocked, "{answer:?}");
        assert_eq!(
            answer.blocked_by,
            Some(rhizo_domain::LockoutReason::SensorFault)
        );
        assert!(
            answer
                .reasons
                .iter()
                .any(|reason| reason.code() == "sample_missing")
        );
        assert_eq!(answer.recommended_ml, None);
    }

    /// A required binding is not satisfied merely because its sensor is marked
    /// healthy in status. Missing measurement evidence must fail closed before
    /// M6 can consume the recommendation.
    #[tokio::test]
    async fn a_missing_required_measurement_cannot_recommend_water() {
        let api = TestApi::start().await;
        drying(&api).await;
        api.json(
            "PUT",
            "/api/v1/plants/monstera-01/bindings/sensors",
            serde_json::json!({
                "device_id": "plant-node-01",
                "sensor_id": "leak-0",
                "point": "tray",
                "kind": "leak_state",
                "role": "required"
            }),
        )
        .await;
        api.json(
            "PUT",
            "/api/v1/plants/monstera-01/measurement-policies/leak_state",
            serde_json::json!({ "stale_after_ms": 900_000 }),
        )
        .await;

        let answer = api.evaluate("monstera-01").await;
        assert_eq!(answer.decision, Decision::Blocked, "{answer:?}");
        assert_eq!(
            answer.blocked_by,
            Some(rhizo_domain::LockoutReason::SensorFault)
        );
        assert!(
            answer
                .reasons
                .iter()
                .any(|reason| reason.code() == "sensor_unhealthy")
        );
        assert_eq!(answer.recommended_ml, None);
    }

    /// SAFETY-005: a reading older than the control-freshness threshold blocks,
    /// and the threshold comes from the telemetry cadence, never from a power
    /// field.
    #[tokio::test]
    async fn safety_005_a_stale_reading_blocks_and_names_the_limit() {
        let api = TestApi::start().await;
        drying(&api).await;
        api.clock.advance(Duration::hours(4));
        let answer = api.evaluate("monstera-01").await;
        assert_eq!(answer.decision, Decision::Blocked, "{answer:?}");
        assert_eq!(
            answer.blocked_by,
            Some(rhizo_domain::LockoutReason::StaleData)
        );
        let (_, body) = api.get("/api/v1/plants/monstera-01/recommendation").await;
        let stale = body["reasons"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["code"] == "sample_stale")
            .unwrap();
        assert!(stale["max_age_seconds"].as_i64().unwrap() > 0);
    }

    /// The trend is `None` with fewer than five valid samples in the window,
    /// and the answer says so rather than inventing a slope.
    #[tokio::test]
    async fn a_trend_is_absent_with_fewer_than_five_samples() {
        let api = TestApi::start().await;
        api.with_device().await;
        api.plant("monstera-01").await;
        api.bind_control("monstera-01").await;
        api.moisture_policy("monstera-01").await;
        for i in 0i64..4 {
            api.sample(base() - Duration::minutes((3 - i) * 60), 30.0 - i as f64)
                .await;
        }
        let answer = api.evaluate("monstera-01").await;
        let codes: Vec<&str> = answer.reasons.iter().map(|r| r.code()).collect();
        assert!(codes.contains(&"trend_unavailable"), "{codes:?}");
        assert!(answer.confidence < 1.0, "sparse data lowers confidence");
    }

    /// The metrics M5-012 names are exported and bounded.
    #[tokio::test]
    async fn the_plant_metrics_are_exported() {
        let api = TestApi::start().await;
        drying(&api).await;
        super::tick(&api.db, api.clock.as_ref(), &api.state.metrics)
            .await
            .unwrap();
        let text = rhizo_telemetry::render_prometheus();
        for name in [
            "plants_total",
            "plant_state",
            "recommendations_total",
            "manual_watering_detected_total",
            "threshold_crossings_total",
        ] {
            assert!(text.contains(name), "{name} is missing from /metrics");
        }
        assert!(api.state.metrics.plants_total.get() >= 1);
        assert!(
            !text
                .lines()
                .filter(|l| l.starts_with("plant_state"))
                .any(|l| l.contains("plant_id=")),
            "no metric may be labelled by plant id"
        );
    }
}
