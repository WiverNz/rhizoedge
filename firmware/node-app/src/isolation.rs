//! The isolated loop's cross-tick bookkeeping (M9-016, ADR-015).
//!
//! # What this exists for
//!
//! [`crate::offline::evaluate_and_act`] decides and acts on **one** instant.
//! Everything that has to be remembered *between* instants lives here, so the
//! ESP32 crate stays what its own module documentation says it is — a layer
//! that moves bytes and time — rather than accumulating safety-relevant state
//! in the loop that is hardest to test.
//!
//! Three things are remembered, and each is load-bearing:
//!
//! * **when the last evaluation happened**, because `evaluate_offline` takes a
//!   monotonic *delta* and never an instant, and computing that delta in the
//!   loop is precisely where a since-boot number gets passed where a credit
//!   belongs;
//! * **whether the edge has been in control since**, because time spent
//!   connected must not be credited to an offline cooldown — the edge was
//!   pacing the plant then, from rows, and crediting it here would shorten the
//!   first offline cooldown by however long the session lasted;
//! * **the refusal last buffered**, because a leak that lasts a week would
//!   otherwise fill the 64-slot audit ring with the same sentence and evict the
//!   record of the dose that matters (SAFETY-020).
//!
//! # A reboot credits zero, and so does a reconnection
//!
//! [`IsolationDriver::step`] credits `now - last_evaluation`, and there is no
//! last evaluation immediately after a boot or after the edge has had control.
//! Zero is the conservative answer in both cases and is the same rule
//! `credit_elapsed` applies to a wake: with no trustworthy evidence that time
//! passed, assume none did (SAFETY-015).

use rhizo_mqtt_contract::payload::{
    MeasurementKind, MeasurementValue, OfflinePolicy, Quality, TelemetryBatch,
};
use rhizo_mqtt_contract::safety::LeakState;
use rhizo_mqtt_contract::{CommandId, EventId, UtcMillis};
use rhizo_policy::{MonotonicMillis, OfflineSample, RefuseReason};

use crate::offline::{AutonomousOutcome, OfflineSeamInputs, OfflineTick, evaluate_and_act};
use crate::persist::PersistedState;
use crate::ports::{NvsStore, Pump};

/// Cross-tick state for autonomous operation while isolated.
#[derive(Clone, Copy, Debug, Default)]
pub struct IsolationDriver {
    last_evaluation_ms: Option<u64>,
    last_refusal: Option<RefuseReason>,
}

