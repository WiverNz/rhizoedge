# Issue M1-006 — Implement typed batched telemetry and actuator payloads

**Milestone:** M1 · **PRD:** [PRD 010](../../prd/010-domain-and-mqtt-protocol.md) · **Depends on:** M1-004

## Context

Protocol sections 5.1-5.3 define typed measurement samples, one batched
telemetry payload, and actuator state. Section 10 requires
a message with one out-of-range field to remain usable: the bad field becomes
null, the good ones are kept. Discarding the whole message would throw away the
reading the safety logic needs.

## Goal

Implement ADR-017's typed batched telemetry with per-sample validation.

## Scope

- `MeasurementKind`, `MeasurementValue`, `Unit`, `Quality`, `KindSpec`
- `MeasurementPoint`, `SensorId`, `CalibrationRef`, and `MeasurementSample`
- one canonical unit and physical range per known kind
- `TelemetryBatch` (1–64 samples) and `ActuatorState`
- `validate()` returning a `ValidationReport` naming invalid sample fields
- `NaN`/`Infinity` treated as out of range
- `point` defaulting to `"default"`
- unknown kinds preserved and marked advisory-only
- scalar and boolean values remain distinct

## Non-goals

- Storing anything (M3).
- Deciding what invalid data means (M6).

## Dependencies

- M1-004

## Implementation notes

`leak_state` is a boolean measurement; a failed read is `value: null` with
`quality: fault`. It must never become evidence that a leak sensor is clear.

Validation is **not** a decode error. `validate()` returns a report; the caller
(M3-009) nulls the offending fields and records an event. A decode failure would
lose the whole message.

The complete canonical kind/unit/range table is normative in mqtt-v1.md §5.1.

## Acceptance criteria

- [x] A full and partial telemetry batch round-trip.
- [x] All eleven known kinds expose their normative canonical unit/class/range.
- [x] Boundary values 0.0 and 100.0 are valid; -0.1 and 100.1 are not.
- [x] `NaN` and `Infinity` are reported out of range.
- [x] A batch with one invalid sample still decodes; the report names that sample field.
- [x] `point` defaults when absent.
- [x] An unknown kind decodes, is preserved, and is advisory-only.
- [x] Empty and over-64 batches are rejected.

## Verification

```bash
cargo test -p rhizo-mqtt-contract payload::telemetry
```

## Tests required

- Full/partial batch and actuator-state round trips.
- Boundary tests per field.
- NaN/Infinity handling.
- Partial-validity report contents.
- Unknown-kind advisory and scalar/boolean distinction tests.

## Documentation impact

- None.

## Files likely affected

```text
crates/mqtt-contract/src/payload/telemetry.rs
crates/mqtt-contract/src/validation.rs
```
