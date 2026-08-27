//! Sampling and telemetry publication.
//!
//! Protocol §5.2. **One `telemetry.batch` per sampling cycle**, carrying every
//! sample taken in that cycle: one envelope, one deduplication key, and a set of
//! readings a redelivery cannot split apart. Adding a measurement kind costs an
//! enum variant, not a topic ([ADR-017](../../../../docs/adr/017-extensible-measurement-model.md)).
//!
//! # Never retained
//!
//! Telemetry retention is not merely unnecessary, it is dangerous: the broker
//! would serve a stored sample to every new subscriber as though it were
//! current. The retain flag is derived from the topic in
//! [`crate::envelope::Publication`], so there is no parameter here to get wrong.
//!
//! # A bounded ring, not a ledger
//!
//! At most [`TELEMETRY_RING`] batches survive a disconnect. A device is not a
//! store of record for samples, and unbounded buffering would exhaust RAM on
//! real hardware long before the network came back.

use rhizo_mqtt_contract::payload::{
    ActuatorKind, ActuatorState, MeasurementKind, MeasurementSample, MeasurementValue, Quality,
    TelemetryBatch, Unit,
};
use rhizo_mqtt_contract::safety::LeakState;
use uuid::Uuid;

use crate::capabilities::{Capabilities, SensorSpec};
use crate::environment::Environment;

/// How many sampling cycles survive a disconnect. Older ones are dropped.
pub const TELEMETRY_RING: usize = 16;

/// A sampling cycle's readings, waiting for a connection.
#[derive(Clone, Debug, PartialEq)]
pub struct BufferedBatch {
    /// The batch itself, already assigned its `batch_id`.
    pub batch: TelemetryBatch,
}

/// The bounded telemetry ring.
#[derive(Clone, Debug, Default)]
pub struct TelemetryRing {
    batches: std::collections::VecDeque<BufferedBatch>,
    dropped: u64,
}

impl TelemetryRing {
    /// Adds a batch, evicting the oldest if the ring is full.
    pub fn push(&mut self, batch: TelemetryBatch) {
        if self.batches.len() == TELEMETRY_RING {
            self.batches.pop_front();
            self.dropped = self.dropped.saturating_add(1);
        }
        self.batches.push_back(BufferedBatch { batch });
    }

    /// Takes everything buffered, oldest first.
    pub fn drain(&mut self) -> Vec<TelemetryBatch> {
        self.batches.drain(..).map(|b| b.batch).collect()
    }

    /// How many batches are waiting.
    #[must_use]
    pub fn len(&self) -> usize {
        self.batches.len()
    }

    /// Whether anything is waiting.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.batches.is_empty()
    }

    /// How many cycles have been lost to eviction since boot.
    #[must_use]
    pub const fn dropped(&self) -> u64 {
        self.dropped
    }
}

/// Whether a sensor is producing usable readings.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SensorHealth {
    /// Reading normally.
    Ok,
    /// Reading, but the probe is not calibrated for this kind.
    Uncalibrated,
    /// Producing values that cannot be trusted.
    Suspect,
    /// Cannot be read at all.
    Faulted,
}

impl SensorHealth {
    /// The `quality` a sample carries.
    const fn quality(self, calibrated: Option<bool>) -> Quality {
        match self {
            Self::Faulted => Quality::Fault,
            Self::Suspect => Quality::Suspect,
            Self::Uncalibrated => Quality::Uncalibrated,
            Self::Ok => match calibrated {
                Some(false) => Quality::Uncalibrated,
                _ => Quality::Ok,
            },
        }
    }
}

