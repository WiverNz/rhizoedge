//! Manual-watering and stuck-sensor detection on the tick (M5-007, M5-008).
//!
//! Both are pure rules in `rhizo-domain`; this module supplies their inputs from
//! storage and records their conclusions. Neither produces a command.
use chrono::{DateTime, Duration, Utc};
use rhizo_domain::detect::{DetectConfig, DetectSample, DetectionSource, detect_manual_watering};
use rhizo_domain::plant::BindingRole;
use rhizo_domain::stuck::{DEFAULT_STUCK_SAMPLE_COUNT, RawReading, StuckOutcome, StuckState};
use rhizo_mqtt_contract::payload::{MeasurementKind, MeasurementValue};
use rhizo_storage::EdgeDb;
use rhizo_storage::repo::{plant as plant_repo, query};

use super::Loaded;

/// What one detection pass found.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DetectionOutcome {
    /// A `detected` watering event was recorded.
    pub recorded: bool,
    /// A rise was seen but attributed to a completed command.
    pub attributed_to_command: bool,
}

/// Looks for water the system did not deliver.
///
/// Compares the two most recent samples of the control kind, and of pot weight
/// where a scale is bound. A rise inside the absorption window of a completed
/// command is that command's and produces nothing (F-050-16) — without which
/// every automatic dose would also register as a manual one and corrupt both the
/// cooldown and the rolling daily total SAFETY-006 depends on.
pub async fn run(
    db: &EdgeDb,
    loaded: &Loaded,
    now: DateTime<Utc>,
) -> Result<DetectionOutcome, rhizo_storage::StorageError> {
    let plant_id = loaded.plant.plant_id.as_str();
    let Some(control) = loaded.control() else {
        return Ok(DetectionOutcome::default());
    };
    let device = control.binding.device_id.to_string();
    let point = control.binding.point.as_str();
    let moisture =
        query::recent_measurements(db, &device, point, control.binding.kind.as_str(), 2).await?;
    if moisture.len() < 2 {
        return Ok(DetectionOutcome::default());
    }

    // A pot scale, where one is bound, gives the better volume estimate.
    let weight = match loaded.binding_for(&MeasurementKind::PotWeight) {
        Some(bound) => {
            query::recent_measurements(
                db,
                bound.binding.device_id.as_ref(),
                bound.binding.point.as_str(),
                "pot_weight",
                2,
            )
            .await?
        }
        None => Vec::new(),
    };
    let weight_at = |index: usize| weight.get(index).and_then(|r| r.value_num);

    let sample = |index: usize| DetectSample {
        moisture_vwc: moisture[index].value_num.filter(|v| v.is_finite()),
        weight_g: if weight.len() == 2 {
            weight_at(index)
        } else {
            None
        },
        at: DateTime::from_timestamp_millis(moisture[index].received_at).unwrap_or(now),
    };

    let config = DetectConfig::new(Duration::minutes(i64::from(
        loaded.profile.absorption_minutes,
    )));
    let last_command = plant_repo::last_command_completed_at(db, plant_id)
        .await?
        .and_then(DateTime::from_timestamp_millis);
    let previous = sample(0);
    let current = sample(1);

    let Some(detected) = detect_manual_watering(&previous, &current, &config, last_command) else {
        // Distinguish "nothing happened" from "a command explains it", so the
        // attribution rule is observable rather than merely silent.
        let attributed = detect_manual_watering(&previous, &current, &config, None).is_some();
        return Ok(DetectionOutcome {
            recorded: false,
            attributed_to_command: attributed,
        });
    };

    let detail = serde_json::json!({
        "source": match detected.source {
            DetectionSource::Moisture => "moisture",
            DetectionSource::Weight => "weight",
        },
        "moisture_rise_pp": detected.moisture_rise_pp,
        "weight_rise_g": detected.weight_rise_g,
    });
    let recorded = plant_repo::insert_detected_watering(
        db,
        plant_id,
        Some(&device),
        detected.at.timestamp_millis(),
        detected.estimated_ml,
        &detail,
    )
    .await?;
    if recorded {
        plant_repo::record_plant_event(
            db,
            Some(plant_id),
            &format!(
                "plant:{plant_id}:manual_watering_detected:{}",
                detected.at.timestamp_millis()
            ),
            "manual_watering_detected",
            "info",
            Some(&detail),
            now.timestamp_millis(),
        )
        .await?;
    }
    Ok(DetectionOutcome {
        recorded,
        attributed_to_command: false,
    })
}

