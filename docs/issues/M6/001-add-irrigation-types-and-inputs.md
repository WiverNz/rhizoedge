# Issue M6-001 — Define IrrigationInputs and IrrigationDecision

**Milestone:** M6 · **PRD:** [PRD 060](../../prd/060-irrigation-control-and-safety.md) · **Depends on:** M5-019

## Context

ADR-006 makes the state machine a pure function. `IrrigationInputs` is the
complete set of things a watering decision may consider — which is why
SAFETY-009 can be enforced structurally: the struct has no cloud-derived field,
and adding one would be a visible change to a type named in the invariants.

## Goal

Define the input and decision types that the gate and machine operate on.

## Scope

- `IrrigationInputs` exactly as PRD 060 specifies
- **`Option<T>` for every input that can be absent** — no defaults
- `IrrigationDecision` with Idle, Recommend, IssueDose, Wait, Lock, CycleComplete
- `EvaluationMode`: Automatic or ManualRequest
- `LeakState` and `TankState` as tri-states including Unknown

## Non-goals

- The gate (M6-002).
- The machine (M6-006).

## Dependencies

- M5-019

## Implementation notes

`Option` rather than a default is the whole design. `unwrap_or_default()` on a
safety input silently converts 'we do not know' into 'it is fine', which is
exactly the SAFETY-012 failure.

`LeakState` must be a three-variant enum (`Clear | Detected | Unknown`), not
`Option<bool>` — `Option<bool>` invites `unwrap_or(false)`, and a named
`Unknown` variant forces the gate to classify it.

The struct must have no field derived from cloud state, ever.

## Acceptance criteria

- [ ] `IrrigationInputs` matches PRD 060's definition field for field.
- [ ] Every absent-able input is `Option` or an explicit tri-state.
- [ ] `LeakState` has an `Unknown` variant distinct from `Clear`.
- [ ] **No field is derived from cloud state.**
- [ ] `rhizo-domain` has no dependency on `rhizo-cloud-client`.
- [ ] The types are pure data with no methods that perform I/O.

## Verification

```bash
cargo test -p rhizo-domain irrigation::types
grep -c 'cloud' crates/domain/Cargo.toml   # expect 0
```

## Tests required

- Type construction.
- An explicit assertion that Unknown != Clear.
- A dependency test asserting no cloud crate is present.

## Documentation impact

- Doc comment stating that adding a cloud field would violate SAFETY-009.

## Files likely affected

```text
crates/domain/src/irrigation/types.rs
```