impl IsolationDriver {
    /// A driver that has never evaluated.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            last_evaluation_ms: None,
            last_refusal: None,
        }
    }

    /// Hands control back to the edge.
    ///
    /// Called when a session opens. It clears the evaluation instant so the
    /// first evaluation after the *next* isolation credits zero rather than the
    /// whole connected period, and clears the remembered refusal so the first
    /// refusal of a new isolation is always recorded — an operator reading the
    /// replayed history needs to see why *this* isolation refused, not be told
    /// it looks like the last one.
    pub const fn on_edge_control(&mut self) {
        self.last_evaluation_ms = None;
        self.last_refusal = None;
    }

    /// The monotonic time this evaluation may credit.
    ///
    /// Public so the loop can log it, and so the rule is testable without
    /// driving a whole evaluation.
    #[must_use]
    pub fn credit(&self, now_mono: u64) -> MonotonicMillis {
        MonotonicMillis(
            self.last_evaluation_ms
                .map_or(0, |last| now_mono.saturating_sub(last)),
        )
    }

    /// Whether an evaluation is due.
    ///
    /// The cadence is tracked **here** rather than in the loop because the loop
    /// is re-entered once per connection attempt, and a full-jitter backoff
    /// starts at a second or two. A loop that evaluated on entry would evaluate
    /// every couple of seconds while the radio was flapping — hundreds of
    /// pointless decisions, each of them a candidate NVS write, on a part with
    /// a flash-endurance limit. Holding the instant across calls makes the
    /// cadence a property of the plant's sampling interval, which is what it
    /// should be.
    ///
    /// A driver that has never evaluated is due immediately: the first
    /// evaluation of an isolation credits zero, so there is nothing to gain by
    /// waiting and a plant to lose.
    #[must_use]
    pub fn due(&self, now_mono: u64, interval_ms: u64) -> bool {
        self.last_evaluation_ms
            .is_none_or(|last| now_mono.saturating_sub(last) >= interval_ms)
    }

    /// Runs one isolated evaluation and acts on the answer.
    ///
    /// Returns what the evaluation did, so the caller can log it. The caller is
    /// responsible for having established that the device is actually isolated:
    /// a connected device is told what to do, and evaluating here as well would
    /// be a second decision path (ADR-006).
    #[allow(
        clippy::too_many_arguments,
        reason = "each argument is a distinct fact the evaluation needs, and the                   two times have different meanings; bundling them is how a                   since-boot instant gets passed where a credit belongs, which                   is what `OfflineTick` documents at length"
    )]
    pub fn step<N: NvsStore, P: Pump>(
        &mut self,
        state: &mut PersistedState,
        nvs: &mut N,
        pump: &mut P,
        plant_id: &str,
        inputs: &OfflineSeamInputs,
        now_mono: u64,
        device_time_ms: Option<UtcMillis>,
        mint_command_id: impl FnOnce() -> CommandId,
        mint_event_id: impl Fn() -> EventId,
    ) -> AutonomousOutcome {
        let elapsed = self.credit(now_mono);
        self.last_evaluation_ms = Some(now_mono);
        let outcome = evaluate_and_act(
            state,
            nvs,
            pump,
            &OfflineTick {
                plant_id,
                inputs,
                elapsed,
                monotonic_ms: now_mono,
                device_time_ms,
                last_refusal: self.last_refusal,
            },
            mint_command_id,
            mint_event_id,
        );
        self.last_refusal = match &outcome {
            AutonomousOutcome::Refused(reason) => Some(*reason),
            AutonomousOutcome::NoValidPolicy => Some(RefuseReason::NoValidPolicy),
            // Anything that is not a refusal ends the run of refusals, so the
            // next one is recorded even if it repeats the one before it.
            AutonomousOutcome::Waiting
            | AutonomousOutcome::Dosed { .. }
            | AutonomousOutcome::BoundRefused(_)
            | AutonomousOutcome::Failed(_) => None,
        };
        outcome
    }
}

/// The plant an isolated device evaluates.
///
/// The first plant in the activated set. M9 provisions one policy per device
/// and the loop evaluates one plant; a device carrying several is an M10
/// question about *ordering*, not about this function, and returning the first
/// deterministically is better than returning an arbitrary one.
#[must_use]
pub fn plant_to_evaluate(state: &PersistedState) -> Option<&str> {
    Some(
        crate::policy::active(state)?
            .policies
            .first()?
            .plant_id
            .as_str(),
    )
}