/// Advances stuck-sensor detection for every bound stream of a plant.
///
/// Returns the streams that became stuck on this pass, so the caller can raise
/// one event each. A stream already reported stays quiet.
pub async fn stuck(
    db: &EdgeDb,
    loaded: &Loaded,
    now: DateTime<Utc>,
) -> Result<Vec<String>, rhizo_storage::StorageError> {
    let mut became_stuck = Vec::new();
    let now_ms = now.timestamp_millis();
    for bound in &loaded.sensors {
        if bound.binding.role == BindingRole::Advisory && !bound.binding.kind.is_known() {
            continue;
        }
        let device = bound.binding.device_id.to_string();
        let sensor = bound.binding.sensor_id.as_str();
        let point = bound.binding.point.as_str();
        let kind = bound.binding.kind.as_str();
        let Some(latest) = query::latest_measurement(db, &device, sensor, point, kind).await?
        else {
            continue;
        };
        let stored = plant_repo::stuck_state(db, &device, sensor, point, kind).await?;
        // Fold a reading in once. The tick reads the *latest* row rather than a
        // stream of new ones, so without this guard the same reading would
        // extend the run on every tick and any sensor at all would look stuck.
        if stored.last_received_at == Some(latest.received_at) {
            continue;
        }
        let mut state = StuckState {
            last: stored
                .last_bool
                .map(RawReading::Boolean)
                .or_else(|| stored.last_bits.map(|bits| RawReading::Scalar(bits as u64))),
            repeats: u32::try_from(stored.repeats).unwrap_or(u32::MAX),
            reported: stored.reported,
        };
        let value = match (latest.value_num, latest.value_bool) {
            (Some(v), _) if v.is_finite() => Some(MeasurementValue::Scalar(v)),
            (_, Some(b)) => Some(MeasurementValue::Boolean(b != 0)),
            _ => None,
        };
        let outcome = state.observe(value, DEFAULT_STUCK_SAMPLE_COUNT);
        let (bits, boolean) = match state.last {
            Some(RawReading::Scalar(bits)) => (Some(bits as i64), None),
            Some(RawReading::Boolean(b)) => (None, Some(b)),
            None => (None, None),
        };
        plant_repo::put_stuck_state(
            db,
            &device,
            sensor,
            point,
            kind,
            plant_repo::StuckStateRow {
                last_bits: bits,
                last_bool: boolean,
                last_received_at: Some(latest.received_at),
                repeats: i64::from(state.repeats),
                reported: state.reported,
            },
            now_ms,
        )
        .await?;
        if outcome == StuckOutcome::BecameStuck {
            let detail = serde_json::json!({
                "device_id": device,
                "point": point,
                "kind": kind,
                "consecutive_identical_readings": state.repeats,
            });
            plant_repo::record_plant_event(
                db,
                Some(loaded.plant.plant_id.as_str()),
                &format!("plant:{device}:{point}:{kind}:sensor_stuck:{now_ms}"),
                "sensor_stuck",
                "warning",
                Some(&detail),
                now_ms,
            )
            .await?;
            became_stuck.push(format!("{device}/{point}/{kind}"));
        }
    }
    Ok(became_stuck)
}

#[cfg(test)]
mod tests {
    use crate::api::testsupport::{TestApi, base};
    use chrono::Duration;
    use rhizo_storage::repo::plant as plant_repo;

    async fn plant(api: &TestApi) {
        api.with_device().await;
        api.plant("monstera-01").await;
        api.bind_control("monstera-01").await;
        api.moisture_policy("monstera-01").await;
    }

