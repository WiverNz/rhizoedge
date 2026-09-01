# Issue M16-006 — Add witness calibration and its versioning

**Milestone:** M16 · **PRD:** [PRD 160](../../prd/160-verified-watering.md) · **Depends on:** M16-005

## Context

M11-004 already established how this project calibrates: five runs, mean and
standard deviation, rejected if σ exceeds 5 % of the mean, stored with a date,
because "high variance means an air lock, a partially occluded tube, or a failing
pump head, and averaging it away produces a calibration that is wrong in a way
nobody notices."

A witness needs the same treatment, and it needs one thing more: a *version*, so
that a volume measured under one calibration is never silently compared with one
measured under another.

## Goal

Versioned, dated witness calibration, and the rule that bad or stale calibration
degrades confidence rather than producing confident wrong numbers.

## Scope

- Scale factor and zero-offset for the reservoir scale, stored in device config
  with `calibration_version` and `calibrated_at`.
- A calibration procedure reusing `command.calibrate`'s discipline: repeated
  known deliveries, mean and standard deviation, variance rejection.
- Cross-check against the pump's own `ml_per_second`: a witness that disagrees
  with calibration beyond `CALIBRATION_DISAGREEMENT_FACTOR` is `Degraded`.
- `calibration_version` and `calibrated_at` on every `DeliveryRecord`.
- Age handling: beyond `CALIBRATION_MAX_AGE_DAYS` the evidence level degrades
  from `FlowMeasured` to `FlowObserved`.

## Non-goals

- Automatic recalibration. A calibration that adjusts itself to match a drifting
  sensor is how a drift becomes invisible.
- Invalidating a delivery for stale calibration. F-160-17 degrades the evidence
  level; the water still went somewhere and the record still says so.

## Dependencies

- M16-005

## Implementation notes

**Bad calibration must degrade confidence, never create certainty.** The failure
this rule exists to prevent: a witness with a wrong scale factor reporting 12 ml
for a 40 ml dose, the system recording `DeliveredVerified: 12 ml`, and every
downstream consumer — the operator, the audit trail, and eventually M15's
dose-response estimator — believing a number that is simply wrong. Disagreement
with the pump calibration is the cheapest available cross-check and it is free:
both numbers already exist for every dose.

Note the asymmetry with the budget rule. `credited_ml` charges
`max(estimated, measured)`, so a low-reading witness cannot buy budget — that
protects the *plant*. Degrading to `Degraded` on disagreement protects the
*record*. Both are needed and neither substitutes for the other.

A version, not just a date. Two deliveries measured under different scale
factors are not comparable, and a date makes that discoverable only by
arithmetic; a version makes it a mismatch. This is the same reasoning M15's
`model_version` uses for estimator semantics.

Reuse `command.calibrate`'s existing full-gate requirement (F-110-10): a
calibration run moves real water and gets the same validation as a dose, with no
bypass or subset validator.

## Acceptance criteria

- [ ] Scale factor, zero offset, `calibration_version`, and `calibrated_at` are
      stored in device config.
- [ ] The procedure rejects a high-variance calibration set.
- [ ] Disagreement with `ml_per_second` beyond the factor yields `Degraded`.
- [ ] Calibration older than `CALIBRATION_MAX_AGE_DAYS` degrades the evidence
      level and does not invalidate the delivery.
- [ ] Every `DeliveryRecord` carries the version and the date in force at the
      time of the dose.
- [ ] A calibration run passes the same gate a water command does.
- [ ] Nothing recalibrates automatically.

## Verification

```bash
cd firmware/esp32-node && cargo test witness::calibration
cargo test -p rhizo-domain delivery::calibration
```

## Tests required

- Variance rejection.
- Disagreement detection in both directions.
- Age-based degradation, at and either side of the boundary.
- A record written under one version is never compared with one under another.

## Documentation impact

- `docs/protocol/mqtt-v1.md` §5.7: the config fields.
- PRD 110 §Calibration: a note that the witness is calibrated the same way.

## Files likely affected

```text
firmware/esp32-node/src/witness/calibration.rs
crates/domain/src/delivery/types.rs
crates/mqtt-contract/src/payload/status.rs
docs/protocol/mqtt-v1.md
```
