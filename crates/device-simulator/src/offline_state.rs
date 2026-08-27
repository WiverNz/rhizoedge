//! Monotonic runtime state for offline autonomy, and the seam M6-019 connects.
//!
//! # M2 prepares; it does not decide
//!
//! Everything here gathers and persists. **Nothing here classifies.** There is
//! no `Dose`, no `Refuse`, and no threshold comparison, because the one shared
//! `rhizo_policy::evaluate_offline` is implemented in M6-019 and called from
//! exactly one place per consumer. A simulator-specific evaluator — even a
//! small one, even a temporary one — is the divergence
//! [ADR-008](../../../../docs/adr/008-shared-code-simulator-and-firmware.md)
//! exists to prevent: every offline safety test in M6 and every isolation
//! scenario in M8 would then be exercising rules the hardware does not follow.
//!
//! [`OfflineSeam`] is deliberately shaped as the exact argument list of that
//! function. M6-019's change to this crate is one call, not a redesign.
//!
//! # Why the state is monotonic and stored as *remaining*
//!
//! `cooldown_remaining_ms` is a remaining duration, never a wall-clock
//! deadline, because an isolated device may have no absolute time at all. On
//! boot the remaining duration is restored intact, so a reboot cannot shorten a
//! cooldown. `budget_used_ml` is likewise never cleared on boot: it is reduced
//! only when the device can demonstrate the window elapsed. A device that
//! reboots repeatedly does not thereby earn more water (SAFETY-015).

use std::collections::BTreeMap;

use rhizo_mqtt_contract::payload::{MeasurementKind, MeasurementValue, OfflinePolicy, Quality};
use rhizo_mqtt_contract::safety::LeakState;
use rhizo_policy::{MonotonicMillis, OfflineCycle, OfflineInputs, OfflineSample, OfflineState};
use serde::{Deserialize, Serialize};

use crate::state::OfflineRuntime;

/// A persistable mirror of [`rhizo_policy::OfflineCycle`].
///
/// The shared type is `no_std` and pure, and deliberately carries no `serde`
/// derive; this is the storage form. Conversion is exhaustive in both
/// directions, so a phase added to the shared crate fails to compile here until
/// someone decides how it is stored.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CyclePhase {
    /// Not in a watering cycle.
    #[default]
    Idle,
    /// Accumulating continuous time below the trigger.
    Confirming,
    /// Waiting for a delivered dose to be absorbed.
    WaitAbsorption,
    /// Waiting out the cooldown after a completed cycle.
    Cooldown,
}

impl From<CyclePhase> for OfflineCycle {
    fn from(phase: CyclePhase) -> Self {
        match phase {
            CyclePhase::Idle => Self::Idle,
            CyclePhase::Confirming => Self::Confirming,
            CyclePhase::WaitAbsorption => Self::WaitAbsorption,
            CyclePhase::Cooldown => Self::Cooldown,
        }
    }
}

impl From<OfflineCycle> for CyclePhase {
    fn from(cycle: OfflineCycle) -> Self {
        match cycle {
            OfflineCycle::Idle => Self::Idle,
            OfflineCycle::Confirming => Self::Confirming,
            OfflineCycle::WaitAbsorption => Self::WaitAbsorption,
            OfflineCycle::Cooldown => Self::Cooldown,
        }
    }
}

impl OfflineRuntime {
    /// The shared evaluator's view of this state.
    #[must_use]
    pub fn as_offline_state(&self) -> OfflineState {
        OfflineState {
            cycle: self.cycle.into(),
            dose_count: self.dose_count,
            budget_used_ml: self.budget_window.delivered_ml,
            cooldown_remaining: MonotonicMillis(self.cooldown_remaining_ms),
            confirm_elapsed: MonotonicMillis(self.confirmation_elapsed_ms),
        }
    }

    /// Adopts the evaluator's updated state.
    ///
    /// The window's own elapsed time is *not* taken from the evaluator: it is
    /// device bookkeeping, advanced by observed monotonic time, and the
    /// evaluator has no opinion about it.
    pub fn apply_offline_state(&mut self, state: &OfflineState) {
        self.cycle = state.cycle.into();
        self.dose_count = state.dose_count;
        self.budget_window.delivered_ml = state.budget_used_ml;
        self.cooldown_remaining_ms = state.cooldown_remaining.0;
        self.confirmation_elapsed_ms = state.confirm_elapsed.0;
    }