    /// A moisture step with no command behind it is a person with a jug.
    #[tokio::test]
    async fn a_moisture_step_creates_a_detected_event_and_resets_the_cooldown() {
        let api = TestApi::start().await;
        plant(&api).await;
        api.sample(base() - Duration::minutes(5), 24.0).await;
        api.sample(base(), 44.0).await;

        assert_eq!(
            plant_repo::last_watering_at(&api.db, "monstera-01")
                .await
                .unwrap(),
            None
        );
        let outcome = super::run(
            &api.db,
            &crate::plant::load(&api.db, "monstera-01")
                .await
                .unwrap()
                .unwrap(),
            base(),
        )
        .await
        .unwrap();
        assert!(outcome.recorded);

        let events = plant_repo::watering_events(&api.db, "monstera-01", None, None, 10)
            .await
            .unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].mode, "detected");
        assert_eq!(
            events[0].command_id, None,
            "a detected watering has no command behind it"
        );
        assert_eq!(
            plant_repo::last_watering_at(&api.db, "monstera-01")
                .await
                .unwrap(),
            Some(base().timestamp_millis()),
            "detection resets time-since-last-watering"
        );
        assert_eq!(
            plant_repo::delivered_since(&api.db, "monstera-01", 0)
                .await
                .unwrap(),
            0.0,
            "and is excluded from the automatic daily total"
        );

        // Re-running the same pass records nothing further.
        let again = super::run(
            &api.db,
            &crate::plant::load(&api.db, "monstera-01")
                .await
                .unwrap()
                .unwrap(),
            base(),
        )
        .await
        .unwrap();
        assert!(!again.recorded);
        assert_eq!(
            plant_repo::watering_events(&api.db, "monstera-01", None, None, 10)
                .await
                .unwrap()
                .len(),
            1
        );
    }

    /// F-050-16: a rise following a completed command creates **no** second
    /// event. Without this the plant appears to have received twice what it did.
    #[tokio::test]
    async fn a_step_following_a_completed_command_creates_no_event() {
        let api = TestApi::start().await;
        plant(&api).await;
        // A completed command two minutes before the rise.
        sqlx::query(
            "INSERT INTO commands(command_id,device_id,plant_id,kind,requested_ml,mode,issued_at,expires_at,status) \
             VALUES('cmd-1','plant-node-01','monstera-01','water',40.0,'manual',?,?,'completed')",
        )
        .bind(base().timestamp_millis() - 300_000)
        .bind(base().timestamp_millis())
        .execute(api.db.pool())
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO watering_events(watering_event_id,plant_id,device_id,command_id,mode,origin,started_at,completed_at,requested_ml,delivered_ml,status) \
             VALUES('we-1','monstera-01','plant-node-01','cmd-1','manual','edge_command',?,?,40.0,40.0,'completed')",
        )
        .bind(base().timestamp_millis() - 300_000)
        .bind(base().timestamp_millis() - 120_000)
        .execute(api.db.pool())
        .await
        .unwrap();

        api.sample(base() - Duration::minutes(5), 24.0).await;
        api.sample(base(), 44.0).await;
        let outcome = super::run(
            &api.db,
            &crate::plant::load(&api.db, "monstera-01")
                .await
                .unwrap()
                .unwrap(),
            base(),
        )
        .await
        .unwrap();
        assert!(!outcome.recorded);
        assert!(
            outcome.attributed_to_command,
            "the rise was seen, and attributed rather than ignored"
        );
        assert_eq!(
            plant_repo::watering_events(&api.db, "monstera-01", None, None, 10)
                .await
                .unwrap()
                .len(),
            1,
            "the command's own event, and no second one"
        );
    }

    #[tokio::test]
    async fn a_sub_threshold_change_creates_nothing() {
        let api = TestApi::start().await;
        plant(&api).await;
        api.sample(base() - Duration::minutes(5), 24.0).await;
        api.sample(base(), 28.0).await;
        let outcome = super::run(
            &api.db,
            &crate::plant::load(&api.db, "monstera-01")
                .await
                .unwrap()
                .unwrap(),
            base(),
        )
        .await
        .unwrap();
        assert!(!outcome.recorded);
        assert!(!outcome.attributed_to_command);
    }

    /// SCEN-024: twenty bit-identical readings mark the sensor, and the event
    /// fires once.
    #[tokio::test]
    async fn scen_024_twenty_identical_readings_raise_one_sensor_stuck_event() {
        let api = TestApi::start().await;
        plant(&api).await;
        let loaded = crate::plant::load(&api.db, "monstera-01")
            .await
            .unwrap()
            .unwrap();
        for i in 0i64..19 {
            api.sample(base() - Duration::minutes(19 - i) * 5, 31.25)
                .await;
            super::stuck(&api.db, &loaded, base()).await.unwrap();
        }
        let events = plant_repo::plant_events(&api.db, "monstera-01", 100)
            .await
            .unwrap();
        assert!(
            !events.iter().any(|(kind, ..)| kind == "sensor_stuck"),
            "nineteen is not twenty"
        );

        for i in 0i64..5 {
            api.sample(base() + Duration::minutes(i * 5), 31.25).await;
            super::stuck(&api.db, &loaded, base() + Duration::minutes(i * 5))
                .await
                .unwrap();
        }
        let events = plant_repo::plant_events(&api.db, "monstera-01", 100)
            .await
            .unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|(kind, ..)| kind == "sensor_stuck")
                .count(),
            1,
            "one event per run, not one per sample"
        );
    }

    #[tokio::test]
    async fn a_changing_reading_never_looks_stuck() {
        let api = TestApi::start().await;
        plant(&api).await;
        let loaded = crate::plant::load(&api.db, "monstera-01")
            .await
            .unwrap()
            .unwrap();
        for i in 0i64..40 {
            api.sample(
                base() + Duration::minutes(i * 5),
                31.25 + (i % 7) as f64 * 0.001,
            )
            .await;
            super::stuck(&api.db, &loaded, base() + Duration::minutes(i * 5))
                .await
                .unwrap();
        }
        let events = plant_repo::plant_events(&api.db, "monstera-01", 100)
            .await
            .unwrap();
        assert!(!events.iter().any(|(kind, ..)| kind == "sensor_stuck"));
    }
}
