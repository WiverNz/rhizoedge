# Issue M10-002 — Implement a generic Modbus RTU client

**Milestone:** M10 · **PRD:** [PRD 100](../../prd/100-real-soil-sensor.md) · **Depends on:** M10-001

## Context

PRD 100 F-100-03/06. RS485 is the strategic sensor path, and a generic client
means a new probe model is configuration rather than code.

## Goal

Implement Modbus RTU over UART with RS485 direction control.

## Scope

- Read holding and input registers
- CRC-16 computation and verification
- Configurable slave address, baud, parity
- RS485 half-duplex direction control with correct turnaround timing
- Timeout, CRC error, and exception response handled **distinctly**

## Non-goals

- Modbus TCP or writing registers.

## Dependencies

- M10-001

## Implementation notes

Turnaround timing is the classic RS485 bug: releasing the driver too early
truncates the last byte, too late collides with the response. Make it
configurable and document the default.

Distinct error variants matter for diagnosis: a timeout usually means wiring or
address, a CRC error usually means cable length or termination.

## Acceptance criteria

- [ ] Register reads work against a mock responder.
- [ ] CRC is computed and verified correctly.
- [ ] Timeout, CRC error, and exception produce distinct errors.
- [ ] Direction control timing is configurable.
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