/// Takes one reading of everything the device declares.
///
/// `health` is consulted per sensor so an injected sensor fault produces the
/// normative shape — `value: null` with `quality: "fault"` — rather than a
/// missing sample or, far worse, a repeated last-good value. A repeated stale
/// value would defeat both staleness detection and stuck-sensor detection at
/// the edge (protocol §5.2).
pub fn sample_cycle(
    capabilities: &Capabilities,
    environment: &mut Environment,
    batch_id: Uuid,
    mut health: impl FnMut(&SensorSpec, &MeasurementKind) -> SensorHealth,
    mut override_value: impl FnMut(&MeasurementKind, MeasurementValue) -> Option<MeasurementValue>,
) -> TelemetryBatch {
    let noise = environment.noise;
    let mut samples = Vec::new();
    for spec in capabilities.sensors() {
        for kind in &spec.kinds {
            let state = health(spec, kind);
            // The generator is drawn from whatever the health is, so a fault
            // does not shift the noise stream and change every later reading.
            let reading = read(kind, environment, noise);
            let value = if state == SensorHealth::Faulted {
                None
            } else {
                reading.and_then(|v| override_value(kind, v).or(Some(v)))
            };
            samples.push(MeasurementSample {
                point: spec.point.clone(),
                kind: kind.clone(),
                value,
                unit: unit_for(kind),
                quality: if value.is_none() {
                    Quality::Fault
                } else {
                    state.quality(spec.calibrated)
                },
                sensor_id: Some(spec.sensor_id.clone()),
                calibration_ref: None,
            });
        }
    }
    TelemetryBatch { batch_id, samples }
}

/// Reads one kind from the models.
fn read(
    kind: &MeasurementKind,
    environment: &mut Environment,
    noise: bool,
) -> Option<MeasurementValue> {
    let value = match kind {
        MeasurementKind::SoilMoisture => {
            MeasurementValue::Scalar(environment.soil.sample_vwc(&mut environment.rng, noise))
        }
        MeasurementKind::SoilTemperature => MeasurementValue::Scalar(
            environment
                .soil
                .sample_temperature_c(&mut environment.rng, noise),
        ),
        MeasurementKind::SoilEc => {
            let vwc = environment.soil.true_vwc();
            MeasurementValue::Scalar(
                environment
                    .ec
                    .sample_us_cm(vwc, &mut environment.rng, noise),
            )
        }
        MeasurementKind::PotWeight => {
            MeasurementValue::Scalar(environment.weight.sample_g(&mut environment.rng, noise))
        }
        MeasurementKind::TankLevel => {
            MeasurementValue::Scalar(environment.tank.sample_percent(&mut environment.rng, noise))
        }
        MeasurementKind::LeakState => match environment.tank.leak() {
            LeakState::Clear => MeasurementValue::Boolean(false),
            LeakState::Detected => MeasurementValue::Boolean(true),
            // An unreadable leak sensor publishes a failed read, never a
            // reassuring `false` (SAFETY-012).
            LeakState::Unknown => return None,
        },
        // Kinds this hardware does not have. Reaching here would mean the
        // capability table declared something the sampler cannot produce, which
        // is the divergence `sampled_kinds` exists to prevent.
        MeasurementKind::SoilPh
        | MeasurementKind::AmbientTemperature
        | MeasurementKind::AmbientHumidity
        | MeasurementKind::Illuminance
        | MeasurementKind::NitrateConcentration
        | MeasurementKind::Unknown(_) => return None,
    };
    Some(value)
}

/// The canonical unit for a kind.
///
/// Taken from the contract's compile-time spec rather than restated here: the
/// `unit` field is a **check, not a choice**, and a second table of units would
/// be a way for the check to disagree with itself.
fn unit_for(kind: &MeasurementKind) -> Unit {
    kind.spec().map_or(Unit::Boolean, |spec| spec.unit)
}

