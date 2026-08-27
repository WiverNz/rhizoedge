# Issue M2-005 — Implement pot weight, tank, leak, and EC models

**Milestone:** M2 · **PRD:** [PRD 020](../../prd/020-device-simulator.md) · **Depends on:** M2-004

## Context

Weight responds immediately to delivered water while VWC lags. That
divergence is what makes weight useful for detecting manual watering and for
catching a pump that runs without delivering, so the simulator must reproduce
it rather than smoothing it away.

## Goal

Implement the remaining sensor models.

## Scope

- Pot weight: `dry_weight + water_g + noise`, rising **immediately** on delivery
- Evapotranspiration reducing water mass over time
- Tank level depleting by delivered volume
- Leak state, injected only
- EC rising as VWC falls, with fertilisation step events
- A `stable` flag on weight readings during settling

## Non-goals

- Publishing (M2-006).
- Fault injection (M2-013).

## Dependencies

- M2-004

## Implementation notes

The immediate-weight versus lagging-VWC divergence is the point of this
issue. A model where both respond identically would make the weight-based
no-delivery detection (M6-017) untestable.

EC: `base_ec * (reference_vwc / current_vwc)` with noise — concentration rises
as water leaves, which is the real relationship.

## Acceptance criteria

- [x] Weight rises immediately on delivery while VWC still lags.
- [x] Weight decreases over time through evapotranspiration.
- [x] Tank depletes by exactly the delivered volume.
- [x] Tank never goes negative.
- [x] EC rises as moisture falls.
- [x] A fertilisation event steps EC up and then decays.
- [x] The `stable` flag is false briefly after a change.

## Verification

```bash
cargo test -p device-simulator model::
```

## Tests required

- Weight/VWC divergence after a dose (the key assertion).
- Tank depletion arithmetic and floor.
- EC/VWC inverse relationship.
- Fertilisation step and decay.

## Documentation impact

- None.

## Files likely affected

```text
crates/device-simulator/src/model/weight.rs
crates/device-simulator/src/model/tank.rs
crates/device-simulator/src/model/ec.rs
```
