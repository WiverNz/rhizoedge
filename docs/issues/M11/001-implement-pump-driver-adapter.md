# Issue M11-001 — Implement the real pump driver adapter

**Milestone:** M11 · **PRD:** [PRD 110](../../prd/110-real-pump-and-safety-hardware.md) · **Depends on:** M10-013

## Context

PRD 110 F-110-01 through F-110-03. **The hardware pull-down is the requirement
software cannot compensate for**: the pump must be electrically off when the pin
is un-driven, covering reset and the bootloader window.

## Goal

Drive a real pump safely.

## Scope

- `RealPump` implementing the **existing** `Pump` trait
- GPIO driving a MOSFET module with a **gate pull-down**
- Pump on its own supply, never the ESP32 rail
- Duration from `effective_ml / ml_per_second`, clamped
- Actual run duration measured and reported

## Non-goals

- The run guard (M11-002).

## Dependencies

- M10-013

## Implementation notes

MOSFET over relay: no contacts to weld, faster turn-off, and a gate
pull-down is trivial. A relay is acceptable only if the pump needs AC.

Document the wiring requirement in `board.rs` alongside the pin definition, so
whoever wires the next board reads it at the right moment.

Measuring actual duration rather than assuming it is what makes overrun
detectable.

## Acceptance criteria

- [ ] The pump runs for the computed duration.
- [ ] Actual duration is measured and reported.
- [ ] The gate pull-down requirement is documented at the pin definition.
- [ ] The pump has a separate supply.
- [ ] Duration is clamped to `FIRMWARE_MAX_RUN_SECONDS`.
- [ ] Host tests cover the logic with a fake GPIO.

## Verification

```bash
cd firmware/esp32-node && cargo test pump::real
```

## Tests required

- Duration computation and clamping.
- Measured duration reporting.

## Documentation impact

- board.rs wiring requirements.

## Files likely affected

```text
firmware/esp32-node/src/pump/real.rs
firmware/esp32-node/src/board.rs
```
