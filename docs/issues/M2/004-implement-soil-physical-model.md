# Issue M2-004 — Implement the soil moisture and temperature model

**Milestone:** M2 · **PRD:** [PRD 020](../../prd/020-device-simulator.md) · **Depends on:** M2-001

## Context

Simulator-strategy section 3 specifies exponential drying, absorption lag,
probe overshoot, and drainage beyond field capacity. The last two exist
specifically because they punish a naive controller — a controller that doses
again on an overshoot-inflated reading should fail in tests, not in a pot.

## Goal

Implement the soil model with the behaviours that exercise control logic.

## Scope

- Exponential decay toward `VWC_floor`, temperature-scaled
- Delivered water entering a pending-absorption pool with `absorption_tau`
- Surface overshoot up to 15% of the change, decaying over ~2 min
- Drainage: volume beyond `field_capacity_vwc` is lost, not measured
- Temperature drift with a diurnal component
- Gaussian noise, on by default

## Non-goals

- Weight, tank, or EC (M2-005).
- Publishing (M2-006).

## Dependencies

- M2-001

## Implementation notes

Drying: `dVWC/dt = -k * (VWC - VWC_floor) * temp_factor(T)`, with
`temp_factor` 1.0 at 21 C, +3% per degree above, floored at 0.5.

Absorption reaches 63% of the change within `absorption_tau` (default 6 min).

Model state advances by virtual elapsed time (M2-014), so it must take `dt`
rather than reading a clock.

Noise defaults on. A controller that only works on clean signals does not work.

## Acceptance criteria

- [ ] Moisture decreases monotonically (modulo noise) with no water added.
- [ ] Drying is faster at higher temperature.
- [ ] Moisture never falls below `VWC_floor`.
- [ ] A dose raises moisture gradually, not instantly.
- [ ] Overshoot appears and decays.
- [ ] Water beyond field capacity does not raise measured VWC proportionally.
- [ ] With noise disabled, the model is deterministic for a given dt sequence.

## Verification

```bash
cargo test -p device-simulator model::soil
```

## Tests required

- Monotonic drying.
- Temperature scaling.
- Floor respected.
- Absorption time constant reached within tolerance.
- Overshoot magnitude and decay.
- Drainage cap.
- Determinism without noise.

## Documentation impact

- None; simulator-strategy.md section 3 is the specification.

## Files likely affected

```text
crates/device-simulator/src/model/soil.rs
```
