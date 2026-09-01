# Issue M16-014 — Add verified-watering observability

**Milestone:** M16 · **PRD:** [PRD 160](../../prd/160-verified-watering.md) · **Depends on:** M16-013

## Context

A feature whose subject is "what actually happened" has to be measurable, and
the question an operator asks during an incident — "was the delivery verified,
and if not, why not?" — must be a query rather than a reconstruction.

This issue also lays the groundwork for the reliability figures the product will
eventually want to state: the share of operations physically verified, the median
requested-versus-delivered error, no-flow incidents, uncertain outcomes,
automatic-watering availability. None is computed here. The requirement is that
every one of them is derivable from what is stored, without a later schema
change.

## Goal

The metric catalogue, the logs, the events, and the guarantee that the product
figures are derivable.

## Scope

- Metrics per PRD 160 §Observability: attempts, outcomes, evidence level,
  delivery error, unknown outcomes by reason, witness health and failures,
  unexpected flow, actuator health, reconciliations by resolution, calibration
  age, and verification latency.
- Structured logs: one `warn` per fault outcome carrying the six doses and the
  evidence level; one `error` per `UnexpectedFlow`; one `info` per
  reconciliation resolution.
- The device events `delivery.fault` and `flow.unexpected` surfaced in the
  device history.
- A documented derivation for each product figure, as a query, in the PRD.

## Non-goals

- Computing or publishing any product reliability figure. Later work; the
  requirement here is only that it is possible.
- A Grafana dashboard. The optional observability profile (M13-015) can consume
  these; nothing depends on it.
- Notifications. M13-007 owns dispatch.

## Dependencies

- M16-013

## Implementation notes

Cardinality is bounded by construction: `outcome` has the taxonomy's variants,
`evidence_level` five, `unknown_reason` five, `health` four, `resolution` two. No
label carries a plant, device, sensor, or actuator identifier — a home with forty
plants would otherwise multiply every series by forty, which is why the existing
catalogue does not do it either.

`Metrics::new()` caches in a `OnceLock`, so every caller in a test binary shares
one set of gauges. A test that sets one of these and reads it back is racing
every other test that touches it. Take `api::health::gauge_lock()` before
asserting on an absolute value, or assert on a delta. `api::health` failed about
one run in three before this was understood.

`rhizo_delivery_error_ml` is the most useful histogram here and its sign matters:
record `effective_ml - measured_ml`, so positive is under-delivery. Document it,
because a histogram whose sign nobody remembers gets read backwards during the
one incident it was built for.

The device events go through the buffered ring, so an isolated device's delivery
faults survive the isolation and replay — a fault that only exists while the
network is up misses the outages it matters most during.

## Acceptance criteria

- [ ] Every metric is registered, documented in the catalogue, and
      bounded-cardinality.
- [ ] No metric label carries a plant, device, sensor, or actuator identifier.
- [ ] `rhizo_delivery_error_ml`'s sign convention is documented.
- [ ] Fault outcomes log once, with the six doses and the evidence level.
- [ ] Device delivery faults appear in the device history and replay after
      isolation.
- [ ] Each product figure in PRD 160 has a documented derivation and needs no
      schema change.
- [ ] Metric tests take the gauge lock or assert on deltas.

## Verification

```bash
cargo test -p edge-controller metrics::
cargo test -p edge-controller delivery::observability
curl -s localhost:8080/metrics | grep -E 'rhizo_(watering|delivery|witness|actuator)'
```

## Tests required

- Each counter increments on its documented event and no other.
- Gauge values match a fixture population.
- A cardinality test asserting the label sets are the documented ones.
- Buffered fault replay after an isolation period.

## Documentation impact

- `docs/adr/010-observability-strategy.md`: the metric catalogue gains the
  delivery family.
- PRD 160 §Observability: the product-figure derivations.

## Files likely affected

```text
crates/edge-controller/src/metrics.rs
crates/telemetry/src/names.rs
crates/edge-controller/src/delivery/mod.rs
docs/adr/010-observability-strategy.md
docs/prd/160-verified-watering.md
```
