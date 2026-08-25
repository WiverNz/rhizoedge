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

## Acceptance criteria

- [ ] Staleness uses `received_at` from the edge clock.
- [ ] The threshold formula and its floor are correct.
- [ ] Automatic watering is blocked when stale.
- [ ] **Manual watering is permitted when stale or under sensor fault.**
- [ ] **Manual watering is still blocked by leak, tank, and daily cap.**
- [ ] A device with a wrong clock does not affect the computation.
- [ ] It auto-clears on fresh data.

## Verification

```bash
cargo test -p rhizo-domain gate::stale
cargo test safety_005 safety_003
```

## Tests required

- `safety_005_stale_or_invalid_blocks_auto` property test over random ages.
- **The manual exception, and its precise boundaries.**
- Wrong device clock has no effect.

## Documentation impact

- None.

## Files likely affected

```text
crates/domain/src/irrigation/gate.rs
```
