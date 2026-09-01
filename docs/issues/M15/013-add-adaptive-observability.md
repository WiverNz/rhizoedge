# Issue M15-013 — Add adaptive observability and anomaly hooks

**Milestone:** M15 · **PRD:** [PRD 150](../../prd/150-per-plant-adaptive-water-model.md) · **Depends on:** M15-012

## Context

A model that influences watering has to be observable in the same terms as
everything else that does: bounded-cardinality Prometheus metrics, structured
`tracing` events, and rows in the plant's own history. The question an operator
will ask during an incident is "was the model involved?", and the answer must be
a query rather than a reconstruction.

This is also where the anomaly-detection *hooks* land. Detecting anomalies is
deliberately not in this milestone — recording the signal a later milestone
needs is, because the signal cannot be reconstructed after the fact.

## Goal

The metric catalogue, the logs, the plant events, and a recorded
predicted-versus-observed series that a future anomaly milestone can consume
without redesign.

## Scope

- Metrics, per [PRD 150](../../prd/150-per-plant-adaptive-water-model.md)
  §Observability: `rhizo_adaptive_plants`,
  `rhizo_adaptive_confidence_plants`, `rhizo_adaptive_model_refreshes_total`,
  `rhizo_adaptive_observations_total`, `rhizo_adaptive_epoch_changes_total`,
  `rhizo_adaptive_proposals_total`, `rhizo_adaptive_static_fallback_total`,
  `rhizo_adaptive_prediction_error_vwc`, `rhizo_adaptive_refresh_duration`.
- Structured logs: one per epoch change at `info` with reason, trigger, and the
  prior estimates; one per refresh at `debug`; one at `warn` when an estimator
  refuses for a non-finite intermediate.
- `plant_events` rows for epoch changes and for a confidence transition in
  either direction.
- Recording `predicted_rise_vwc` next to the observed rise on every
  dose-response observation, which is the anomaly hook.

## Non-goals

- **Anomaly detection.** No detector, no threshold, no alert. The signal is
  recorded; deciding what "abnormal" means for a plant is a later milestone with
  its own false-positive budget.
- Notifications. M13-007 owns dispatch, and a model that pages someone before it
  has been validated on real plants is the wrong first use of it.
- A Grafana dashboard. The optional observability profile (M13-015) can consume
  these metrics; nothing here depends on it.

## Dependencies

- M15-012

## Implementation notes

Label cardinality is bounded by construction: `mode` has four values,
`confidence` four, `reason` the `EpochReason` variants, `outcome` the proposal
outcomes. No label carries a `plant_id`, for the reason the existing catalogue
does not — a home with forty plants would otherwise multiply every series by
forty.

`Metrics::new()` is a process-wide `OnceLock` singleton, so a test that sets one
of these gauges and reads it back is racing every other test in the binary. Take
`api::health::gauge_lock()` before asserting on an absolute value, or assert on a
delta. `api::health` learned this the expensive way.

`rhizo_adaptive_static_fallback_total` is the most useful of these and deserves
its labels chosen carefully: `cold_start`, `low_confidence`, `no_drying_estimate`,
`no_response_estimate`, `stale_reading`, `model_error`. "The model did nothing"
is the normal case, and an operator needs to know which normal case it was.

Prediction error is recorded, not judged. Write `predicted_rise_vwc` from the
model that was current when the dose was issued — not from the model after the
observation was folded in, which would compare a prediction against the history
that includes its own outcome.

## Acceptance criteria

- [ ] Every metric is registered, documented in the catalogue, and
      bounded-cardinality.
- [ ] No metric label carries a plant, device, or sensor identifier.
- [ ] Epoch changes and confidence transitions appear in the plant's history.
- [ ] `predicted_rise_vwc` is recorded from the pre-observation model.
- [ ] A non-finite estimator refusal produces exactly one `warn` and one counter
      increment.
- [ ] No detector, threshold, or alert is introduced.
- [ ] Metric tests take the gauge lock or assert on deltas.

## Verification

```bash
cargo test -p edge-controller metrics::
cargo test -p edge-controller hydration::observability
curl -s localhost:8080/metrics | grep rhizo_adaptive
```

## Tests required

- Each counter increments on its documented event and on no other.
- Gauge values match a fixture population of plants.
- Prediction error is computed against the pre-observation model.
- A cardinality test asserting the label sets are the documented ones.

## Documentation impact

- `docs/adr/010-observability-strategy.md`: the metric catalogue gains the
  adaptive family.
- PRD 150 §Observability, if the catalogue deviates.

## Files likely affected

```text
crates/edge-controller/src/metrics.rs
crates/telemetry/src/names.rs
crates/edge-controller/src/plant/hydration/mod.rs
docs/adr/010-observability-strategy.md
```
