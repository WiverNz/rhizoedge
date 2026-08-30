# Issue M6-005 — Implement the staleness gate check and the manual exception

**Milestone:** M6 · **PRD:** [PRD 060](../../prd/060-irrigation-control-and-safety.md) · **Depends on:** M6-004, M4-004

## Context

SAFETY-005 completed. This issue also implements the deliberate asymmetry:
manual watering is permitted under sensor fault and stale data, because a human
has looked at the plant and taken responsibility.

## Goal

Implement the freshness check and the mode-dependent privilege difference.

## Scope

- `sample_age = now - received_at` using the **edge** clock
- Threshold `max(15 min, 3 x telemetry_interval)`
- `StaleData` lockout when exceeded
- **`manual` mode skips only `SensorFault` and `StaleData`** — every other check applies
- Auto-clears when fresh data resumes

## Non-goals

- Any further manual relaxation — leak, tank, and caps always apply.

## Dependencies

- M6-004
- M4-004

## Implementation notes

The manual exception is precise and must not widen. Manual watering is
permitted with a broken or silent sensor; it remains blocked by leak, empty
tank, daily cap, and the firmware hard limits. The UI explains this (PRD 120 F-120-27).

`received_at`, never `device_time_ms`. A device with a backwards clock must not
be able to make stale data look fresh.

**The threshold is `max(15 min, 3 × telemetry_interval)` and takes no power
field.** Call `device::health::max_sample_age_seconds`, which accepts a cadence
and nothing else. Do **not** reach for `liveness_interval_seconds`: that is the
M4 registry's *liveness* cadence, it is widened by the battery
`wake_interval_seconds`, and it exists for the connectivity badge. A device
declaring a long wake interval must not thereby make an old control measurement
actionable — the same reason this check uses `received_at`
([PRD 040](../../prd/040-device-registry-and-health.md) F-040-26,
[ADR-018](../../adr/018-battery-and-deep-sleep-device-mode.md) §7). Per-plant
`MeasurementPolicy.stale_after` (M5-014) overrides the default; it is plant
configuration, never a device declaration.

## Acceptance criteria

- [x] Staleness uses `received_at` from the edge clock.
- [x] The threshold formula and its floor are correct.
- [x] Automatic watering is blocked when stale.
- [x] **Manual watering is permitted when stale or under sensor fault.**
- [x] **Manual watering is still blocked by leak, tank, and daily cap.**
- [x] A device with a wrong clock does not affect the computation.
- [x] No power field, and no `devices.wake_interval_seconds`, reaches the
      threshold — a battery device declaring an 86 400-second wake interval
      blocks automatic watering on a stale sample exactly like any other device.
- [x] It auto-clears on fresh data.

## Verification

```bash
cargo test -p rhizo-domain gate::stale
cargo test safety_005 safety_003
```

## Tests required

- `safety_005_stale_or_invalid_blocks_auto` property test over random ages.
- A battery device with a long declared wake interval is still blocked when
  stale.
- **The manual exception, and its precise boundaries.**
- Wrong device clock has no effect.

## Documentation impact

- None.

## Files likely affected

```text
crates/domain/src/irrigation/gate.rs
```
