# Issue M5-022 — M5 verification and exit criteria

**Milestone:** M5 · **PRD:** [PRD 050](../../prd/050-plant-model-and-recommendations.md) · **Depends on:** M5-001, M5-002, M5-003, M5-004, M5-005, M5-006, M5-007, M5-008, M5-009, M5-010, M5-011, M5-012, M5-013, M5-014, M5-015, M5-016, M5-017, M5-018, M5-019, M5-021

## Context

Final gate for M5. The last milestone before the system can move water.

## Goal

Verify every PRD 050 acceptance criterion.

## Scope

- Full gate plus integration
- Verify the zero-commands property specifically
- Update ROADMAP.md and record the report

## Non-goals

- New behaviour.

## Dependencies

- M5-001
- M5-002
- M5-003
- M5-004
- M5-005
- M5-006
- M5-007
- M5-008
- M5-009
- M5-010
- M5-011
- M5-012
- M5-013
- M5-014
- M5-015
- M5-016
- M5-017
- M5-018
- M5-021

## Implementation notes

The verification that matters: run a complete drying cycle and confirm the
plant reaches `WaterRecommended` with reasons while **no MQTT command is
published at all**. That property is what makes M5 a safe place to validate the
recommendation logic against a real plant before M6 gives it a pump.

## Acceptance criteria

- [ ] All gate commands pass.
- [ ] Simulator drying produces `WaterRecommended` with a non-empty reason list.
- [ ] **Zero MQTT commands published during the entire scenario.**
- [ ] A profile with `dose_ml = 200` is rejected with 422 naming the limit.
- [ ] A manual moisture step creates a `detected` event and resets the cooldown.
- [ ] A step following a command creates **no** second event.
- [ ] Trend is `None` with fewer than 5 valid samples.
- [ ] A new plant has `auto_watering_enabled = false`.
- [ ] ROADMAP.md updated and the report recorded.

## Verification

```bash
cargo test --workspace --all-features
cargo test --test integration
mosquitto_sub -h localhost -u rhizo-edge -P "$P" -t 'rhizo/v1/devices/+/commands/#' -v  # silent
cargo run --manifest-path tools/docscheck/Cargo.toml
```

## Tests required

- Full suite.

## Documentation impact

- ROADMAP.md.
- Milestone report.

## Files likely affected

```text
ROADMAP.md
```
