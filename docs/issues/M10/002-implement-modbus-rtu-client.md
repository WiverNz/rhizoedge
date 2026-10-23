# Issue M10-002 — Implement a generic Modbus RTU client

**Milestone:** M10 · **PRD:** [PRD 100](../../prd/100-real-soil-sensor.md) · **Depends on:** M10-001

## Context

PRD 100 F-100-03/06. RS485 is the strategic sensor path, and a generic client
means a new probe model is configuration rather than code.

## Goal

Implement Modbus RTU over plain UART TX/RX through the DFR0845 isolated RS485
module.

## Scope

- Read holding and input registers
- CRC-16 computation and verification
- Configurable slave address, baud, parity
- Configurable response timeout with a documented default
- Timeout, CRC error, and exception response handled **distinctly**

## Non-goals

- Modbus TCP or writing registers.

## Dependencies

- M10-001

## Implementation notes

The reference RS485 interface is the **DFRobot DFR0845**, whose MCU side is TX,
RX, GND and VCC only. It switches the bus direction itself, so this client owns
no DE or /RE GPIO and no turnaround setting (F-100-06): direction timing is not
a firmware parameter on this board, and adding one would configure nothing.
What remains timing-sensitive is Modbus RTU framing itself: honour the
3.5-character inter-frame silence and a bounded response timeout. A future
board profile with a bare transceiver would add DE/RE control in its board
layer, not in this client.

Distinct error variants matter for diagnosis: a timeout usually means wiring or
address, a CRC error usually means cable length or termination.

## Acceptance criteria

- [ ] Register reads work against a mock responder.
- [ ] CRC is computed and verified correctly.
- [ ] Timeout, CRC error, and exception produce distinct errors.
- [ ] The response timeout is configurable and its default documented; no DE/RE
      GPIO or turnaround setting is introduced for the DFR0845 reference build.
- [ ] Host tests cover the frame layer with a fake UART.

## Verification

```bash
cd firmware/esp32-node && cargo test modbus::
```

## Tests required

- Frame encode/decode with known vectors.
- CRC correctness.
- Each error path.
- Exception response parsing.

## Documentation impact

- None.

## Files likely affected

```text
firmware/esp32-node/src/sensors/modbus/mod.rs
```
