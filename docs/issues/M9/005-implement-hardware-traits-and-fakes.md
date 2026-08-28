# Issue M9-005 — Define hardware traits and fake adapters

**Milestone:** M9 · **PRD:** [PRD 090](../../prd/090-esp32-rust-firmware.md) · **Depends on:** M9-003

## Context

PRD 090 F-090-40 and F-090-44…F-090-46. The trait boundary is what lets M10 and
M11 swap in real hardware without the edge learning about it — and what lets
M9's safety logic be host-tested.

It is also what makes the board replaceable. The traits are the seam the board
layer fills: `board-devkitc02` constructs the real adapters from
ESP32-C3-DevKitC-02 pins, a later `board-xiao-esp32c3` constructs them from
different ones, and nothing above the trait can tell which
([ADR-007](../../adr/007-esp32-rust-framework-and-toolchain.md), amended
2026-08-28).

## Goal

Define the hardware abstraction and its host-testable fakes.

## Scope

- `Pump`, `SoilSensor`, `TankSensor`, `LeakSensor`, `Scale`, `Clock`, `NvsStore`
- **`Clock::now_ms() -> Option<i64>`** — `None` means unsynced
- Fake adapters for all of them, configurable from tests
- **Construction of every real adapter from the board profile** (M9-003's
  `src/board/`), so a trait implementation receives its pin and polarity rather
  than naming one
- The trait signatures carrying no pin, port, or polarity in any argument or
  associated type — a trait that mentions GPIO 4 is not a hardware abstraction

## Non-goals

- Real adapters (M10, M11).

## Dependencies

- M9-003

## Implementation notes

`Option` on the clock rather than a sentinel is deliberate: an unsynced clock
is not a time, and the type makes forgetting to check impossible. That is the
mechanism behind SAFETY-002's refusal path.

Pin assignments belong to the board profile, not here. This issue defines *what*
a pump is; `src/board/devkitc02.rs` defines *which pin* it is on and whether it
is active-high. Keeping the two apart is what makes the XIAO a new file instead
of a refactor, and M9-003's structural check fails the suite if a pin number
appears in `src/pump/` or `src/sensors/`.

Fakes must be configurable to produce every failure the real hardware can:
read errors, stuck values, out-of-range readings, and a pump that reports
success without delivering.

## Acceptance criteria

- [ ] All seven traits are defined.
- [ ] `Clock::now_ms` returns `Option`.
- [ ] Fakes exist for every trait and are configurable.
- [ ] Fakes can simulate read errors, stuck values, and no-delivery.
- [ ] Pin assignments live in the board profile, and no trait definition or
      adapter outside `src/board/` names a pin, port, or polarity.
- [ ] Real adapters are constructed by the board profile and handed to the app
      as trait objects.
- [ ] Host tests can drive the full app with fakes only, with **no** board
      profile involved.

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
firmware/esp32-node/src/board/mod.rs
firmware/esp32-node/src/board/devkitc02.rs
```
