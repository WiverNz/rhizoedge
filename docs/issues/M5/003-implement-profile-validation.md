# Issue M5-003 — Implement profile validation that rejects rather than clamps

**Milestone:** M5 · **PRD:** [PRD 050](../../prd/050-plant-model-and-recommendations.md) · **Depends on:** M1-012

## Context

ADR-011: silent clamping means the operator believes something false about
their system and discovers it during an incident. Rejection at edit time teaches
the real limit.

## Goal

Validate profiles against internal coherence and firmware hard limits.

## Scope

- `target_min >= target_max` rejected
- `dose_ml * max_doses_per_cycle > max_daily_ml` rejected
- Non-positive intervals rejected
- **`dose_ml > FIRMWARE_MAX_ML_PER_RUN` rejected**, not clamped
- `max_daily_ml > FIRMWARE_MAX_DAILY_ML` rejected
- Each rule produces a distinct, specific error

## Non-goals

- Runtime clamping — that is the device's job (M1-009).

## Dependencies

- M1-012

## Implementation notes

Validation lives in `rhizo-domain` so it is pure and testable, and so the
same rules apply whether a profile arrives via the API, a fixture, or a future
import.

The hard-limit checks read the constants from `rhizo-mqtt-contract`, so a
firmware limit change automatically tightens profile validation.

## Acceptance criteria

- [x] Each rule rejects with its own error variant.
- [x] A valid profile passes.
- [x] `dose_ml = 200` against a hard limit of 80 is **rejected**, not clamped.
- [x] Boundary values (exactly at the limit) are accepted.
- [x] Validation is a pure function with no I/O.

## Verification

```bash
cargo test -p rhizo-domain profile::validate
```

## Tests required

- One test per rule.
- Boundary acceptance.
- An explicit test that no clamping occurs.

## Documentation impact

- None.

## Files likely affected

```text
crates/domain/src/profile.rs
```
