# Issue M1-012 — Create the domain crate with the Clock trait and state enums

**Milestone:** M1 · **PRD:** [PRD 010](../../prd/010-domain-and-mqtt-protocol.md) · **Depends on:** M1-006, M0-010

## Context

ADR-006 requires the domain to be pure and clock-free so safety logic is
property-testable in microseconds. The enums land now so M3 and M4 can store and
report state before the M6 state machine exists.

## Goal

Create `rhizo-domain` with identifiers, state enums, the `Clock` trait, and profile types.

## Scope

- `PlantId`, `ProfileId`, `WateringEventId` newtypes
- `PlantState`, `IrrigationState`, `LockoutReason`, `WateringMode` enums
- `Clock` trait and `SystemClock`
- `TestClock` in testkit implementing `Clock`
- `PlantProfile` and `SoilSample` with `is_valid()` and `is_stale(now, max_age)`
- No I/O, no direct clock access

## Non-goals

- Transitions (M6-006).
- The recommendation engine (M5-009).
- Profile validation logic (M5-003).

## Dependencies

- M1-006
- M0-010

## Implementation notes

Define the enums exactly as PRD 010's state model section lists them, so
M3 and M4 can persist them as strings without a later rename.

`SoilSample::is_stale` takes `now` and `max_age` as parameters rather than
consulting a clock, which is what makes it testable without a runtime.

`TestClock` already exists from M0-010; this issue makes it implement the trait
rather than defining a second clock.

## Acceptance criteria

- [x] The crate depends only on `rhizo-mqtt-contract`.
- [x] `grep -r 'Utc::now\|SystemTime::now' crates/domain/src` returns nothing.
- [x] `TestClock` implements `Clock`.
- [x] All state enums serialise to stable snake_case strings.
- [x] `is_stale` is a pure function of its parameters.

## Verification

```bash
cargo test -p rhizo-domain
cargo tree -p rhizo-domain
```

## Tests required

- Enum serde round trips (the strings become database values).
- `is_valid` and `is_stale` boundary cases.
- `SystemClock` and `TestClock` both satisfy the trait.

## Documentation impact

- Crate docs stating the purity constraint.

## Files likely affected

```text
crates/domain/src/lib.rs
crates/domain/src/clock.rs
crates/domain/src/state.rs
crates/domain/src/profile.rs
crates/testkit/src/clock.rs
```
