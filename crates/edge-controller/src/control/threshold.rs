//! Per-plant threshold evaluation on the tick (M5-015).
//!
//! Kept strictly separate from actuation. A critical ambient temperature is real
//! and worth alerting on, and it is **not** a reason to pump water. Alerts are
//! raised whether or not the plant has an actuator: a monitoring-only plant is
//! the common case, and its critical readings matter just as much.
use chrono::{DateTime, Utc};
use rhizo_domain::threshold::{Crossing, Severity, ThresholdState, evaluate};
use rhizo_storage::EdgeDb;
use rhizo_storage::repo::{plant as plant_repo, query};

use crate::plant::Loaded;

/// The wire name of a severity.
#[must_use]
pub const fn severity_name(severity: Severity) -> &'static str {
    severity.as_str()
}

fn severity_from_str(name: &str) -> Severity {
    match name {
        "warning" => Severity::Warning,
        "critical" => Severity::Critical,
        _ => Severity::Normal,
    }
}

/// Evaluates every bound kind against its policy and records the crossings.
///
/// Returns one entry per transition, never one per tick.
pub async fn run(
    db: &EdgeDb,
    loaded: &Loaded,
    now: DateTime<Utc>,
) -> Result<Vec<(String, Crossing)>, rhizo_storage::StorageError> {
    let plant_id = loaded.plant.plant_id.as_str();
    let now_ms = now.timestamp_millis();
    let mut crossings = Vec::new();
    for bound in &loaded.sensors {
        let kind = bound.binding.kind.as_str().to_owned();
        let Some(policy) = loaded.policy(&bound.binding.kind) else {
            continue;
        };
        let device = bound.binding.device_id.to_string();
        let sensor = bound.binding.sensor_id.as_str();
        let point = bound.binding.point.as_str();
        let latest = query::latest_measurement(db, &device, sensor, point, &kind).await?;
        let stored = plant_repo::threshold_state(db, plant_id, &kind).await?;
        let mut state =
            stored
                .as_ref()
                .map_or_else(ThresholdState::default, |row| ThresholdState {
                    current: severity_from_str(&row.severity),
                    candidate: row.candidate.as_deref().map(severity_from_str),
                    candidate_since: row
                        .candidate_since
                        .and_then(DateTime::from_timestamp_millis),
                });
        // A reading older than the plant's freshness horizon is not evidence of
        // the present. It neither raises nor clears (SAFETY-012).
        let fresh = latest.as_ref().is_some_and(|row| {
            now_ms.saturating_sub(row.received_at) < i64::from(policy.stale_after_ms)
        });
        let value = latest
            .as_ref()
            .filter(|row| fresh && row.quality == "ok")
            .and_then(|row| row.value_num);
        let at = latest
            .as_ref()
            .and_then(|row| DateTime::from_timestamp_millis(row.received_at))
            .unwrap_or(now);
        let crossing = evaluate(&mut state, value, at, policy);
        plant_repo::put_threshold_state(
            db,
            plant_id,
            &kind,
            &plant_repo::ThresholdStateRow {
                severity: state.current.as_str().to_owned(),
                candidate: state.candidate.map(|s| s.as_str().to_owned()),
                candidate_since: state.candidate_since.map(|v| v.timestamp_millis()),
            },
            now_ms,
        )
        .await?;
        let Some(crossing) = crossing else { continue };
        let (event, severity) = match crossing.to {
            Severity::Critical => ("threshold.critical", "critical"),
            Severity::Warning => ("threshold.warning", "warning"),
            Severity::Normal => ("threshold.cleared", "info"),
        };
        let detail = serde_json::json!({
            "kind": kind,
            "from": crossing.from.as_str(),
            "to": crossing.to.as_str(),
            "value": crossing.value,
        });
        plant_repo::record_plant_event(
            db,
            Some(plant_id),
            &format!("plant:{plant_id}:{kind}:{event}:{now_ms}"),
            event,
            severity,
            Some(&detail),
            now_ms,
        )
        .await?;
        crossings.push((kind, crossing));
    }
    Ok(crossings)
}

#[cfg(test)]
mod tests {
    use crate::api::testsupport::{TestApi, base};
    use chrono::Duration;
    use rhizo_domain::threshold::Severity;
    use rhizo_storage::repo::plant as plant_repo;

    /// A monitoring-only plant with an ambient-temperature policy.
    async fn monitoring_only(api: &TestApi) {
        api.with_device().await;
        api.plant("fern-01").await;
        api.json(
            "PUT",
            "/api/v1/plants/fern-01/bindings/sensors",
            serde_json::json!({
                "device_id": "plant-node-01",
                "sensor_id": "soil-0",
                "point": "default",
                "kind": "soil_temperature",
                "role": "advisory",
            }),
        )
        .await;
        api.json(
            "PUT",
            "/api/v1/plants/fern-01/measurement-policies/soil_temperature",
            serde_json::json!({
                "target_min": 16.0, "target_max": 24.0,
                "warning_low": 12.0, "warning_high": 28.0,
                "critical_low": 8.0, "critical_high": 32.0,
                "stale_after_ms": 900_000,
                "hysteresis": 1.0,
            }),
        )
        .await;
    }

