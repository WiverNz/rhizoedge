# Issue M5-007 — Implement manual watering detection

**Milestone:** M5 · **PRD:** [PRD 050](../../prd/050-plant-model-and-recommendations.md) · **Depends on:** M5-005

## Context

The system should notice water it did not deliver. PRD 050 F-050-16 adds the
critical constraint: a rise following a completed command must not be
double-counted, which would corrupt both the cooldown and the daily total.

## Goal

Detect operator watering from moisture and weight steps.

## Scope

- Moisture rise >= `detect_moisture_delta` (default 8 pp) between consecutive samples
- Weight rise >= `detect_weight_delta` (default 100 g) where a scale exists
- Weight gives the better volume estimate when available
- **Attribution**: a rise within the absorption window of a completed command is not a detection
- Creates a `watering_events` row with `mode='detected'` and `command_id = NULL`

## Non-goals

- Acting on the detection (M6).

## Dependencies

- M5-005

## Implementation notes

Attribution is the requirement that protects SAFETY-006's accounting. Without
it, every automatic dose would also register as a detected watering, and the
plant would appear to have received twice what it did.

`mode='detected'` rows are excluded from the automatic daily cap (they were not
automatic) but **do** reset the cooldown — a human watered the plant, so the
machine should wait.

## Acceptance criteria

- [ ] A moisture step above the threshold creates a `detected` event.
- [ ] A weight step creates one and gives a better volume estimate.
- [ ] **A rise following a completed command creates no event.**
- [ ] Detection resets time-since-last-watering.
- [ ] `mode='detected'` rows are excluded from the automatic daily total.
- [ ] Sub-threshold changes create nothing.

## Verification

```bash
cargo test -p rhizo-domain detect::
cargo test --test integration manual_watering_detection
```

## Tests required

- Moisture and weight detection.
- **Command attribution suppression.**
- Cooldown reset.
- Daily-total exclusion.
- Threshold boundaries.

## Documentation impact

- None.

## Files likely affected

```text
crates/domain/src/detect.rs
crates/edge-controller/src/plant/detect.rs
```