    /// Advances the state by observed monotonic time.
    ///
    /// **Only by time the device actually observed.** Cooldown is counted down
    /// and the budget window advanced from elapsed milliseconds the monotonic
    /// clock really produced — never from a wall-clock difference across a
    /// reboot, which the device cannot vouch for.
    pub fn advance(&mut self, elapsed_ms: u64) {
        self.cooldown_remaining_ms = self.cooldown_remaining_ms.saturating_sub(elapsed_ms);
        self.budget_window.elapsed_ms = self.budget_window.elapsed_ms.saturating_add(elapsed_ms);
    }

    /// Rolls the rolling budget window over once it has fully elapsed.
    ///
    /// Called with the policy's window length. The budget is reduced **only**
    /// here, and only on evidence the device observed the whole window pass. A
    /// device that reboots repeatedly does not thereby earn more water.
    pub fn roll_window(&mut self, window_ms: u64) {
        if window_ms > 0 && self.budget_window.elapsed_ms >= window_ms {
            self.budget_window.elapsed_ms = 0;
            self.budget_window.delivered_ml = 0.0;
        }
    }
}

/// The most recent reading of one measurement kind, with its monotonic age.
#[derive(Clone, Debug, PartialEq)]
pub struct RecentSample {
    /// The value, or `None` for a failed read.
    pub value: Option<MeasurementValue>,
    /// Reported quality.
    pub quality: Quality,
    /// Monotonic instant the sample was taken.
    pub taken_at_ms: u64,
}

/// Per-kind last readings, held in RAM.
///
/// SAFETY-017 requires per-kind sample ages so a required measurement that has
/// gone stale can block actuation. Deliberately **not** persisted: a reading
/// that survived a reboot would carry an age the device cannot vouch for, and
/// an unknown-age reading must count as missing rather than as fresh.
#[derive(Clone, Debug, Default)]
pub struct RecentSamples {
    by_kind: BTreeMap<String, RecentSample>,
}

impl RecentSamples {
    /// Records a reading.
    pub fn record(
        &mut self,
        kind: &MeasurementKind,
        value: Option<MeasurementValue>,
        quality: Quality,
        taken_at_ms: u64,
    ) {
        self.by_kind.insert(
            kind.as_str().to_owned(),
            RecentSample {
                value,
                quality,
                taken_at_ms,
            },
        );
    }

    /// The most recent reading of a kind, if there is one.
    #[must_use]
    pub fn get(&self, kind: &MeasurementKind) -> Option<&RecentSample> {
        self.by_kind.get(kind.as_str())
    }

    /// How many kinds have been seen.
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_kind.len()
    }

    /// Whether nothing has been recorded.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_kind.is_empty()
    }

    /// Builds the shared evaluator's sample form, with a monotonic age.
    #[must_use]
    pub fn as_offline_sample(
        &self,
        kind: &MeasurementKind,
        monotonic_now_ms: u64,
    ) -> Option<OfflineSample> {
        self.get(kind).map(|recent| OfflineSample {
            kind: kind.clone(),
            value: recent.value,
            quality: recent.quality,
            age: MonotonicMillis(monotonic_now_ms.saturating_sub(recent.taken_at_ms)),
        })
    }
}

/// Everything `rhizo_policy::evaluate_offline` will be handed, and nothing else.
///
/// The four fields are that function's four parameters, in order. M6-019 adds
/// the call; M2 builds the arguments and stops. There is deliberately no method
/// here that returns a decision, and no field that holds one.
#[derive(Clone, Debug, PartialEq)]
pub struct OfflineSeam<'a> {
    /// The activated policy for this plant.
    pub policy: &'a OfflinePolicy,
    /// Persisted monotonic state.
    pub state: OfflineState,
    /// Fail-closed inputs, every absent one explicitly `None`.
    pub inputs: OfflineInputs,
    /// Monotonic time since boot.
    pub elapsed: MonotonicMillis,
}

