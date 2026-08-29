//! Detecting water the system did not deliver (PRD 050 F-050-14…F-050-16).
//!
//! # Attribution is the safety-load-bearing part
//!
//! Every automatic dose also raises the moisture. Without attribution each one
//! would *also* register as a manual watering, and the plant would appear to
//! have received twice what it did — corrupting both the cooldown and the
//! rolling 24-hour total SAFETY-006 depends on. A rise inside the absorption
//! window of a completed command is therefore that command's rise, not a
//! detection.
//!
//! # What a detection means afterwards
//!
//! A `detected` row is **excluded from the automatic daily cap** — it was not
//! automatic — and **does reset the cooldown** — a human watered the plant, so
//! the machine should wait. Those two facts live in the queries that consume the
//! ledger; this module only decides that the event happened.
use chrono::{DateTime, Duration, Utc};

/// Default moisture step that counts as a watering, in percentage points.
pub const DEFAULT_MOISTURE_DELTA_PP: f64 = 8.0;
/// Default pot-weight step that counts as a watering, in grams.
pub const DEFAULT_WEIGHT_DELTA_G: f64 = 100.0;

/// Detection thresholds and the attribution window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DetectConfig {
    /// Moisture rise between consecutive samples, percentage points.
    pub moisture_delta_pp: f64,
    /// Pot-weight rise between consecutive samples, grams.
    pub weight_delta_g: f64,
    /// How long after a completed command a rise is still that command's.
    pub absorption: Duration,
}

impl DetectConfig {
    /// The documented starting values (PRD 050 §Open questions 2).
    #[must_use]
    pub fn new(absorption: Duration) -> Self {
        Self {
            moisture_delta_pp: DEFAULT_MOISTURE_DELTA_PP,
            weight_delta_g: DEFAULT_WEIGHT_DELTA_G,
            absorption,
        }
    }
}

/// One consecutive observation pair member.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DetectSample {
    /// Validated moisture, or `None` when absent or invalid.
    pub moisture_vwc: Option<f64>,
    /// Validated pot weight in grams, or `None` when there is no scale.
    pub weight_g: Option<f64>,
    /// **Edge** receipt time.
    pub at: DateTime<Utc>,
}

/// Which signal produced the detection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DetectionSource {
    /// Moisture step only. No volume estimate is possible.
    Moisture,
    /// Pot-weight step, which yields a volume estimate.
    Weight,
}

/// A watering the system did not perform.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DetectedWatering {
    /// Which signal fired.
    pub source: DetectionSource,
    /// Estimated delivered volume, where a scale made one possible.
    ///
    /// `None` for a moisture-only detection: converting a moisture step to
    /// millilitres needs a pot volume and a soil model, and a number invented
    /// from neither would be a claim the system cannot support.
    pub estimated_ml: Option<f64>,
    /// The moisture rise, when there was one.
    pub moisture_rise_pp: Option<f64>,
    /// The weight rise, when there was one.
    pub weight_rise_g: Option<f64>,
    /// Edge receipt time of the sample that revealed it.
    pub at: DateTime<Utc>,
}

