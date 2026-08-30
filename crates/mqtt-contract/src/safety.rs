//! The single normative, pure, allocation-free water-command gate.
use crate::{
    CommandId, UtcMillis,
    payload::{CommandResult, RejectReason, WaterCommand},
};
/// Compile-time maximum pump run.
pub const FIRMWARE_MAX_RUN_SECONDS: u32 = 20;
/// Compile-time maximum per dose.
pub const FIRMWARE_MAX_ML_PER_RUN: f32 = 80.0;
/// Compile-time rolling device budget.
pub const FIRMWARE_MAX_DAILY_ML: f32 = 500.0;
/// TTL skew allowance.
pub const MAX_CLOCK_SKEW_SECONDS: i64 = 5;
/// Fixed NVS command outcome ring size.
pub const COMMAND_DEDUP_RING: usize = 16;
/// Tri-state leak input; unknown never means clear.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LeakState {
    Clear,
    Detected,
    Unknown,
}
/// Previously executed command and stored result.
#[derive(Clone, Copy, Debug)]
pub struct PreviousCommand<'a> {
    /** Command id. */
    pub command_id: CommandId,
    /** Stored result. */
    pub result: &'a CommandResult,
}
/// Every device-side input required by ordered validation.
#[derive(Clone, Copy, Debug)]
pub struct DeviceGuardState<'a> {
    /** Stored dedup outcomes. */
    pub previous: &'a [PreviousCommand<'a>],
    /** Valid edge synchronization. */
    pub clock_synced: bool,
    /** Current device wall time. */
    pub now_ms: UtcMillis,
    /** Leak sensor. */
    pub leak: LeakState,
    /** Tank reading. */
    pub tank_percent: Option<f32>,
    /** Configured tank minimum. */
    pub tank_min_percent: f32,
    /** Pump operational enable. */
    pub pump_enabled: bool,
    /** Pump fault state. */
    pub pump_faulted: bool,
    /** Pump calibration. */
    pub pump_ml_per_second: f32,
    /** Rolling delivered volume. */
    pub delivered_today_ml: f32,
}
/// Ordered gate outcome.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CommandVerdict<'a> {
    /// Safe bounded dose.
    Accept {
        /// Effective volume.
        effective_ml: f32,
        /// Bounded duration.
        run_ms: u32,
        /// Hard limit changed request.
        clamped: bool,
    },
    /// Refusal.
    Reject(RejectReason),
    /// Deduplicated outcome to republish.
    AlreadyExecuted {
        /// Stored result.
        previous: &'a CommandResult,
    },
}
/// Executes protocol §5.8 checks 1–12 in exact order, without allocation or side effects.
pub fn validate_water_command<'a>(
    cmd: &WaterCommand,
    state: &DeviceGuardState<'a>,
) -> CommandVerdict<'a> {
    if let Some(previous) = state
        .previous
        .iter()
        .find(|p| p.command_id == cmd.command_id)
    {
        return CommandVerdict::AlreadyExecuted {
            previous: previous.result,
        };
    }
    if !state.clock_synced {
        return CommandVerdict::Reject(RejectReason::ClockUnsynced);
    }
    if state.now_ms.0
        > cmd
            .expires_at_ms
            .0
            .saturating_add(MAX_CLOCK_SKEW_SECONDS * 1000)
    {
        return CommandVerdict::Reject(RejectReason::Expired);
    }
    if !cmd.requested_ml.is_finite() || cmd.requested_ml <= 0.0 {
        return CommandVerdict::Reject(RejectReason::MalformedCommand);
    }
    if state.leak == LeakState::Detected {
        return CommandVerdict::Reject(RejectReason::LeakDetected);
    }
    if state.leak == LeakState::Unknown {
        return CommandVerdict::Reject(RejectReason::LeakUnknown);
    }
    // §5.8 step 7: an absent, unreadable, or unusably configured tank is
    // `Unknown`, never `TankLow` — the latter is a *measured* condition.
    let Some(tank) = state.tank_percent.filter(|t| t.is_finite()) else {
        return CommandVerdict::Reject(RejectReason::TankUnknown);
    };
    if !state.tank_min_percent.is_finite() {
        return CommandVerdict::Reject(RejectReason::TankUnknown);
    }
    if tank <= state.tank_min_percent {
        return CommandVerdict::Reject(RejectReason::TankLow);
    }
    // §5.8 step 9: an unusable calibration is an unavailable pump, and step 12
    // divides by it.
    if state.pump_faulted
        || !state.pump_enabled
        || !state.pump_ml_per_second.is_finite()
        || state.pump_ml_per_second <= 0.0
    {
        return CommandVerdict::Reject(RejectReason::PumpUnavailable);
    }
    match bound_dose(
        cmd.requested_ml,
        state.pump_ml_per_second,
        state.delivered_today_ml,
    ) {
        DoseBound::Accept {
            effective_ml,
            run_ms,
            clamped,
        } => CommandVerdict::Accept {
            effective_ml,
            run_ms,
            clamped,
        },
        DoseBound::Reject(reason) => CommandVerdict::Reject(reason),
    }
}

