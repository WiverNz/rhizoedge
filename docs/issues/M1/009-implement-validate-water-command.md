# Issue M1-009 — Implement validate_water_command — the shared actuation gate

**Milestone:** M1 · **PRD:** [PRD 010](../../prd/010-domain-and-mqtt-protocol.md) · **Depends on:** M1-008

## Context

**The single most important function in the project.** ADR-008 requires the
simulator and the firmware to share one actuation gate so the simulator cannot be
more permissive than hardware. Without it, every safety test in M6 validates a
device that does not exist.

## Goal

Implement the ordered validation from protocol section 5.8, allocation-free and no_std.

## Scope

- Hard-limit constants: `FIRMWARE_MAX_RUN_SECONDS`, `FIRMWARE_MAX_ML_PER_RUN`, `FIRMWARE_MAX_DAILY_ML`, `MAX_CLOCK_SKEW_SECONDS`, `COMMAND_DEDUP_RING`
- `DeviceGuardState` carrying every input the checks need
- `CommandVerdict`: `Accept { effective_ml, run_ms, clamped }`, `Reject(reason)`, `AlreadyExecuted { previous }`
- The twelve checks in **exactly** the documented order
- Steps 10 and 12 clamp; every other failure rejects

## Non-goals

- Actuation itself (M2-008, M9-011).
- NVS or state persistence.

## Dependencies

- M1-008

## Implementation notes

Order is normative, not incidental. An expired *and* oversized command must
reject as `expired`, not clamp — the test for this exists because getting the
order wrong produces a subtly wrong device.

Allocation-free: no `Vec`, no `String`. It runs on a device with 400 KB of SRAM
and may be called from a constrained context.

`DeviceGuardState.tank_percent` is `Option<f32>`; `None` rejects with
`TankUnknown` (SAFETY-012). Same for the leak tri-state.

Steps: dedup, clock_synced, expired, malformed, leak, leak-unknown, tank-unknown,
tank-low, pump-unavailable, clamp-ml, daily-max, clamp-duration.

## Acceptance criteria

- [x] Every one of the twelve checks has a test asserting its exact verdict.
- [x] An expired **and** oversized command rejects as `Expired` (order proof).
- [x] `requested_ml = 10000` never yields an `Accept` above `FIRMWARE_MAX_ML_PER_RUN`.
- [x] `clock_synced = false` always rejects, whatever else is true.
- [x] `tank_percent = None` rejects with `TankUnknown`.
- [x] `leak = Unknown` rejects with `LeakUnknown`.
- [x] A `command_id` in the ring yields `AlreadyExecuted` and never `Accept`.
- [x] The function performs no heap allocation.
- [x] It compiles with `--no-default-features`.

## Verification

```bash
cargo test -p rhizo-mqtt-contract safety::
PROPTEST_CASES=10000 cargo test -p rhizo-mqtt-contract safety_007
cargo build -p rhizo-mqtt-contract --no-default-features
```

## Tests required

- One test per ordered check.
- The order-proof test.
- `safety_002_expired_command_rejected`.
- `safety_007_clamp_never_exceeds_hard_max` as a property test over random requested_ml including absurd values.
- `safety_012` cases: tank None, leak Unknown, clock unsynced.

## Documentation impact

- Doc comment stating this is the only actuation gate and that a second implementation is forbidden.

## Files likely affected

```text
crates/mqtt-contract/src/safety.rs
```
