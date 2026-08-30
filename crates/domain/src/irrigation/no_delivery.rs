//! No-delivery detection (M6-017, F-060-33).
//!
//! A pump that runs but delivers nothing — an air lock, a kinked tube, a dry
//! line — is the most damaging plausible failure in this class of system,
//! because the naive response is to escalate: the plant still reads dry, so the
//! machine doses again, and again. The reservoir empties onto whatever the tube
//! is actually pointing at.
//!
//! # Both signals must be silent
//!
//! Requiring **moisture and weight** to be unresponsive avoids the obvious false
//! positive: soil near field capacity may show no moisture rise even though
//! water arrived, but the pot got heavier. Where no scale is fitted, moisture
//! alone decides and the answer is necessarily less certain — which is stated
//! rather than hidden.
//!
//! # Unknown evidence stops escalation
//!
//! An absent or unreadable reading counts as **no response**, so uncertainty
//! stops the cycle rather than continuing it. That is the fail-closed direction
//! here: the dangerous outcome is another dose, not a refusal.

/// What is known about the plant's response to the doses just delivered.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DeliveryEvidence {
    /// Moisture immediately before the cycle's first dose.
    pub pre_dose_vwc: Option<f64>,
    /// Moisture now.
    pub latest_vwc: Option<f64>,
    /// Pot mass immediately before the cycle's first dose.
    pub pre_dose_grams: Option<f64>,
    /// Pot mass now.
    pub latest_grams: Option<f64>,
    /// Whether a scale contributes to this plant at all.
    pub has_weight_sensor: bool,
    /// The configured rise that counts as a moisture response.
    pub recovery_delta_vwc: f64,
    /// Doses delivered in the current cycle.
    pub doses_this_cycle: u8,
}

/// The number of consecutive unresponsive doses that stops the cycle.
pub const UNRESPONSIVE_DOSE_LIMIT: u8 = 2;

/// The smallest pot-mass rise, in grams, that counts as water having arrived.
///
/// One millilitre of water weighs one gram, so five grams is well under the
/// smallest dose any profile may configure and comfortably above scale noise.
/// It is a floor on *evidence*, not a dose parameter, which is why it lives here
/// rather than in a profile an operator could lower to defeat the check.
pub const WEIGHT_RESPONSE_GRAMS: f64 = 5.0;

impl DeliveryEvidence {
    /// Whether moisture rose by at least the configured delta.
    ///
    /// An absent or non-finite reading on either side answers `false`: an
    /// unmeasurable rise is not a rise.
    #[must_use]
    pub fn moisture_responded(&self) -> bool {
        match (self.pre_dose_vwc, self.latest_vwc) {
            (Some(before), Some(after))
                if before.is_finite()
                    && after.is_finite()
                    && self.recovery_delta_vwc.is_finite() =>
            {
                after >= before + self.recovery_delta_vwc
            }
            (Some(_) | None, Some(_) | None) => false,
        }
    }

    /// Whether the pot got measurably heavier.
    ///
    /// Answers `false` when the plant has no scale, so a plant that never had
    /// one is judged on moisture alone rather than being permanently "not
    /// responding by weight".
    #[must_use]
    pub fn weight_responded(&self) -> bool {
        if !self.has_weight_sensor {
            return false;
        }
        match (self.pre_dose_grams, self.latest_grams) {
            (Some(before), Some(after)) if before.is_finite() && after.is_finite() => {
                after >= before + WEIGHT_RESPONSE_GRAMS
            }
            (Some(_) | None, Some(_) | None) => false,
        }
    }
}

/// Whether the cycle must stop because the doses are reaching nothing.
///
/// `true` only when both signals are silent **and** at least
/// [`UNRESPONSIVE_DOSE_LIMIT`] doses have been delivered. One unresponsive dose
/// is normal: absorption takes time, which is what the absorption wait is for.
#[must_use]
pub fn no_delivery_detected(evidence: &DeliveryEvidence) -> bool {
    evidence.doses_this_cycle >= UNRESPONSIVE_DOSE_LIMIT
        && !evidence.moisture_responded()
        && !evidence.weight_responded()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
#[allow(
    clippy::module_inception,
    reason = "the module name is the verification filter the issue quotes literally"
)]
mod no_delivery {
    use super::*;

    fn evidence() -> DeliveryEvidence {
        DeliveryEvidence {
            pre_dose_vwc: Some(20.0),
            latest_vwc: Some(20.1),
            pre_dose_grams: Some(1_800.0),
            latest_grams: Some(1_800.2),
            has_weight_sensor: true,
            recovery_delta_vwc: 6.0,
            doses_this_cycle: 2,
        }
    }

    #[test]
    fn two_unresponsive_doses_stop_the_cycle() {
        assert!(no_delivery_detected(&evidence()));
    }

    #[test]
    fn one_unresponsive_dose_does_not() {
        let mut e = evidence();
        e.doses_this_cycle = 1;
        assert!(!no_delivery_detected(&e));
        e.doses_this_cycle = 0;
        assert!(!no_delivery_detected(&e));
    }

    /// The false positive this design exists to avoid: near field capacity the
    /// soil may not read wetter, but the pot is heavier and water plainly
    /// arrived.
    #[test]
    fn a_weight_rise_without_a_moisture_rise_does_not_trigger_it() {
        let mut e = evidence();
        e.latest_grams = Some(1_840.0);
        assert!(e.weight_responded());
        assert!(!e.moisture_responded());
        assert!(!no_delivery_detected(&e));
    }

    #[test]
    fn a_moisture_rise_alone_is_enough_of_a_response() {
        let mut e = evidence();
        e.latest_vwc = Some(26.0);
        assert!(e.moisture_responded());
        assert!(!no_delivery_detected(&e));
    }

    /// A plant with no scale is judged on moisture alone rather than being
    /// permanently unresponsive by weight.
    #[test]
    fn a_plant_with_no_scale_is_judged_on_moisture_alone() {
        let mut e = evidence();
        e.has_weight_sensor = false;
        e.pre_dose_grams = None;
        e.latest_grams = None;
        assert!(no_delivery_detected(&e));
        e.latest_vwc = Some(26.0);
        assert!(!no_delivery_detected(&e));
    }

    /// Unknown evidence stops escalation rather than licensing it.
    #[test]
    fn unreadable_evidence_counts_as_no_response() {
        for (pre, latest) in [
            (None, Some(30.0)),
            (Some(20.0), None),
            (Some(f64::NAN), Some(30.0)),
            (Some(20.0), Some(f64::INFINITY)),
            (None, None),
        ] {
            let mut e = evidence();
            e.pre_dose_vwc = pre;
            e.latest_vwc = latest;
            e.pre_dose_grams = None;
            e.latest_grams = None;
            assert!(
                no_delivery_detected(&e),
                "pre {pre:?} latest {latest:?} must stop the cycle"
            );
        }
    }

    /// The weight threshold is a floor on evidence, not a dose parameter.
    #[test]
    fn the_weight_threshold_is_a_fixed_evidence_floor() {
        let mut e = evidence();
        e.latest_grams = Some(1_800.0 + WEIGHT_RESPONSE_GRAMS - 0.01);
        assert!(!e.weight_responded());
        e.latest_grams = Some(1_800.0 + WEIGHT_RESPONSE_GRAMS);
        assert!(e.weight_responded());
    }
}