/// The outcome of the hard-limit steps.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DoseBound {
    /// A bounded dose the pump may deliver.
    Accept {
        /// Effective volume after clamping.
        effective_ml: f32,
        /// Bounded run duration.
        run_ms: u32,
        /// Whether a hard limit changed the request.
        clamped: bool,
    },
    /// A refusal. Only the two reasons steps 11 and 12 can produce.
    Reject(RejectReason),
}

/// Protocol §5.8 steps 10–12: the firmware hard limits, alone.
///
/// **The one implementation of the clamping rules**, extracted so the
/// *autonomous* actuation path can apply them without also applying the steps
/// that make no sense for it. An isolated device has no synchronised wall clock
/// and no command TTL to evaluate — SAFETY-015 says so explicitly, and
/// SAFETY-002 governs only edge commands — but the volume and duration ceilings
/// apply to every drop of water this device moves, whoever decided to move it
/// (SAFETY-007, SAFETY-014).
///
/// A second copy of these three steps for the offline path is exactly the
/// divergence ADR-008 exists to prevent, which is why this function exists
/// rather than a duplicated block.
pub fn bound_dose(
    requested_ml: f32,
    pump_ml_per_second: f32,
    delivered_today_ml: f32,
) -> DoseBound {
    if !requested_ml.is_finite() || requested_ml <= 0.0 {
        return DoseBound::Reject(RejectReason::MalformedCommand);
    }
    // Step 12 divides by the calibration, so an unusable one is an unavailable
    // pump and must be refused before the division is reached.
    if !pump_ml_per_second.is_finite() || pump_ml_per_second <= 0.0 {
        return DoseBound::Reject(RejectReason::PumpUnavailable);
    }
    let mut effective = requested_ml.min(FIRMWARE_MAX_ML_PER_RUN);
    let mut clamped = effective < requested_ml;
    // §5.8 step 11: a device that cannot prove it is under budget assumes it is
    // not. `NaN + x > max` is false, so the guard must precede the comparison.
    if !delivered_today_ml.is_finite() || delivered_today_ml + effective > FIRMWARE_MAX_DAILY_ML {
        return DoseBound::Reject(RejectReason::OverDailyMax);
    }
    let max_run_ms = FIRMWARE_MAX_RUN_SECONDS * 1000;
    let calculated = effective / pump_ml_per_second * 1000.0;
    let run_ms = if calculated > max_run_ms as f32 {
        effective = pump_ml_per_second * FIRMWARE_MAX_RUN_SECONDS as f32;
        clamped = true;
        max_run_ms
    } else {
        let truncated = calculated as u32;
        truncated + u32::from((truncated as f32) < calculated)
    };
    DoseBound::Accept {
        effective_ml: effective,
        run_ms,
        clamped,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::payload::{CommandOrigin, CommandStatus};
    use proptest::prelude::*;
    use uuid::Uuid;
    fn cmd() -> WaterCommand {
        WaterCommand {
            command_id: CommandId::from_uuid(Uuid::nil()),
            requested_ml: 40.,
            issued_at_ms: UtcMillis(0),
            expires_at_ms: UtcMillis(10000),
        }
    }
    fn state<'a>(p: &'a [PreviousCommand<'a>]) -> DeviceGuardState<'a> {
        DeviceGuardState {
            previous: p,
            clock_synced: true,
            now_ms: UtcMillis(0),
            leak: LeakState::Clear,
            tank_percent: Some(50.),
            tank_min_percent: 15.,
            pump_enabled: true,
            pump_faulted: false,
            pump_ml_per_second: 10.,
            delivered_today_ml: 0.,
        }
    }
    fn result() -> CommandResult {
        CommandResult {
            command_id: cmd().command_id,
            status: CommandStatus::Completed,
            requested_ml: 40.,
            delivered_ml: Some(40.),
            duration_ms: Some(4000),
            clamped: false,
            reason: None,
            delivered_today_ml: 40.,
            origin: CommandOrigin::EdgeCommand,
            detail: None,
        }
    }
    #[test]
    fn all_ordered_rejections() {
        let r = result();
        let previous = [PreviousCommand {
            command_id: cmd().command_id,
            result: &r,
        }];
        assert!(matches!(
            validate_water_command(&cmd(), &state(&previous)),
            CommandVerdict::AlreadyExecuted { .. }
        ));
        let mut s = state(&[]);
        s.clock_synced = false;
        assert_eq!(
            validate_water_command(&cmd(), &s),
            CommandVerdict::Reject(RejectReason::ClockUnsynced)
        );
        let mut s = state(&[]);
        s.now_ms = UtcMillis(15001);
        assert_eq!(
            validate_water_command(&cmd(), &s),
            CommandVerdict::Reject(RejectReason::Expired)
        );
        let mut c = cmd();
        c.requested_ml = 0.;
        assert_eq!(
            validate_water_command(&c, &state(&[])),
            CommandVerdict::Reject(RejectReason::MalformedCommand)
        );
        let mut s = state(&[]);
        s.leak = LeakState::Detected;
        assert_eq!(
            validate_water_command(&cmd(), &s),
            CommandVerdict::Reject(RejectReason::LeakDetected)
        );
        s.leak = LeakState::Unknown;
        assert_eq!(
            validate_water_command(&cmd(), &s),
            CommandVerdict::Reject(RejectReason::LeakUnknown)
        );
        let mut s = state(&[]);
        s.tank_percent = None;
        assert_eq!(
            validate_water_command(&cmd(), &s),
            CommandVerdict::Reject(RejectReason::TankUnknown)
        );
        s.tank_percent = Some(15.);
        assert_eq!(
            validate_water_command(&cmd(), &s),
            CommandVerdict::Reject(RejectReason::TankLow)
        );
        let mut s = state(&[]);
        s.pump_enabled = false;
        assert_eq!(
            validate_water_command(&cmd(), &s),
            CommandVerdict::Reject(RejectReason::PumpUnavailable)
        );
        let mut s = state(&[]);
        s.delivered_today_ml = 490.;
        assert_eq!(
            validate_water_command(&cmd(), &s),
            CommandVerdict::Reject(RejectReason::OverDailyMax)
        );
    }
    /// §5.8 steps 7, 9 and 11: every non-finite guard input maps to the refusal
    /// its usable counterpart would produce, never to permission (SAFETY-012).
    #[test]
    fn safety_012_nonfinite_guard_inputs_are_unknown_not_permission() {
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let mut s = state(&[]);
            s.tank_percent = Some(bad);
            assert_eq!(
                validate_water_command(&cmd(), &s),
                CommandVerdict::Reject(RejectReason::TankUnknown),
                "tank_percent {bad}"
            );
            let mut s = state(&[]);
            s.tank_min_percent = bad;
            assert_eq!(
                validate_water_command(&cmd(), &s),
                CommandVerdict::Reject(RejectReason::TankUnknown),
                "tank_min_percent {bad}"
            );
            let mut s = state(&[]);
            s.pump_ml_per_second = bad;
            assert_eq!(
                validate_water_command(&cmd(), &s),
                CommandVerdict::Reject(RejectReason::PumpUnavailable),
                "pump_ml_per_second {bad}"
            );
            let mut s = state(&[]);
            s.delivered_today_ml = bad;
            assert_eq!(
                validate_water_command(&cmd(), &s),
                CommandVerdict::Reject(RejectReason::OverDailyMax),
                "delivered_today_ml {bad}"
            );
        }
        let mut s = state(&[]);
        s.pump_ml_per_second = 0.;
        assert_eq!(
            validate_water_command(&cmd(), &s),
            CommandVerdict::Reject(RejectReason::PumpUnavailable)
        );
    }

    #[test]
    fn safety_002_expired_command_rejected_before_clamp() {
        let mut c = cmd();
        c.requested_ml = 10000.;
        let mut s = state(&[]);
        s.now_ms = UtcMillis(15001);
        assert_eq!(
            validate_water_command(&c, &s),
            CommandVerdict::Reject(RejectReason::Expired)
        );
    }
    proptest! {
    #[test]
    fn safety_007_clamp_never_exceeds_hard_max(requested in 0.0001f32..f32::MAX) {
            let mut c = cmd();
            c.requested_ml = requested;
            if let CommandVerdict::Accept { effective_ml, .. } =
                validate_water_command(&c, &state(&[]))
            {
                prop_assert!(effective_ml <= FIRMWARE_MAX_ML_PER_RUN);
            }
    }
    }
    #[test]
    fn duration_clamps_and_recomputes() {
        let mut s = state(&[]);
        s.pump_ml_per_second = 1.;
        assert_eq!(
            validate_water_command(&cmd(), &s),
            CommandVerdict::Accept {
                effective_ml: 20.,
                run_ms: 20000,
                clamped: true
            }
        );
    }
}