/// Builds the actuator state payload.
pub fn actuator_state(
    actuator_id: rhizo_mqtt_contract::payload::SensorId,
    kind: ActuatorKind,
    active: bool,
    last_run_ms: Option<u32>,
    delivered_today_ml: f32,
    faulted: bool,
) -> ActuatorState {
    ActuatorState {
        actuator_id,
        kind,
        active,
        last_run_ms,
        delivered_today_ml,
        faulted,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::cli;

    fn healthy(_: &SensorSpec, _: &MeasurementKind) -> SensorHealth {
        SensorHealth::Ok
    }

    fn no_override(_: &MeasurementKind, _: MeasurementValue) -> Option<MeasurementValue> {
        None
    }

    fn cycle(flags: &[&str]) -> (TelemetryBatch, Capabilities) {
        let cli = cli(flags);
        let capabilities = Capabilities::from_cli(&cli);
        let mut environment = Environment::from_cli(&cli);
        let batch = sample_cycle(
            &capabilities,
            &mut environment,
            Uuid::from_u128(1),
            healthy,
            no_override,
        );
        (batch, capabilities)
    }

    #[test]
    fn a_cycle_produces_one_valid_batch_of_every_declared_kind() {
        let (batch, capabilities) = cycle(&[]);
        assert!(batch.validate().is_ok());
        let kinds: Vec<_> = batch.samples.iter().map(|s| s.kind.clone()).collect();
        assert_eq!(
            kinds,
            capabilities.sampled_kinds(),
            "the batch contains all and only the declared kinds"
        );
        for sample in &batch.samples {
            assert!(
                sample.validate().is_valid(),
                "{sample:?} failed the contract's own validation"
            );
            assert!(sample.sensor_id.is_some(), "samples name their sensor");
        }
    }

    #[test]
    fn disabling_a_group_removes_exactly_its_kinds() {
        let (batch, _) = cycle(&["--sensors", "soil"]);
        let kinds: Vec<_> = batch.samples.iter().map(|s| s.kind.clone()).collect();
        assert_eq!(
            kinds,
            vec![
                MeasurementKind::SoilMoisture,
                MeasurementKind::SoilTemperature,
                MeasurementKind::SoilEc,
            ]
        );
        assert!(
            !kinds.contains(&MeasurementKind::TankLevel),
            "a disabled sensor produces no sample at all, which is how the \
             missing-sensor lockout paths are exercised"
        );
    }

    #[test]
    fn a_device_with_no_sensors_produces_an_empty_batch_which_is_not_publishable() {
        let (batch, _) = cycle(&["--sensors", ""]);
        assert!(batch.samples.is_empty());
        assert!(
            batch.validate().is_err(),
            "an empty batch MUST NOT be published (protocol §5.2)"
        );
    }

    #[test]
    fn an_uncalibrated_probe_publishes_matching_quality() {
        let (batch, _) = cycle(&["--sensors", "soil"]);
        let ec = batch
            .samples
            .iter()
            .find(|s| s.kind == MeasurementKind::SoilEc)
            .unwrap();
        assert_eq!(ec.quality, Quality::Uncalibrated);
        let moisture = batch
            .samples
            .iter()
            .find(|s| s.kind == MeasurementKind::SoilMoisture)
            .unwrap();
        assert_eq!(moisture.quality, Quality::Ok);
    }

    #[test]
    fn a_failed_read_publishes_null_and_fault_never_a_stale_value() {
        let cli = cli(&["--sensors", "soil"]);
        let capabilities = Capabilities::from_cli(&cli);
        let mut environment = Environment::from_cli(&cli);
        let batch = sample_cycle(
            &capabilities,
            &mut environment,
            Uuid::from_u128(2),
            |_, kind| {
                if *kind == MeasurementKind::SoilMoisture {
                    SensorHealth::Faulted
                } else {
                    SensorHealth::Ok
                }
            },
            no_override,
        );
        let moisture = batch
            .samples
            .iter()
            .find(|s| s.kind == MeasurementKind::SoilMoisture)
            .unwrap();
        assert_eq!(moisture.value, None);
        assert_eq!(moisture.quality, Quality::Fault);
        assert!(
            moisture.validate().is_valid(),
            "null with fault quality is the normative shape, not an error"
        );
        assert_eq!(
            batch.samples.len(),
            3,
            "a faulted sensor still occupies its place in the batch"
        );
    }

    #[test]
    fn an_unreadable_leak_sensor_publishes_a_failed_read_not_a_reassuring_false() {
        let cli = cli(&["--sensors", "leak"]);
        let capabilities = Capabilities::from_cli(&cli);
        let mut environment = Environment::from_cli(&cli);
        environment.tank.set_leak(LeakState::Unknown);
        let batch = sample_cycle(
            &capabilities,
            &mut environment,
            Uuid::from_u128(3),
            healthy,
            no_override,
        );
        assert_eq!(batch.samples[0].value, None);
        assert_eq!(batch.samples[0].quality, Quality::Fault);
    }

    #[test]
    fn a_detected_leak_publishes_true() {
        let cli = cli(&["--sensors", "leak"]);
        let capabilities = Capabilities::from_cli(&cli);
        let mut environment = Environment::from_cli(&cli);
        environment.tank.set_leak(LeakState::Detected);
        let batch = sample_cycle(
            &capabilities,
            &mut environment,
            Uuid::from_u128(4),
            healthy,
            no_override,
        );
        assert_eq!(
            batch.samples[0].value,
            Some(MeasurementValue::Boolean(true))
        );
        assert_eq!(batch.samples[0].unit, Unit::Boolean);
    }

    #[test]
    fn every_sample_carries_the_kinds_canonical_unit() {
        let (batch, _) = cycle(&[]);
        for sample in &batch.samples {
            let expected = sample.kind.spec().unwrap().unit;
            assert_eq!(
                sample.unit, expected,
                "the unit is a check, not a choice: {:?}",
                sample.kind
            );
        }
    }

    #[test]
    fn an_overridden_value_replaces_the_reading_for_that_kind_only() {
        let cli = cli(&["--sensors", "soil"]);
        let capabilities = Capabilities::from_cli(&cli);
        let mut environment = Environment::from_cli(&cli);
        let batch = sample_cycle(
            &capabilities,
            &mut environment,
            Uuid::from_u128(5),
            healthy,
            |kind, _| {
                (*kind == MeasurementKind::SoilMoisture).then_some(MeasurementValue::Scalar(99.0))
            },
        );
        assert_eq!(
            batch.samples[0].value,
            Some(MeasurementValue::Scalar(99.0)),
            "the stuck-sensor and invalid-soil faults work through this hook"
        );
        assert_ne!(batch.samples[1].value, Some(MeasurementValue::Scalar(99.0)));
    }

    // ------------------------------------------------------------ the ring

    fn batch(n: u128) -> TelemetryBatch {
        TelemetryBatch {
            batch_id: Uuid::from_u128(n),
            samples: Vec::new(),
        }
    }

    #[test]
    fn the_ring_keeps_the_newest_sixteen_cycles_and_no_more() {
        let mut ring = TelemetryRing::default();
        assert!(ring.is_empty());
        for n in 0..40 {
            ring.push(batch(n));
        }
        assert_eq!(ring.len(), TELEMETRY_RING);
        assert_eq!(ring.dropped(), 24);
        let drained = ring.drain();
        assert_eq!(drained.len(), TELEMETRY_RING);
        assert_eq!(
            drained.first().unwrap().batch_id,
            Uuid::from_u128(24),
            "the oldest survivors are the newest cycles, in order"
        );
        assert_eq!(drained.last().unwrap().batch_id, Uuid::from_u128(39));
        assert!(ring.is_empty(), "draining empties it");
    }

    #[test]
    fn a_short_disconnect_loses_nothing() {
        let mut ring = TelemetryRing::default();
        for n in 0..TELEMETRY_RING as u128 {
            ring.push(batch(n));
        }
        assert_eq!(ring.dropped(), 0);
        assert_eq!(ring.drain().len(), TELEMETRY_RING);
    }
}