/// Detects a manual watering between two consecutive samples.
///
/// `last_command_completed_at` is the completion instant of the most recent
/// command for this plant, if any. A rise observed at or after that instant and
/// within `config.absorption` of it is attributed to the command and produces
/// **no** detection.
///
/// Pure: it reads no clock and touches no storage.
#[must_use]
pub fn detect_manual_watering(
    previous: &DetectSample,
    current: &DetectSample,
    config: &DetectConfig,
    last_command_completed_at: Option<DateTime<Utc>>,
) -> Option<DetectedWatering> {
    if current.at <= previous.at {
        return None;
    }
    let moisture_rise = match (previous.moisture_vwc, current.moisture_vwc) {
        (Some(before), Some(after)) if before.is_finite() && after.is_finite() => {
            Some(after - before)
        }
        _ => None,
    };
    let weight_rise = match (previous.weight_g, current.weight_g) {
        (Some(before), Some(after)) if before.is_finite() && after.is_finite() => {
            Some(after - before)
        }
        _ => None,
    };
    let by_weight = weight_rise.is_some_and(|d| d >= config.weight_delta_g);
    let by_moisture = moisture_rise.is_some_and(|d| d >= config.moisture_delta_pp);
    if !by_weight && !by_moisture {
        return None;
    }
    // Attribution (F-050-16). The window is closed at both ends: a rise before
    // the command completed cannot be its effect, and one long after it is a
    // separate event.
    if let Some(completed) = last_command_completed_at
        && current.at >= completed
        && current.at.signed_duration_since(completed) <= config.absorption
    {
        return None;
    }
    // Weight is the better estimate wherever a scale exists: one gram of water
    // is one millilitre, and nothing has to be assumed about the soil.
    let (source, estimated_ml) = if by_weight {
        (DetectionSource::Weight, weight_rise)
    } else {
        (DetectionSource::Moisture, None)
    };
    Some(DetectedWatering {
        source,
        estimated_ml,
        moisture_rise_pp: moisture_rise,
        weight_rise_g: weight_rise,
        at: current.at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn base() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 1, 1, 12, 0, 0).unwrap()
    }
    fn config() -> DetectConfig {
        DetectConfig::new(Duration::minutes(30))
    }
    fn pair(before: f64, after: f64) -> (DetectSample, DetectSample) {
        (
            DetectSample {
                moisture_vwc: Some(before),
                weight_g: None,
                at: base(),
            },
            DetectSample {
                moisture_vwc: Some(after),
                weight_g: None,
                at: base() + Duration::minutes(5),
            },
        )
    }

    #[test]
    fn a_moisture_step_above_the_threshold_is_a_detection() {
        let (a, b) = pair(24.0, 44.0);
        let detected = detect_manual_watering(&a, &b, &config(), None).unwrap();
        assert_eq!(detected.source, DetectionSource::Moisture);
        assert_eq!(
            detected.estimated_ml, None,
            "moisture alone estimates no ml"
        );
        assert_eq!(detected.moisture_rise_pp, Some(20.0));
        assert_eq!(detected.at, b.at);
    }

    #[test]
    fn a_weight_step_detects_and_estimates_a_volume() {
        let a = DetectSample {
            moisture_vwc: Some(24.0),
            weight_g: Some(5000.0),
            at: base(),
        };
        let b = DetectSample {
            moisture_vwc: Some(25.0),
            weight_g: Some(5350.0),
            at: base() + Duration::minutes(5),
        };
        let detected = detect_manual_watering(&a, &b, &config(), None).unwrap();
        assert_eq!(detected.source, DetectionSource::Weight);
        assert_eq!(detected.estimated_ml, Some(350.0));
        assert_eq!(
            detected.moisture_rise_pp,
            Some(1.0),
            "a sub-threshold moisture rise is still reported as context"
        );
    }

    /// F-050-16. Without this every automatic dose is also counted as a manual
    /// one, and both the cooldown and SAFETY-006's rolling total are corrupted.
    #[test]
    fn a_rise_following_a_completed_command_creates_no_event() {
        let (a, b) = pair(24.0, 44.0);
        let completed = base() + Duration::minutes(1);
        assert_eq!(
            detect_manual_watering(&a, &b, &config(), Some(completed)),
            None
        );
        // At the far edge of the window it is still the command's rise.
        let edge = b.at - Duration::minutes(30);
        assert_eq!(detect_manual_watering(&a, &b, &config(), Some(edge)), None);
        // One millisecond earlier and the window has closed.
        let outside = edge - Duration::milliseconds(1);
        assert!(detect_manual_watering(&a, &b, &config(), Some(outside)).is_some());
        // A command that completes *after* the rise cannot have caused it.
        let later = b.at + Duration::minutes(1);
        assert!(detect_manual_watering(&a, &b, &config(), Some(later)).is_some());
    }

    #[test]
    fn sub_threshold_changes_and_falls_create_nothing() {
        for (before, after) in [(24.0, 31.9), (24.0, 24.0), (44.0, 24.0)] {
            let (a, b) = pair(before, after);
            assert_eq!(
                detect_manual_watering(&a, &b, &config(), None),
                None,
                "{before} -> {after}"
            );
        }
        // Exactly at the threshold is a detection: the rule is `>=`.
        let (a, b) = pair(24.0, 32.0);
        assert!(detect_manual_watering(&a, &b, &config(), None).is_some());
    }

    #[test]
    fn missing_or_out_of_order_readings_detect_nothing() {
        let a = DetectSample {
            moisture_vwc: None,
            weight_g: None,
            at: base(),
        };
        let b = DetectSample {
            moisture_vwc: Some(44.0),
            weight_g: None,
            at: base() + Duration::minutes(5),
        };
        assert_eq!(detect_manual_watering(&a, &b, &config(), None), None);

        let (a, b) = pair(24.0, 44.0);
        assert_eq!(detect_manual_watering(&b, &a, &config(), None), None);
    }
}
