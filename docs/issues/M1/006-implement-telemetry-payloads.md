# Issue M1-006 — Implement telemetry payload types and range validation

**Milestone:** M1 · **PRD:** [PRD 010](../../prd/010-domain-and-mqtt-protocol.md) · **Depends on:** M1-004

## Context

Protocol section 5.1-5.4 defines four telemetry payloads. Section 10 requires
a message with one out-of-range field to remain usable: the bad field becomes
null, the good ones are kept. Discarding the whole message would throw away the
reading the safety logic needs.

## Goal

Implement the telemetry payloads with per-field range validation.

## Scope

- `SoilTelemetry`, `WeightTelemetry`, `TankTelemetry`, `PumpTelemetry`
- Range constants for every field
- `validate()` returning a `ValidationReport` naming each out-of-range field
- `NaN`/`Infinity` treated as out of range
- `point` defaulting to `"default"`
- `leak_detected: null` decoding to an explicit Unknown, never to `false`

## Non-goals

- Storing anything (M3).
- Deciding what invalid data means (M6).

## Dependencies

- M1-004

## Implementation notes

The `leak_detected: null` case is safety-critical. It must decode to a
tri-state (`Clear | Detected | Unknown`), because mapping it to `false` would
silently convert a broken sensor into a permission to pump (SAFETY-012).

Validation is **not** a decode error. `validate()` returns a report; the caller
(M3-009) nulls the offending fields and records an event. A decode failure would
lose the whole message.

Ranges: moisture 0-100, temperature -20-80, EC 0-20000, tank 0-100,
weight 0-100000.

## Acceptance criteria

- [ ] All four payloads round-trip.
- [ ] Boundary values 0.0 and 100.0 are valid; -0.1 and 100.1 are not.
- [ ] `NaN` and `Infinity` are reported out of range.
- [ ] A payload with one invalid field still decodes; the report names that field.
- [ ] `point` defaults when absent.
- [ ] `leak_detected: null` yields Unknown, and `Unknown != Clear` is asserted.

## Verification

```bash
cargo test -p rhizo-mqtt-contract payload::telemetry
```

## Tests required

- Round trip per payload.
- Boundary tests per field.
- NaN/Infinity handling.
- Partial-validity report contents.
- An explicit test that Unknown is not Clear.

## Documentation impact

- None.

## Files likely affected

```text
crates/mqtt-contract/src/payload/telemetry.rs
crates/mqtt-contract/src/validation.rs
```
