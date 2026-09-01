# Issue M15-005 — Extract dose-response observations from watering events

**Milestone:** M15 · **PRD:** [PRD 150](../../prd/150-per-plant-adaptive-water-model.md) · **Depends on:** M15-004

## Context

The system already captures a pre-dose reading (`irrigation_state.pre_dose_vwc`)
and already watches for a response inside the absorption window, and then
reduces the whole thing to a boolean through `recovery_delta_vwc`. The magnitude
it discards is precisely the observation this model needs.

## Goal

Turn each qualifying completed watering event into a `DoseResponseObservation`,
with the same discard-rather-than-salvage discipline M15-003 applies to segments.

## Scope

- `plant::hydration::responses`: for each `completed` `watering_events` row in
  the current epoch not yet observed, find the last validated reading within
  `max_sample_age` before `started_at` and the **peak** validated reading inside
  `automation.absorption` after `completed_at`.
- Require both; discard the event otherwise.
- Record `requested_ml`, `delivered_ml`, `verified` (whether `delivered_ml` was
  reported), `pre_vwc`, `peak_vwc`, `peak_at`, `rise_vwc`, and `vwc_per_ml`.
- Discard when another watering event of any mode falls inside the window, when
  a lockout began inside it, or when the rise is negative.
- Persist accepted observations and advance a watermark, as M15-003 does.

## Non-goals

- Fitting. M15-006.
- Using `detected` watering events as observations: a manual dose has no known
  volume, and `DetectedWatering::estimated_ml` is `None` for moisture-only
  detections by design. A `detected` event still *discards* an overlapping
  observation, which is the role it is qualified for.
- Flow-sensed verified delivery. See M15-014 §Future work and PRD 150.

## Dependencies

- M15-004

## Implementation notes

Take the **peak** inside the absorption window, not the last reading in it. A
probe reads its maximum some minutes after the water arrives and then settles;
the last reading in the window measures how far it has already settled, which
mixes the response with the drying rate the other estimator is separately trying
to measure.

`credited_ml`'s conservatism does not transfer here. That function deliberately
charges the full request for `interrupted` and `failed` because over-counting is
the safe direction for a **budget**. For an **observation**, charging a volume
that may not have been delivered corrupts the learned response — so only
`completed` events qualify, which is the same set `creates_watering_event`
already admits.

Where `delivered_ml` is absent, use `requested_ml` and set `verified = false`.
M15-006 weights it lower and the explanation says so; dropping it entirely would
discard every observation from a device that cannot measure flow, which is every
device before verified watering exists.

## Acceptance criteria

- [ ] A completed event with a fresh pre-dose reading and a peak in the window
      produces exactly one observation.
- [ ] A missing pre-dose reading, a stale one, or an absent peak discards it.
- [ ] `rejected`, `interrupted`, and `failed` events produce nothing.
- [ ] An overlapping watering event of any mode discards it.
- [ ] A negative rise discards it.
- [ ] An absent `delivered_ml` produces `verified = false`, not a discard.
- [ ] The peak, not the last reading, is used.
- [ ] Incremental and from-scratch extraction agree exactly.

## Verification

```bash
cargo test -p edge-controller hydration::responses
```

## Tests required

- Each discard condition, separately.
- Peak-versus-last selection on a rise-then-settle history.
- Observations never cross an epoch boundary.
- A `detected` event inside a window discards, and never becomes an observation.

## Documentation impact

- PRD 150 §Data model, if the qualifying rule deviates.

## Files likely affected

```text
crates/edge-controller/src/plant/hydration/responses.rs
crates/storage/src/repo/hydration.rs
```
