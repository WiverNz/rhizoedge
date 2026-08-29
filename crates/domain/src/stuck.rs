//! Stuck-sensor detection (PRD 050 F-050-17).
//!
//! A sensor returning a constant value passes every range check while telling
//! you nothing, which makes it one of the failure modes SAFETY-005 has to catch.
//!
//! # Bit-identical, deliberately
//!
//! Comparison is bit-for-bit, not within a tolerance. Real sensors have noise,
//! so genuinely identical consecutive readings are strong evidence of a fault;
//! a tolerance-based comparison would fire on a genuinely stable environment,
//! which is the false positive that teaches operators to ignore the alert.
//!
//! [`DEFAULT_STUCK_SAMPLE_COUNT`] readings at a 300-second cadence is over an
//! hour and a half: long enough to avoid false positives, short enough to matter.
use rhizo_mqtt_contract::payload::MeasurementValue;

/// Consecutive identical readings that mark a sensor unhealthy.
pub const DEFAULT_STUCK_SAMPLE_COUNT: u32 = 20;

/// The comparable form of a reading.
///
/// Scalars are compared by their bit pattern rather than by `==`, so the check
/// means "the sensor produced the same number" rather than "the numbers are
/// close". It also gives `NaN` a stable identity, which `==` does not.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RawReading {
    /// A scalar, as its IEEE-754 bit pattern.
    Scalar(u64),
    /// A boolean.
    Boolean(bool),
}

impl RawReading {
    /// The comparable form of a measurement value.
    #[must_use]
    pub fn of(value: MeasurementValue) -> Self {
        match value {
            MeasurementValue::Scalar(v) => Self::Scalar(v.to_bits()),
            MeasurementValue::Boolean(v) => Self::Boolean(v),
        }
    }
}

/// Per-sensor run-length state, persisted so a restart does not lose a run.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StuckState {
    /// The reading the current run is made of.
    pub last: Option<RawReading>,
    /// How many consecutive readings the run holds, including the first.
    pub repeats: u32,
    /// Whether the current run has already raised its event.
    pub reported: bool,
}

/// What one reading did.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StuckOutcome {
    /// The run continues, below the threshold.
    Running,
    /// The run reached the threshold on this reading. Raise `sensor_stuck`
    /// **once** — subsequent identical readings answer [`Self::AlreadyStuck`].
    BecameStuck,
    /// The run is past the threshold and the event has already been raised.
    AlreadyStuck,
    /// A different value ended the run.
    Reset,
    /// A failed read. It ends no run and starts none: an absent reading is not
    /// evidence that the sensor is alive.
    Ignored,
}

impl StuckState {
    /// Folds one reading in.
    ///
    /// `value` is `None` for a failed read.
    pub fn observe(&mut self, value: Option<MeasurementValue>, threshold: u32) -> StuckOutcome {
        let Some(reading) = value.map(RawReading::of) else {
            return StuckOutcome::Ignored;
        };
        if self.last != Some(reading) {
            self.last = Some(reading);
            self.repeats = 1;
            self.reported = false;
            return StuckOutcome::Reset;
        }
        self.repeats = self.repeats.saturating_add(1);
        if self.repeats < threshold {
            return StuckOutcome::Running;
        }
        if self.reported {
            return StuckOutcome::AlreadyStuck;
        }
        self.reported = true;
        StuckOutcome::BecameStuck
    }

    /// Whether the sensor is currently considered unhealthy.
    #[must_use]
    pub const fn is_stuck(&self, threshold: u32) -> bool {
        self.repeats >= threshold
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scalar(v: f64) -> Option<MeasurementValue> {
        Some(MeasurementValue::Scalar(v))
    }

    /// SCEN-024. Nineteen identical readings are a stable sensor; twenty are a
    /// stuck one, and the event fires exactly once.
    #[test]
    fn twenty_identical_readings_mark_the_sensor_and_raise_one_event() {
        let mut state = StuckState::default();
        let mut outcomes = Vec::new();
        for _ in 0..19 {
            outcomes.push(state.observe(scalar(31.25), DEFAULT_STUCK_SAMPLE_COUNT));
        }
        assert_eq!(outcomes[0], StuckOutcome::Reset, "the first starts a run");
        assert!(outcomes[1..].iter().all(|o| *o == StuckOutcome::Running));
        assert!(
            !state.is_stuck(DEFAULT_STUCK_SAMPLE_COUNT),
            "nineteen is not twenty"
        );

        assert_eq!(
            state.observe(scalar(31.25), DEFAULT_STUCK_SAMPLE_COUNT),
            StuckOutcome::BecameStuck
        );
        assert!(state.is_stuck(DEFAULT_STUCK_SAMPLE_COUNT));
        for _ in 0..5 {
            assert_eq!(
                state.observe(scalar(31.25), DEFAULT_STUCK_SAMPLE_COUNT),
                StuckOutcome::AlreadyStuck,
                "the event is raised once, not once per sample"
            );
        }
    }

    #[test]
    fn a_different_value_resets_the_counter() {
        let mut state = StuckState::default();
        for _ in 0..25 {
            state.observe(scalar(31.25), DEFAULT_STUCK_SAMPLE_COUNT);
        }
        assert!(state.is_stuck(DEFAULT_STUCK_SAMPLE_COUNT));
        assert_eq!(
            state.observe(scalar(31.26), DEFAULT_STUCK_SAMPLE_COUNT),
            StuckOutcome::Reset
        );
        assert_eq!(state.repeats, 1);
        assert!(!state.reported);
        assert!(!state.is_stuck(DEFAULT_STUCK_SAMPLE_COUNT));
    }

    /// The reason the comparison is bit-identical: a noisy but stable sensor
    /// varies in the last decimal, and must not be reported as faulty.
    #[test]
    fn noisy_but_stable_readings_never_trigger_it() {
        let mut state = StuckState::default();
        for i in 0..200 {
            let jitter = f64::from(i % 7) * 0.001;
            state.observe(scalar(31.25 + jitter), DEFAULT_STUCK_SAMPLE_COUNT);
            assert!(!state.is_stuck(DEFAULT_STUCK_SAMPLE_COUNT));
        }
    }

    #[test]
    fn a_failed_read_neither_extends_nor_breaks_a_run() {
        let mut state = StuckState::default();
        for _ in 0..5 {
            state.observe(scalar(31.25), DEFAULT_STUCK_SAMPLE_COUNT);
        }
        let before = state;
        assert_eq!(
            state.observe(None, DEFAULT_STUCK_SAMPLE_COUNT),
            StuckOutcome::Ignored
        );
        assert_eq!(state, before);
    }

    #[test]
    fn booleans_and_nan_have_a_stable_identity() {
        let mut state = StuckState::default();
        for _ in 0..DEFAULT_STUCK_SAMPLE_COUNT {
            state.observe(
                Some(MeasurementValue::Boolean(false)),
                DEFAULT_STUCK_SAMPLE_COUNT,
            );
        }
        assert!(state.is_stuck(DEFAULT_STUCK_SAMPLE_COUNT));
        assert_eq!(
            state.observe(
                Some(MeasurementValue::Boolean(true)),
                DEFAULT_STUCK_SAMPLE_COUNT
            ),
            StuckOutcome::Reset
        );
        assert_eq!(
            RawReading::of(MeasurementValue::Scalar(f64::NAN)),
            RawReading::of(MeasurementValue::Scalar(f64::NAN)),
            "bit comparison gives NaN the identity `==` refuses it"
        );
    }
}
