# Issue M11-008 — Add the hardware configuration schema

**Milestone:** M11 · **PRD:** [PRD 110](../../prd/110-real-pump-and-safety-hardware.md) · **Depends on:** M11-005, M11-006

## Context

Pump calibration, tank sensor kind and thresholds, and leak sensor polarity
are device config (ADR-011 layer L3) — configurable, but never safety limits.

## Goal

Extend device config with pump and safety-hardware settings.

## Scope

- `pump` block: `ml_per_second`, `enabled`, `calibrated_at`
- `tank` block: kind, `min_percent`, raw calibration points
- `leak` block: kind, `active_low`
- Validation with rejection
- **Still no safety limit fields**

## Non-goals

- Anything that could raise a firmware hard limit.

## Dependencies

- M11-005
- M11-006

## Implementation notes

`pump.ml_per_second` is a calibration value, not a safety limit: a wrong
calibration changes accuracy, and the firmware clamps on **duration** as well as
volume, so an inflated calibration cannot produce an over-long run.

Assert once more that no config path affects `FIRMWARE_MAX_*`.

## Acceptance criteria

- [ ] All three blocks are accepted and applied.
- [ ] Invalid values are rejected and the previous config retained.
- [ ] `calibrated_at` is recorded.
- [ ] **No config field affects a firmware hard limit**, asserted by a test.
- [ ] Changes apply as documented.

## Verification

```bash
cd firmware/esp32-node && cargo test config::hardware
```

## Tests required

- Application and validation.
- **Hard limits unaffected.**

## Documentation impact

- protocol/mqtt-v1.md config section extended additively.

## Files likely affected

```text
firmware/esp32-node/src/app/config.rs
crates/mqtt-contract/src/payload/config.rs
```