/// Assembles the evaluator's inputs from what the device currently knows.
///
/// Every absent input is `None` rather than a default. A missing leak reading is
/// not "clear", a missing tank level is not "full", and an unknown pump health
/// is not "healthy" — absence of evidence is not permission (SAFETY-012), and
/// building these with defaults is precisely how that invariant would be lost
/// before the evaluator ever ran.
#[must_use]
pub fn gather_inputs(
    policy: &OfflinePolicy,
    samples: &RecentSamples,
    monotonic_now_ms: u64,
    leak: Option<LeakState>,
    tank_percent: Option<f32>,
    pump_healthy: Option<bool>,
) -> OfflineInputs {
    OfflineInputs {
        control: samples.as_offline_sample(&policy.control_measurement.kind, monotonic_now_ms),
        required: policy
            .required_measurements
            .iter()
            .filter_map(|required| samples.as_offline_sample(&required.kind, monotonic_now_ms))
            .collect(),
        leak,
        tank_percent,
        pump_healthy,
    }
}

// ---------------------------------------------------------------- the seam

impl crate::device::Device {
    /// The arguments `rhizo_policy::evaluate_offline` will be called with.
    ///
    /// **The single integration seam M6-019 connects.** It returns exactly that
    /// function's four parameters and nothing else: no decision, no
    /// classification, no threshold comparison. M6-019's change here is one
    /// call added at one call site.
    ///
    /// `None` means there is nothing to evaluate — no activated policy for this
    /// plant — which is itself the fail-closed answer: absence of a policy is
    /// not permission (SAFETY-013).
    #[must_use]
    pub fn offline_seam(&self, plant_id: &str) -> Option<OfflineSeam<'_>> {
        let policy = self
            .active_policy()?
            .policies
            .iter()
            .find(|p| p.plant_id.as_str() == plant_id)?;
        let now = self.uptime_ms();
        Some(OfflineSeam {
            policy,
            state: self.store().state().offline_runtime.as_offline_state(),
            inputs: gather_inputs(
                policy,
                self.recent_samples(),
                now,
                self.leak_reading(),
                self.tank_reading(),
                self.pump_health(),
            ),
            elapsed: MonotonicMillis(now),
        })
    }

    /// The plants this device holds an activated policy for.
    #[must_use]
    pub fn policy_plants(&self) -> Vec<String> {
        self.active_policy().map_or_else(Vec::new, |set| {
            set.policies
                .iter()
                .map(|p| p.plant_id.as_str().to_owned())
                .collect()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{BudgetWindow, OfflineRuntime};

    fn runtime() -> OfflineRuntime {
        OfflineRuntime {
            cycle: CyclePhase::Cooldown,
            budget_window: BudgetWindow {
                elapsed_ms: 1_800_000,
                delivered_ml: 70.0,
            },
            cooldown_remaining_ms: 14_400_000,
            confirmation_elapsed_ms: 45_000,
            dose_count: 2,
        }
    }

    #[test]
    fn the_persisted_state_round_trips_through_the_shared_type() {
        let before = runtime();
        let mut after = OfflineRuntime::default();
        after.apply_offline_state(&before.as_offline_state());
        // The window's own elapsed time is device bookkeeping the evaluator
        // does not carry, so it is compared separately.
        after.budget_window.elapsed_ms = before.budget_window.elapsed_ms;
        assert_eq!(after, before);
    }

    #[test]
    fn every_cycle_phase_converts_both_ways() {
        for phase in [
            CyclePhase::Idle,
            CyclePhase::Confirming,
            CyclePhase::WaitAbsorption,
            CyclePhase::Cooldown,
        ] {
            let shared: OfflineCycle = phase.into();
            assert_eq!(CyclePhase::from(shared), phase);
        }
    }

    #[test]
    fn advancing_counts_the_cooldown_down_and_the_window_up() {
        let mut state = runtime();
        state.advance(400_000);
        assert_eq!(state.cooldown_remaining_ms, 14_000_000);
        assert_eq!(state.budget_window.elapsed_ms, 2_200_000);
    }

    #[test]
    fn a_cooldown_never_goes_negative_or_wraps() {
        let mut state = runtime();
        state.advance(u64::MAX);
        assert_eq!(state.cooldown_remaining_ms, 0);
        assert_eq!(state.budget_window.elapsed_ms, u64::MAX);
    }

    /// SAFETY-015: the budget is reduced only on observed elapsed time.
    #[test]
    fn the_budget_is_replenished_only_when_the_whole_window_was_observed() {
        let mut state = runtime();
        let window = 86_400_000;

        state.roll_window(window);
        assert_eq!(
            state.budget_window.delivered_ml, 70.0,
            "half a window is not a window"
        );

        state.advance(window);
        state.roll_window(window);
        assert_eq!(state.budget_window.delivered_ml, 0.0);
        assert_eq!(state.budget_window.elapsed_ms, 0);
    }

    #[test]
    fn a_zero_length_window_never_replenishes_anything() {
        let mut state = runtime();
        state.advance(1_000_000_000);
        state.roll_window(0);
        assert_eq!(
            state.budget_window.delivered_ml, 70.0,
            "a nonsensical window must not hand out a fresh allowance"
        );
    }

    #[test]
    fn recent_samples_carry_a_monotonic_age() {
        let mut samples = RecentSamples::default();
        assert!(samples.is_empty());
        samples.record(
            &MeasurementKind::SoilMoisture,
            Some(MeasurementValue::Scalar(31.7)),
            Quality::Ok,
            1_000,
        );
        assert_eq!(samples.len(), 1);

        let sample = samples
            .as_offline_sample(&MeasurementKind::SoilMoisture, 61_000)
            .expect("a recorded kind");
        assert_eq!(sample.age, MonotonicMillis(60_000));
        assert_eq!(sample.value, Some(MeasurementValue::Scalar(31.7)));
        assert_eq!(sample.quality, Quality::Ok);
    }

    #[test]
    fn a_kind_never_sampled_is_absent_rather_than_defaulted() {
        let samples = RecentSamples::default();
        assert!(
            samples
                .as_offline_sample(&MeasurementKind::TankLevel, 1_000)
                .is_none(),
            "absence of evidence is not a reading of zero"
        );
    }

    #[test]
    fn a_failed_read_is_recorded_as_a_failed_read_not_dropped() {
        let mut samples = RecentSamples::default();
        samples.record(&MeasurementKind::LeakState, None, Quality::Fault, 500);
        let sample = samples
            .as_offline_sample(&MeasurementKind::LeakState, 500)
            .unwrap();
        assert_eq!(sample.value, None);
        assert_eq!(sample.quality, Quality::Fault);
    }

    #[test]
    fn absent_safety_inputs_stay_absent_in_the_gathered_inputs() {
        use rhizo_mqtt_contract::payload::{
            ActuatorKind, ControlMeasurement, MeasurementPoint, OfflineActuator, OfflineLimits,
            OfflinePolicy, OfflineSafety, RequiredMeasurement, SensorId,
        };
        let policy = OfflinePolicy {
            plant_id: SensorId::parse("monstera-01").unwrap(),
            policy_version: 7,
            enabled: true,
            actuator: Some(OfflineActuator {
                actuator_id: SensorId::parse("pump-0").unwrap(),
                kind: ActuatorKind::IrrigationPump,
                dose_ml: 35.0,
                max_doses_per_cycle: 3,
                absorption_wait_ms: 900_000,
            }),
            control_measurement: ControlMeasurement {
                kind: MeasurementKind::SoilMoisture,
                point: MeasurementPoint::parse("default").unwrap(),
                trigger_below: 28.0,
                resume_above: 34.0,
                confirm_duration_ms: 1_800_000,
                max_age_ms: 900_000,
            },
            required_measurements: vec![RequiredMeasurement {
                kind: MeasurementKind::TankLevel,
                point: MeasurementPoint::parse("reservoir").unwrap(),
                max_age_ms: 1_800_000,
            }],
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
        };

        let inputs = gather_inputs(&policy, &RecentSamples::default(), 0, None, None, None);
        assert!(inputs.control.is_none());
        assert!(inputs.required.is_empty());
        assert!(inputs.leak.is_none());
        assert!(inputs.tank_percent.is_none());
        assert!(inputs.pump_healthy.is_none());

        let mut samples = RecentSamples::default();
        samples.record(
            &MeasurementKind::SoilMoisture,
            Some(MeasurementValue::Scalar(20.0)),
            Quality::Ok,
            0,
        );
        samples.record(
            &MeasurementKind::TankLevel,
            Some(MeasurementValue::Scalar(72.0)),
            Quality::Ok,
            0,
        );
        let inputs = gather_inputs(
            &policy,
            &samples,
            60_000,
            Some(LeakState::Clear),
            Some(72.0),
            Some(true),
        );
        assert_eq!(
            inputs.control.as_ref().unwrap().age,
            MonotonicMillis(60_000)
        );
        assert_eq!(inputs.required.len(), 1);
        assert_eq!(inputs.leak, Some(LeakState::Clear));
        assert_eq!(inputs.tank_percent, Some(72.0));
        assert_eq!(inputs.pump_healthy, Some(true));
    }
}
