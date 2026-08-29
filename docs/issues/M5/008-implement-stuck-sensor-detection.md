# Issue M5-008 — Implement stuck sensor detection

**Milestone:** M5 · **PRD:** [PRD 050](../../prd/050-plant-model-and-recommendations.md) · **Depends on:** M5-005

## Context

A sensor returning a constant value looks healthy to every range check while
telling you nothing. It is one of the failure modes SAFETY-005 must catch.

## Goal

Detect a sensor whose readings never change.

## Scope

- Count consecutive bit-identical raw readings
- At `stuck_sample_count` (default 20), mark the sensor unhealthy
- Raise a `sensor_stuck` event
- Reset on any different value

## Non-goals

- The resulting lockout (M6-004).

## Dependencies

- M5-005

## Implementation notes

Compare bit-identically, not within a tolerance. Real sensors have noise, so
genuinely identical consecutive readings are strong evidence of a fault; a
tolerance-based comparison would false-positive on a stable environment.

Twenty samples at a 300-second interval is over an hour and a half — long enough
to avoid false positives, short enough to matter.

## Acceptance criteria

- [x] 20 identical readings mark the sensor unhealthy.
- [x] 19 do not.
- [x] A different value resets the counter.
- [x] A `sensor_stuck` event is raised once, not per sample.
- [x] Noisy but stable readings do not trigger it.

## Verification

```bash
cargo test -p rhizo-domain stuck::
cargo test --test integration stuck_sensor
```

## Tests required

- Threshold boundary.
- Reset.
- Single event.
- SCEN-024.

## Documentation impact

- None.

## Files likely affected

```text
crates/domain/src/stuck.rs
```
