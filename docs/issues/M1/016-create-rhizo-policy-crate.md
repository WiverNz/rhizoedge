# Issue M1-016 — Create the rhizo-policy crate

**Milestone:** M1 · **PRD:** [PRD 010](../../prd/010-domain-and-mqtt-protocol.md) · **Depends on:** M1-014, M1-012

## Context

[ADR-015](../../adr/015-device-offline-autonomy.md) puts the offline evaluator in
its own `no_std` crate so the firmware, the simulator, and the edge all run one
implementation of the offline rules. This issue creates the crate and its types;
the evaluator logic lands in M6-019.

## Goal

Establish `rhizo-policy` with its state and decision types, `no_std` and pure.

## Scope

- `crates/policy` as workspace member `rhizo-policy`, `#![no_std]` + `alloc`
- Depends only on `rhizo-mqtt-contract`; no I/O, no clock, no `std` in the default build
- `OfflineState` — cycle state, dose count, budget accumulator, cooldown remaining, confirm elapsed
- `OfflineInputs` — latest sample per required kind with age, leak, tank, pump health
- `OfflineDecision` — `Idle` | `Confirming` | `Dose` | `WaitAbsorption` | `Cooldown` | `Refuse(reason)`
- `RefuseReason` covering every gate step in offline-autonomy.md §4
- `MonotonicMillis` newtype — elapsed time is a parameter, never read

## Non-goals

- The evaluator function body (M6-019).
- Persistence (M2-016, M9-015).

## Dependencies

- M1-014
- M1-012

## Implementation notes

Every absent-able input is `Option` or an explicit tri-state, exactly as in
`IrrigationInputs`. `unwrap_or_default()` on a safety input is the failure this
shape exists to prevent.

The crate must not read a clock. `evaluate_offline` will take
`elapsed: MonotonicMillis` as a parameter — that is what makes SAFETY-015
structural rather than disciplined, and what makes the property tests trivial.

Add it to `[workspace.dependencies]` so `rhizo-domain` and, later, the firmware
both pick up one version.

## Acceptance criteria

- [x] `cargo build -p rhizo-policy --no-default-features` succeeds.
- [x] The crate depends only on `rhizo-mqtt-contract` within the workspace.
- [x] `grep` finds no `Utc::now`, `SystemTime`, or `Instant` in the crate.
- [x] Every absent-able field in `OfflineInputs` is `Option` or a tri-state.
- [x] `RefuseReason` has a variant for every gate step in offline-autonomy.md §4.
- [x] `rhizo-domain` links it and compiles.

## Verification

```bash
cargo build -p rhizo-policy --no-default-features
cargo test -p rhizo-policy
cargo tree -p rhizo-policy
```

## Tests required

- Type construction.
- A dependency test asserting no std-only or I/O crates.
- A grep-based test asserting no clock access.

## Documentation impact

- ADR-001 crate table already lists it; verify accurate.

## Files likely affected

```text
Cargo.toml
crates/policy/Cargo.toml
crates/policy/src/lib.rs
crates/policy/src/types.rs
```