    async fn loaded(api: &TestApi) -> crate::plant::Loaded {
        crate::plant::load(&api.db, "fern-01")
            .await
            .unwrap()
            .unwrap()
    }

    /// A critical alert is raised for a plant that cannot be watered at all —
    /// which is the common case, not a degraded one.
    #[tokio::test]
    async fn a_monitoring_only_plant_raises_critical_alerts_normally() {
        let api = TestApi::start().await;
        monitoring_only(&api).await;
        api.sample_kind(base(), "soil_temperature", "celsius", 34.0)
            .await;
        let crossings = super::run(&api.db, &loaded(&api).await, base())
            .await
            .unwrap();
        assert_eq!(crossings.len(), 1);
        assert_eq!(crossings[0].0, "soil_temperature");
        assert_eq!(crossings[0].1.to, Severity::Critical);

        let events = plant_repo::plant_events(&api.db, "fern-01", 50)
            .await
            .unwrap();
        assert!(events.iter().any(|(kind, severity, ..)| {
            kind == "threshold.critical" && severity == "critical"
        }));

        let (_, plant) = api.get("/api/v1/plants/fern-01").await;
        assert_eq!(plant["has_actuator"], false);
        assert_eq!(
            plant["thresholds"]["soil_temperature"], "critical",
            "threshold state is visible per measurement in the plant API"
        );
    }

    /// One event per transition, and hysteresis stops the oscillation that
    /// would otherwise produce one per tick.
    #[tokio::test]
    async fn a_crossing_raises_one_event_and_hysteresis_holds_it() {
        let api = TestApi::start().await;
        monitoring_only(&api).await;
        api.sample_kind(base(), "soil_temperature", "celsius", 28.5)
            .await;
        let first = super::run(&api.db, &loaded(&api).await, base())
            .await
            .unwrap();
        assert_eq!(first[0].1.to, Severity::Warning);

        // Hovering just inside the band does not clear it, and repeat ticks on
        // the same reading raise nothing.
        for (i, value) in [27.5, 27.2, 27.6].into_iter().enumerate() {
            let at = base() + Duration::minutes(i as i64 + 1);
            api.sample_kind(at, "soil_temperature", "celsius", value)
                .await;
            assert!(
                super::run(&api.db, &loaded(&api).await, at)
                    .await
                    .unwrap()
                    .is_empty(),
                "{value} is inside the 1.0 hysteresis band"
            );
        }
        let events = plant_repo::plant_events(&api.db, "fern-01", 50)
            .await
            .unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|(kind, ..)| kind.starts_with("threshold."))
                .count(),
            1
        );

        // A genuine return clears it, once.
        let at = base() + Duration::minutes(10);
        api.sample_kind(at, "soil_temperature", "celsius", 20.0)
            .await;
        let cleared = super::run(&api.db, &loaded(&api).await, at).await.unwrap();
        assert_eq!(cleared[0].1.to, Severity::Normal);
    }

    /// A reading older than the plant's freshness horizon neither raises nor
    /// clears: silence is not evidence.
    #[tokio::test]
    async fn a_stale_reading_neither_raises_nor_clears() {
        let api = TestApi::start().await;
        monitoring_only(&api).await;
        api.sample_kind(base(), "soil_temperature", "celsius", 34.0)
            .await;
        super::run(&api.db, &loaded(&api).await, base())
            .await
            .unwrap();
        let later = base() + Duration::hours(3);
        assert!(
            super::run(&api.db, &loaded(&api).await, later)
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            plant_repo::threshold_state(&api.db, "fern-01", "soil_temperature")
                .await
                .unwrap()
                .unwrap()
                .severity,
            "critical",
            "a critical alert is not cleared by the sensor going quiet"
        );
    }

    /// **No threshold crossing of any kind triggers actuation.** A critical
    /// reading raises an alert and changes no watering decision.
    #[tokio::test]
    async fn threshold_alerts_never_actuate() {
        let api = TestApi::start().await;
        monitoring_only(&api).await;
        api.bind_control("fern-01").await;
        api.moisture_policy("fern-01").await;
        api.json(
            "PUT",
            "/api/v1/plants/fern-01/bindings/actuator",
            serde_json::json!({ "device_id": "plant-node-01", "actuator_id": "pump-0" }),
        )
        .await;
        for i in 0i64..72 {
            api.sample(base() - Duration::minutes((71 - i) * 5), 34.0)
                .await;
        }
        let before = api.evaluate("fern-01").await;

        // Now drive the temperature critical and re-evaluate.
        api.sample_kind(base(), "soil_temperature", "celsius", 34.0)
            .await;
        let after = api.evaluate("fern-01").await;
        assert_eq!(
            before.decision, after.decision,
            "a critical temperature changed the watering answer"
        );
        assert_eq!(before.blocked_by, after.blocked_by);
        assert_eq!(
            plant_repo::watering_events(&api.db, "fern-01", None, None, 10)
                .await
                .unwrap()
                .len(),
            0,
            "and watered nothing"
        );
    }
}
