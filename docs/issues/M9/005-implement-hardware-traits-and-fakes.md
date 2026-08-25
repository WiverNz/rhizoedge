# Issue M9-005 — Define hardware traits and fake adapters

**Milestone:** M9 · **PRD:** [PRD 090](../../prd/090-esp32-rust-firmware.md) · **Depends on:** M9-003

## Context

PRD 090 F-090-40. The trait boundary is what lets M10 and M11 swap in real
hardware without the edge learning about it — and what lets M9's safety logic be
host-tested.

## Goal

Define the hardware abstraction and its host-testable fakes.

## Scope

- `Pump`, `SoilSensor`, `TankSensor`, `LeakSensor`, `Scale`, `Clock`, `NvsStore`
- **`Clock::now_ms() -> Option<i64>`** — `None` means unsynced
- Fake adapters for all of them, configurable from tests
- `board.rs` centralising pin assignments

## Non-goals

- Real adapters (M10, M11).

## Dependencies

- M9-003

## Implementation notes

`Option` on the clock rather than a sentinel is deliberate: an unsynced clock
is not a time, and the type makes forgetting to check impossible. That is the
mechanism behind SAFETY-002's refusal path.

Fakes must be configurable to produce every failure the real hardware can:
read errors, stuck values, out-of-range readings, and a pump that reports
success without delivering.

## Acceptance criteria

- [ ] All seven traits are defined.
- [ ] `Clock::now_ms` returns `Option`.
- [ ] Fakes exist for every trait and are configurable.
- [ ] Fakes can simulate read errors, stuck values, and no-delivery.
- [ ] Pin assignments are in one file.
- [ ] Host tests can drive the full app with fakes only.

## Verification

```bash
cd firmware/esp32-node && cargo test adapters::
```

## Tests required

- Fake behaviour per trait.
- Failure simulation.

## Documentation impact

- None.

## Files likely affected

```text
firmware/esp32-node/src/sensors/mod.rs
firmware/esp32-node/src/pump/mod.rs
firmware/esp32-node/src/board.rs
```
