# Issue M9-007 — Implement boot-safe pump state

**Milestone:** M9 · **PRD:** [PRD 090](../../prd/090-esp32-rust-firmware.md) · **Depends on:** M9-005

## Context

SAFETY-011, and the requirement software cannot compensate for on its own:
the pump GPIO must be driven inactive as the **first** statement in `main`, and
the hardware must be wired so an un-driven pin is pump-off.

## Goal

Guarantee the pump is off from the earliest possible moment.

## Scope

- `pump_off()` as the first statement in `main`, before Wi-Fi, before MQTT, before NVS
- A documented hardware requirement: gate pull-down so an un-driven pin is off,
  recorded in the board profile next to the pin it constrains
- Hardware watchdog enabled
- A watchdog reset leaves the pump off
- Host tests asserting the boot ordering

## Non-goals

- The real pump driver (M11-001).

## Dependencies

- M9-005

## Implementation notes

The bootloader window is the part software cannot cover: before any Rust
runs, the pin floats. Only a hardware pull-down makes that safe, which is why
M11-009 (HIL-1) puts a multimeter on the line across twenty resets.

Assert the ordering in a host test by recording the sequence of adapter calls —
an ordering regression is otherwise invisible until hardware exists.

`main` calls `pump_off()` on the board-supplied pump; it does not name a pin.
The pull-down requirement is a property of a specific board's wiring, so it is
documented in `src/board/devkitm1.rs` beside that board's pump pin, and every
future board profile carries the same note for its own pin. A board profile that
cannot satisfy it is not a supported board.

## Acceptance criteria

- [ ] `pump_off()` is the first statement in `main`.
- [ ] A host test asserts the call ordering.
- [ ] The watchdog is enabled.
- [ ] A watchdog reset path leaves the pump off.
- [ ] The hardware pull-down requirement is documented in the board profile,
      beside the pump pin it constrains.
- [ ] No initialisation path can energise the pump.

## Verification

```bash
cd firmware/esp32-node && cargo test boot::
cargo test safety_011_boot_state_pump_off
```

## Tests required

- **`safety_011_boot_state_pump_off`** asserting call ordering.
- Watchdog reset path.

## Documentation impact

- The board profile documents the pull-down requirement.

## Files likely affected

```text
firmware/esp32-node/src/main.rs
firmware/esp32-node/src/board/devkitm1.rs
```
