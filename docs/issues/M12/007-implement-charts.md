# Issue M12-007 — Implement inline SVG charts

**Milestone:** M12 · **PRD:** [PRD 120](../../prd/120-rust-ui.md) · **Depends on:** M12-004

## Context

ADR-009: charts are inline SVG generated in Leptos — no JavaScript charting
library, because that would reintroduce the toolchain this project excludes.

## Goal

Render the time series that make trends legible.

## Scope

- Moisture, EC, and weight over a selectable range
- **The target band shaded** on the moisture chart
- Watering events marked
- Axis labels and a legend
- Graceful handling of gaps and missing series

## Non-goals

- Zoom and hover tooltips — deferred; the band and markers carry most of the value.

## Dependencies

- M12-004

## Implementation notes

The target band plus watering markers is what turns a line into an
explanation: the operator sees the plant drying toward the band's lower edge and
a dose arresting it. That combination is most of the chart's value, which is why
interactivity is deferred rather than prioritised.

Gaps must render as gaps, not as interpolated lines — an interpolated gap hides
exactly the outage the operator needs to see.

## Acceptance criteria

- [ ] All three series render as inline SVG.
- [ ] **The target band is shaded on the moisture chart.**
- [ ] Watering events are marked.
- [ ] The range selector works.
- [ ] **Gaps render as gaps, not interpolated.**
- [ ] A missing series (no scale fitted) is handled cleanly.
- [ ] No JavaScript library is used.

## Verification

```bash
cd ui/rhizo-ui && cargo test charts::
```

## Tests required

- SVG generation.
- **Gap handling.**
- Band and marker placement.

## Documentation impact

- None.

## Files likely affected

```text
ui/rhizo-ui/src/components/chart.rs
```