/// Marshals a freshly read telemetry batch into the evaluator's inputs.
///
/// **Marshalling only — nothing here classifies.** Every threshold, veto and
/// staleness rule belongs to `rhizo_policy::offline_gate`, which this feeds.
/// The one judgement call is the age it stamps on the samples: a batch this
/// device has just read is zero milliseconds old, and that is a fact about when
/// it was read rather than an opinion about whether it is usable. A sensor that
/// failed to read arrives as `value: None, quality: Fault` and the gate refuses
/// it — which is why a failed read must reach here as a sample rather than be
/// dropped from the batch.
///
/// A kind the batch does not contain is **absent**, and absence refuses. That
/// is SAFETY-017's second half: a plant that declared pot weight `required`
/// does not water while its scale is silent.
#[must_use]
pub fn seam_inputs(
    policy: &OfflinePolicy,
    batch: &TelemetryBatch,
    pump_ml_per_second: f32,
    pump_healthy: Option<bool>,
) -> OfflineSeamInputs {
    let fresh = MonotonicMillis(0);
    let find = |kind: &MeasurementKind, point: Option<&str>| {
        batch
            .samples
            .iter()
            .find(|sample| &sample.kind == kind && point.is_none_or(|p| sample.point.as_str() == p))
    };
    let as_offline = |sample: &rhizo_mqtt_contract::payload::MeasurementSample| OfflineSample {
        kind: sample.kind.clone(),
        value: sample.value,
        quality: sample.quality,
        age: fresh,
    };

    let control = find(
        &policy.control_measurement.kind,
        Some(policy.control_measurement.point.as_str()),
    )
    .map(as_offline);

    let required = policy
        .required_measurements
        .iter()
        .filter_map(|required| find(&required.kind, None).map(as_offline))
        .collect();

    // The two hard vetoes read their own kinds, and each resolves to the
    // *unknown* case when the batch carries nothing usable. `None` is not
    // "clear" and it is not "full" (SAFETY-012).
    let leak = find(&MeasurementKind::LeakState, None).map(|sample| match sample.value {
        Some(MeasurementValue::Boolean(true)) => LeakState::Detected,
        Some(MeasurementValue::Boolean(false)) if sample.quality == Quality::Ok => LeakState::Clear,
        Some(MeasurementValue::Boolean(false) | MeasurementValue::Scalar(_)) | None => {
            LeakState::Unknown
        }
    });

    let tank_percent = find(&MeasurementKind::TankLevel, None)
        .filter(|sample| sample.quality == Quality::Ok)
        .and_then(|sample| match sample.value {
            Some(MeasurementValue::Scalar(percent)) => Some(percent as f32),
            Some(MeasurementValue::Boolean(_)) | None => None,
        });

    OfflineSeamInputs {
        control,
        required,
        leak,
        tank_percent,
        pump_healthy,
        pump_ml_per_second,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fakes::{FakeNvs, FakePump, call_log};
    use rhizo_mqtt_contract::payload::{
        ActuatorKind, ControlMeasurement, MeasurementPoint, MeasurementSample, OfflineActuator,
        OfflineLimits, OfflinePolicySet, OfflineSafety, SensorId, Unit,
    };
    use uuid::Uuid;

    fn policy() -> OfflinePolicy {
        OfflinePolicy {
            plant_id: SensorId::parse("basil").expect("id"),
            policy_version: 1,
            enabled: true,
            actuator: Some(OfflineActuator {
                actuator_id: SensorId::parse("pump-1").expect("id"),
                kind: ActuatorKind::IrrigationPump,
                dose_ml: 40.0,
                max_doses_per_cycle: 2,
                absorption_wait_ms: 900_000,
            }),
            control_measurement: ControlMeasurement {
                kind: MeasurementKind::SoilMoisture,
                point: MeasurementPoint::parse("default").expect("point"),
                trigger_below: 25.0,
                resume_above: 35.0,
                confirm_duration_ms: 600_000,
                max_age_ms: 900_000,
            },
            required_measurements: Vec::new(),
            advisory_measurements: Vec::new(),
            limits: OfflineLimits {
                cooldown_ms: 21_600_000,
                max_volume_per_window_ml: 300.0,
                window_ms: 86_400_000,
            },
            safety: OfflineSafety {
                require_leak_clear: true,
                require_tank_above_percent: 15.0,
                require_pump_healthy: true,
            },
        }
    }

    fn empty_batch() -> TelemetryBatch {
        TelemetryBatch {
            batch_id: Uuid::nil(),
            samples: Vec::new(),
        }
    }

    fn scalar(kind: MeasurementKind, unit: Unit, value: f64) -> MeasurementSample {
        MeasurementSample {
            point: MeasurementPoint::parse("default").expect("point"),
            kind,
            value: Some(MeasurementValue::Scalar(value)),
            unit,
            quality: Quality::Ok,
            sensor_id: None,
            calibration_ref: None,
        }
    }

    fn state_with_policy() -> PersistedState {
        let mut state = PersistedState::default();
        crate::policy::apply(
            &mut state,
            &OfflinePolicySet {
                policies: vec![policy()],
            },
            crate::policy::UpdateStep::Complete,
        );
        state
    }

    /// SAFETY-015. The first evaluation after a boot credits nothing, whatever
    /// the monotonic clock reads.
    #[test]
    fn safety_015_the_first_evaluation_credits_zero() {
        let driver = IsolationDriver::new();
        assert_eq!(driver.credit(9_999_999), MonotonicMillis(0));
    }

    /// And so does the first evaluation after the edge has had control: the
    /// connected period was paced by the edge, from rows.
    #[test]
    fn a_connected_period_is_not_credited_to_an_offline_cooldown() {
        let mut driver = IsolationDriver::new();
        let mut state = state_with_policy();
        let mut nvs = FakeNvs::default();
        let mut pump = FakePump::new(call_log());
        let inputs = seam_inputs(&policy(), &empty_batch(), 2.0, Some(true));

        driver.step(
            &mut state,
            &mut nvs,
            &mut pump,
            "basil",
            &inputs,
            1_000,
            None,
            || CommandId::from_uuid(Uuid::nil()),
            || EventId::from_uuid(Uuid::nil()),
        );
        assert_eq!(driver.credit(61_000), MonotonicMillis(60_000));

        driver.on_edge_control();
        assert_eq!(
            driver.credit(61_000),
            MonotonicMillis(0),
            "an hour of edge control is not an hour of offline cooldown"
        );
    }

    /// Successive evaluations credit the interval between them, not the time
    /// since boot. Handing the evaluator a since-boot instant where it expects a
    /// delta is the mistake this whole type exists to make impossible.
    #[test]
    fn successive_evaluations_credit_the_interval_between_them() {
        let mut driver = IsolationDriver::new();
        let mut state = state_with_policy();
        let mut nvs = FakeNvs::default();
        let mut pump = FakePump::new(call_log());
        let inputs = seam_inputs(&policy(), &empty_batch(), 2.0, Some(true));
        let mut step = |driver: &mut IsolationDriver, now| {
            driver.step(
                driver_state(&mut state),
                &mut nvs,
                &mut pump,
                "basil",
                &inputs,
                now,
                None,
                || CommandId::from_uuid(Uuid::nil()),
                || EventId::from_uuid(Uuid::nil()),
            );
        };
        step(&mut driver, 100_000);
        assert_eq!(driver.credit(400_000), MonotonicMillis(300_000));
    }

    fn driver_state(state: &mut PersistedState) -> &mut PersistedState {
        state
    }

    /// **The gate refuses an empty batch**, which is what an M9 device with no
    /// sensors fitted produces. Fail-closed, and the reason says which input is
    /// missing rather than a generic failure.
    #[test]
    fn safety_012_a_batch_with_no_readings_refuses() {
        let inputs = seam_inputs(&policy(), &empty_batch(), 2.0, Some(true));
        assert!(inputs.control.is_none());
        assert!(inputs.leak.is_none(), "absence is not a clear tray");
        assert!(inputs.tank_percent.is_none(), "absence is not a full tank");
    }

    /// A boolean leak reading maps to the tri-state, and a *faulty* one maps to
    /// `Unknown` rather than to the value it happens to carry.
    #[test]
    fn safety_012_a_faulty_leak_reading_is_unknown_not_clear() {
        let mut sample = MeasurementSample {
            point: MeasurementPoint::parse("default").expect("point"),
            kind: MeasurementKind::LeakState,
            value: Some(MeasurementValue::Boolean(false)),
            unit: Unit::Boolean,
            quality: Quality::Ok,
            sensor_id: None,
            calibration_ref: None,
        };
        let batch = TelemetryBatch {
            batch_id: Uuid::nil(),
            samples: vec![sample.clone()],
        };
        assert_eq!(
            seam_inputs(&policy(), &batch, 2.0, Some(true)).leak,
            Some(LeakState::Clear)
        );

        sample.quality = Quality::Fault;
        let batch = TelemetryBatch {
            batch_id: Uuid::nil(),
            samples: vec![sample.clone()],
        };
        assert_eq!(
            seam_inputs(&policy(), &batch, 2.0, Some(true)).leak,
            Some(LeakState::Unknown),
            "a faulty sensor saying 'dry' is not evidence that the tray is dry"
        );

        sample.value = Some(MeasurementValue::Boolean(true));
        sample.quality = Quality::Ok;
        let batch = TelemetryBatch {
            batch_id: Uuid::nil(),
            samples: vec![sample],
        };
        assert_eq!(
            seam_inputs(&policy(), &batch, 2.0, Some(true)).leak,
            Some(LeakState::Detected)
        );
    }

    /// The control sample is found by kind **and point**: another probe on the
    /// same device reporting the same kind at a different point is not evidence
    /// about this plant.
    #[test]
    fn the_control_sample_is_matched_by_kind_and_point() {
        let mut wrong_point = scalar(MeasurementKind::SoilMoisture, Unit::VwcPercent, 12.0);
        wrong_point.point = MeasurementPoint::parse("deep").expect("point");
        let batch = TelemetryBatch {
            batch_id: Uuid::nil(),
            samples: vec![wrong_point],
        };
        assert!(
            seam_inputs(&policy(), &batch, 2.0, Some(true))
                .control
                .is_none(),
            "a reading from another point must not become this plant's control"
        );

        let batch = TelemetryBatch {
            batch_id: Uuid::nil(),
            samples: vec![scalar(
                MeasurementKind::SoilMoisture,
                Unit::VwcPercent,
                12.0,
            )],
        };
        assert!(
            seam_inputs(&policy(), &batch, 2.0, Some(true))
                .control
                .is_some()
        );
    }

    /// A tank reading the sensor flagged is no reading at all.
    #[test]
    fn safety_012_a_faulty_tank_reading_is_absent_not_low() {
        let mut sample = scalar(MeasurementKind::TankLevel, Unit::Percent, 70.0);
        sample.quality = Quality::Suspect;
        let batch = TelemetryBatch {
            batch_id: Uuid::nil(),
            samples: vec![sample],
        };
        assert!(
            seam_inputs(&policy(), &batch, 2.0, Some(true))
                .tank_percent
                .is_none()
        );
    }

    /// The plant evaluated comes from the activated set, and an unprovisioned
    /// device names none. Absence is not permission (SAFETY-013).
    #[test]
    fn safety_013_an_unprovisioned_device_names_no_plant() {
        assert_eq!(plant_to_evaluate(&PersistedState::default()), None);
        assert_eq!(plant_to_evaluate(&state_with_policy()), Some("basil"));
    }
    /// **M9-016's headline criterion, through the driver the image calls.**
    ///
    /// The evaluator and the gate were host-tested from the start; what was
    /// missing was any path from the loop to them, so a provisioned device
    /// spent every outage doing nothing at all. This drives the driver exactly
    /// as `run::serve_isolated` does — one evaluation per sampling interval,
    /// monotonic instants, nothing else — and asserts that a dose reaches the
    /// pump.
    #[test]
    fn safety_013_an_isolated_device_with_a_valid_policy_waters_through_the_driver() {
        let mut driver = IsolationDriver::new();
        let mut state = state_with_policy();
        let mut nvs = FakeNvs::default();
        let mut pump = FakePump::new(call_log());
        let ids = std::cell::Cell::new(0u128);
        let mint = || {
            ids.set(ids.get() + 1);
            Uuid::from_u128(ids.get())
        };

        // A dry reading, well below the policy's 25 % trigger.
        let batch = TelemetryBatch {
            batch_id: Uuid::nil(),
            samples: vec![
                scalar(MeasurementKind::SoilMoisture, Unit::VwcPercent, 10.0),
                scalar(MeasurementKind::TankLevel, Unit::Percent, 70.0),
                MeasurementSample {
                    point: MeasurementPoint::parse("default").expect("point"),
                    kind: MeasurementKind::LeakState,
                    value: Some(MeasurementValue::Boolean(false)),
                    unit: Unit::Boolean,
                    quality: Quality::Ok,
                    sensor_id: None,
                    calibration_ref: None,
                },
            ],
        };
        let inputs = seam_inputs(&policy(), &batch, 8.0, Some(true));

        // First evaluation: confirmation starts, and credits nothing.
        let first = driver.step(
            &mut state,
            &mut nvs,
            &mut pump,
            "basil",
            &inputs,
            0,
            None,
            || CommandId::from_uuid(mint()),
            || EventId::from_uuid(mint()),
        );
        assert_eq!(first, AutonomousOutcome::Waiting, "{first:?}");
        assert_eq!(pump.total_run_ms, 0, "nothing moves before confirmation");

        // Ten minutes later the policy's 600 s confirmation is satisfied.
        let second = driver.step(
            &mut state,
            &mut nvs,
            &mut pump,
            "basil",
            &inputs,
            600_000,
            None,
            || CommandId::from_uuid(mint()),
            || EventId::from_uuid(mint()),
        );
        assert_eq!(
            second,
            AutonomousOutcome::Dosed {
                delivered_ml: 40.0,
                policy_version: 1
            },
            "{second:?}"
        );
        assert_eq!(pump.total_run_ms, 5_000, "40 ml at 8 ml/s");
        assert!(state.in_flight_dose.is_none());
        // The edge learns what happened twice over: through replayed history
        // and through the result ledger.
        assert!(!state.buffer.is_empty());
        assert_eq!(state.pending_results.len(), 1);
    }

    /// SAFETY-020. A condition that persists for a week records itself once,
    /// not once per evaluation, or it evicts the record of the dose that
    /// matters from the 64-slot audit ring.
    #[test]
    fn safety_020_an_unchanged_refusal_is_recorded_once() {
        let mut driver = IsolationDriver::new();
        let mut state = state_with_policy();
        let mut nvs = FakeNvs::default();
        let mut pump = FakePump::new(call_log());
        // An empty batch: no control reading, so every evaluation refuses for
        // the same reason.
        let inputs = seam_inputs(&policy(), &empty_batch(), 8.0, Some(true));
        let ids = std::cell::Cell::new(0u128);

        for tick in 0..20u64 {
            let mint = || {
                ids.set(ids.get() + 1);
                Uuid::from_u128(ids.get())
            };
            driver.step(
                &mut state,
                &mut nvs,
                &mut pump,
                "basil",
                &inputs,
                tick * 300_000,
                None,
                || CommandId::from_uuid(mint()),
                || EventId::from_uuid(mint()),
            );
        }
        assert_eq!(
            state.buffer.len(),
            1,
            "twenty evaluations of one unchanged condition are one record"
        );

        // A *different* refusal is recorded, so suppression never hides a
        // change an operator needs to see.
        let mut leaking = inputs.clone();
        leaking.leak = Some(LeakState::Detected);
        let mint = || {
            ids.set(ids.get() + 1);
            Uuid::from_u128(ids.get())
        };
        driver.step(
            &mut state,
            &mut nvs,
            &mut pump,
            "basil",
            &leaking,
            6_000_000,
            None,
            || CommandId::from_uuid(mint()),
            || EventId::from_uuid(mint()),
        );
        assert_eq!(state.buffer.len(), 2);
    }
    /// **The cadence survives a flapping radio.** A full-jitter backoff
    /// re-enters the isolated loop every second or two, and evaluating on each
    /// entry would mean hundreds of decisions — and candidate NVS writes — for
    /// one sampling interval's worth of readings.
    #[test]
    fn the_evaluation_cadence_is_held_across_reconnection_attempts() {
        let mut driver = IsolationDriver::new();
        let interval = 300_000;
        assert!(
            driver.due(0, interval),
            "a driver that has never evaluated is due at once"
        );

        let mut state = state_with_policy();
        let mut nvs = FakeNvs::default();
        let mut pump = FakePump::new(call_log());
        let inputs = seam_inputs(&policy(), &empty_batch(), 2.0, Some(true));
        driver.step(
            &mut state,
            &mut nvs,
            &mut pump,
            "basil",
            &inputs,
            0,
            None,
            || CommandId::from_uuid(Uuid::nil()),
            || EventId::from_uuid(Uuid::nil()),
        );

        // Several short backoffs inside one interval ask, and are told no.
        for now in [1_000, 5_000, 60_000, 299_999] {
            assert!(
                !driver.due(now, interval),
                "an evaluation at {now} ms is inside the sampling interval"
            );
        }
        assert!(driver.due(300_000, interval));
    }
}
