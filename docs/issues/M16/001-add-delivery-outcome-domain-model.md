# Issue M16-001 — Add the delivery outcome and evidence domain model

**Milestone:** M16 · **PRD:** [PRD 160](../../prd/160-verified-watering.md) · **Depends on:** M11-014

## Context

The system currently collapses six different quantities into one
`delivered_ml`, and four different situations into one `completed`. Before any
sensor exists, the vocabulary has to exist — in the pure crate, where it can be
tested exhaustively and where the clock ban keeps it replayable.

[ADR-020](../../adr/020-verified-watering-and-delivery-evidence.md) §1 and §2
settle the naming; do not re-open it here.

## Goal

`rhizo_domain::delivery`: the outcome taxonomy, the ordered evidence level, the
six-dose ladder, and the pure classifier that turns evidence into an outcome.

## Scope

- New module `crates/domain/src/delivery/` with `types.rs` and `classify.rs`.
- `EvidenceLevel` — an **ordered** enum, `Commanded` < `Actuated` <
  `FlowObserved` < `FlowMeasured` < `ResponseCorroborated`.
- `DeliveryOutcome` with three families, and `UnknownReason` as a typed enum.
- `DoseLadder` carrying `requested_ml`, `authorized_ml`, `commanded_ml`,
  `effective_ml`, `estimated_ml`, `measured_ml`.
- `HydraulicEvidence`, `FlowObservation`, `WitnessHealth`, `DeliveryRecord`.
- `classify(&HydraulicEvidence, &DoseLadder) -> DeliveryOutcome`.
- Documented constants: `FLOW_START_TIMEOUT_MS`, `FLOW_SETTLE_MS`,
  `OVER_DELIVERY_FACTOR`, `PARTIAL_DELIVERY_FRACTION`,
  `MAX_PLAUSIBLE_FLOW_ML_S`, `CALIBRATION_MAX_AGE_DAYS`.

## Non-goals

- Any storage. M16-002.
- Any wire change. M16-003.
- Any hardware or witness implementation. M16-004 and M16-005.
- Renaming `irrigation::no_delivery::DeliveryEvidence`. ADR-020 §1 keeps it, and
  a rename inside the irrigation machine to make room for a new feature is cheap
  to write and expensive to review.

## Dependencies

- M11-014

## Implementation notes

**No `success: bool`, anywhere.** The whole point of the taxonomy is that the
question has more than two answers, and a boolean beside the enum is how it
quietly acquires two again.

`Ord` on `EvidenceLevel` is what makes "never report weaker evidence as
stronger" checkable rather than a convention. Derive it, and write the test that
asserts the declaration order matches the intended ordering — a reordered
variant would silently invert the comparison.

Every field that can be absent is an `Option`, and `classify` matches
exhaustively with no catch-all arm, following `safety_gate`'s discipline. A
`DeliveryOutcome` variant added later must break the build until someone decides
what it credits.

`classify` is pure and total: every combination of evidence, including all-absent
and all-nonsense, maps to exactly one outcome. A non-finite or negative
`measured_ml` maps to `FlowSensorInvalid`, never to a volume.

Cross-reference the two evidence types in both doc comments —
`no_delivery::DeliveryEvidence` is soil and pot weight (the biological half),
`delivery::HydraulicEvidence` is what left the reservoir. Someone will confuse
them otherwise.

## Acceptance criteria

- [ ] `rhizo_domain::delivery` compiles with no I/O and no `Utc::now`.
- [ ] `EvidenceLevel` is `Ord`, and a test asserts the intended ordering.
- [ ] `classify` is total, exhaustive, and has no catch-all arm.
- [ ] No `success: bool` exists in the module.
- [ ] Non-finite, negative, and implausible measured volumes map to
      `FlowSensorInvalid`.
- [ ] `OutcomeUnknown` always carries a typed reason.
- [ ] Both `DeliveryEvidence` types name each other in their doc comments.
- [ ] Every constant carries its provenance as a starting value to be measured.

## Verification

```bash
cargo test -p rhizo-domain delivery::
cargo clippy -p rhizo-domain --all-targets -- -D warnings
```

## Tests required

- A table-driven case per `DeliveryOutcome` variant.
- `EvidenceLevel` ordering, including that a missing witness never exceeds
  `Actuated`.
- Property: `classify` never panics and always returns exactly one outcome, over
  arbitrary generated evidence including non-finite values.
- A source scan asserting no `_ =>` arm in the module.

## Documentation impact

- `component-model.md`: `rhizo-domain` gains the delivery module.
- PRD 160 §Interfaces, if a signature deviates.

## Files likely affected

```text
crates/domain/src/lib.rs
crates/domain/src/delivery/mod.rs
crates/domain/src/delivery/types.rs
crates/domain/src/delivery/classify.rs
docs/architecture/component-model.md
```
