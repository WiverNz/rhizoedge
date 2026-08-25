# Issue M13-010 — Add measurement downsampling

**Milestone:** M13 · **PRD:** [PRD 130](../../prd/130-multi-plant-home.md) · **Depends on:** M13-001

## Context

ADR-004 retention: raw measurements kept 90 days, older data downsampled to
hourly. At 20 devices that is roughly a gigabyte a year of raw data on an SD
card.

## Goal

Bound long-term storage without losing the shape of history.

## Scope

- `measurements_hourly` with avg, min, max, and sample count
- A downsampling task running beyond the raw retention window
- The `resolution` API parameter served from the aggregate for `hour` and `day`
- Charts using the aggregate for long ranges

## Non-goals

- Downsampling watering events — never aggregated.

## Dependencies

- M13-001

## Implementation notes

Keeping min and max alongside the average matters: an hourly average hides
the dry excursion that triggered a dose, which is exactly what someone reviewing
a plant's history is looking for.

Watering events, commands, and device events are never aggregated — they are the
ledger (M3-015).

## Acceptance criteria

- [ ] Hourly aggregates are computed with avg, min, max, and count.
- [ ] Raw data beyond the window is pruned after aggregation.
- [ ] `resolution=hour` and `day` are served from the aggregate.
- [ ] Charts use the aggregate for long ranges.
- [ ] **Watering events are never aggregated.**
- [ ] Storage growth is bounded.

## Verification

```bash
cargo test -p edge-controller downsample::
curl -s 'localhost:8080/api/v1/plants/monstera-01/measurements?resolution=hour' | jq
```

## Tests required

- Aggregation correctness including min/max.
- Pruning after aggregation.
- **Ledger tables untouched.**

## Documentation impact

- ADR-004 retention section verified.

## Files likely affected

```text
crates/edge-controller/src/retention.rs
migrations/edge/0005_downsampling.sql
```
