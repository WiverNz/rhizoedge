# Issue M11-003 — Implement pump fault handling

**Milestone:** M11 · **PRD:** [PRD 110](../../prd/110-real-pump-and-safety-hardware.md) · **Depends on:** M11-002

## Context

Failure-model 5.2: a welded relay or shorted MOSFET means the pump cannot be
turned off by software. The correct response is to stop trying and tell someone.

## Goal

Detect and latch a pump fault.

## Scope

- Overrun or failure to de-energise sets `faulted`
- A faulted pump refuses all commands with `pump_unavailable`
- The fault clears only on reboot
- `pump_fault` published as a device event
- The edge locks out the plant

## Non-goals

- Automatic recovery — a hardware fault needs hands.

## Dependencies

- M11-002

## Implementation notes

Latching until reboot is deliberate. A fault that clears itself would let a
failing driver oscillate between working and not, delivering unpredictable
volumes.

The edge-side lockout is `PumpFault`, which is explicit-clear (M6-003).

## Acceptance criteria

- [ ] An overrun sets `faulted`.
- [ ] A faulted pump refuses commands with `pump_unavailable`.
- [ ] The fault survives until reboot.
- [ ] `pump_fault` reaches the edge as an event.
- [ ] The plant is locked out.
- [ ] Clearing requires an explicit operator action after a reboot.

## Verification

```bash
cd firmware/esp32-node && cargo test pump::fault
```

## Tests required

- Fault latching.
- Command refusal.
- Event propagation.

## Documentation impact

- None.

## Files likely affected

```text
firmware/esp32-node/src/pump/fault.rs
```
