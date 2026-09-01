# Issue M15-003 — Extract drying segments from the measurement stream

**Milestone:** M15 · **PRD:** [PRD 150](../../prd/150-per-plant-adaptive-water-model.md) · **Depends on:** M15-002

## Context

A drying rate is only meaningful over a span in which nothing but drying
happened. The system already knows when something else happened: watering events
of every mode including `detected`, lockouts, sensor faults, and sampling gaps
are all recorded. Extraction is the act of turning that knowledge into clean
spans; if it is done badly, every number downstream is wrong in a way no
estimator can repair.

## Goal

Produce `DryingSegment` observations from a plant's measurement history,
deterministically, incrementally, and with contaminated spans discarded rather
than salvaged.

## Scope

- `plant::hydration::segments` in `edge-controller`: read validated
  `soil_moisture` samples for the plant's `control`-role binding since the last
  extracted watermark, cut them into candidate spans, and accept or discard each.
- Cut a span at: any `watering_events` row (any mode) inside it, any lockout
  interval, any sample gap wider than the plant's `max_sample_age`, any epoch
  boundary, and any sample the validator rejected.
- Discard a candidate that is shorter than `MIN_SEGMENT_HOURS`, has fewer than
  `MIN_SEGMENT_SAMPLES`, or does not fall monotonically enough to be drying.
- Record `mean_ambient_c` and `mean_illuminance` where those bindings exist.
- Persist accepted segments; advance a per-plant watermark so re-running is a
  no-op.

## Non-goals

- Fitting anything. M15-004.
- Rejecting outliers across segments — that is a property of the population, not
  of one span, and belongs to the estimator.
- Inferring undetected manual watering. What `detect` cannot see, this cannot
  see either; PRD 150 §Failure modes records the resulting bias.

## Dependencies

- M15-002

## Implementation notes

Use the **edge** `received_at` throughout, never a device timestamp — the same
rule SAFETY-005 imposes on freshness, for the same reason.

A rise inside a span is the interesting case. A small rise is noise; a rise past
`DetectConfig::moisture_delta_pp` should already have produced a `detected`
watering event and therefore already cuts the span. If it did not, the span is
discarded rather than fitted: an unexplained rise means the assumption "nothing
but drying happened here" is false, and that is exactly what the segment claims.

Extraction is incremental but must be **replayable**: extracting from scratch
over the same history must produce the same segments as extracting incrementally
in any number of passes. Test that directly; it is the property M15-014's replay
acceptance criterion rests on.

The dry-duration accumulator's lesson applies here too — fold every unobserved
sample, never one per tick, or the result becomes a property of how often the
loop runs.

## Acceptance criteria

- [ ] A synthetic clean drying history produces exactly the expected segments.
- [ ] A watering event of any mode, including `detected`, cuts the span.
- [ ] A lockout interval cuts the span.
- [ ] A gap wider than `max_sample_age` cuts the span.
- [ ] An unexplained rise discards the span.
- [ ] Segments never cross an epoch boundary.
- [ ] Incremental extraction and from-scratch extraction agree exactly.
- [ ] Re-running extraction with no new samples writes nothing.

## Verification

```bash
cargo test -p edge-controller hydration::segments
cargo test -p edge-controller hydration::segments::replay
```

## Tests required

- Each cut condition, separately and in combination.
- Incremental-versus-from-scratch equivalence over a generated history.
- A history containing only invalid samples produces no segments and no error.
- Environmental means are absent, not zero, when the bindings are absent.

## Documentation impact

- PRD 150 §Data model: any deviation in the accepted-span rule.

## Files likely affected

```text
crates/edge-controller/src/plant/hydration/mod.rs
crates/edge-controller/src/plant/hydration/segments.rs
crates/storage/src/repo/hydration.rs
```
