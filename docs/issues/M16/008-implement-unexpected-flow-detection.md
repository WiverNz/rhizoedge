# Issue M16-008 — Implement unexpected and continued flow detection

**Milestone:** M16 · **PRD:** [PRD 160](../../prd/160-verified-watering.md) · **Depends on:** M16-007

## Context

The system has no representation for water moving when nothing authorised it. A
siphon through a tube left below the reservoir waterline, a valve stuck open, or
a pump driven by a shorted MOSFET empties a reservoir into a pot with no leak
asserted — the tray can be perfectly dry — and no command in flight. Today it is
invisible until the reservoir is empty and the plant is drowned.

This is **SAFETY-024**, and it is deliberately a separate issue from M16-007:
different trigger, different severity, different recovery.

## Goal

Detect flow with no authorisation and flow that continues after shutdown, act
locally and immediately, and make the fault survive a reconnect.

## Scope

- Continued flow past `FLOW_SETTLE_MS` after pump shutdown → assert the pump-off
  path, latch the actuator, publish `flow.unexpected`, refuse further commands.
- Flow observed with no authorised actuation → the same fault class, raised from
  the idle sampling path.
- Cumulative flow beyond the plausible bound during either → the same fault.
- The fault buffered as a device event so an isolated device's fault survives and
  replays.
- `RejectReason::UnexpectedFlow` on every subsequent command.
- The fault latched until reboot **and** an explicit operator clear.

## Non-goals

- Edge-side lockout and recovery. M16-011.
- Distinguishing *why* — siphon, stuck valve, welded relay. The device's job is
  to stop and say so; diagnosis needs hands.
- New hardware. The desirable production answer is a normally-closed solenoid,
  documented in PRD 160 §Hardware hard stop and deliberately not built here.

## Dependencies

- M16-007

## Implementation notes

**This is not a leak, and conflating them would be a real bug.** SAFETY-003's
leak is water where it should not be, detected in the tray, and it already
blocks every mode. Unexpected flow is water moving through the *intended* path
with no authorisation. A siphon asserts no leak sensor until the pot overflows
onto the tray — by which point several litres have moved. Two invariants,
because they are two failures.

Latching until reboot follows M11-003's reasoning exactly: a fault that clears
itself lets a failing driver oscillate between working and not, delivering
unpredictable volumes. Add the explicit operator clear on top, because a reboot
alone does not mean anyone looked at the tube.

The idle detector must not cost a battery node its sleep. Sample the witness on
the existing telemetry cadence rather than continuously, and accept that an
unauthorised flow on a sleeping device is detected at the next wake — which is
still far earlier than an empty reservoir. Say so explicitly rather than
implying continuous vigilance the power budget cannot fund.

Publish the event through the buffered ring, not only live. An isolated device
that detects a siphon and then cannot reach the broker must still report it on
reconnect; a fault that only exists while the network is up is a fault that
misses the outages it matters most during.

## Acceptance criteria

- [ ] Continued flow past the settle window latches the actuator and asserts
      pump-off.
- [ ] Flow with no authorised actuation raises the same fault class.
- [ ] Both publish `flow.unexpected` with typed detail.
- [ ] The fault is buffered and replays after an isolation period.
- [ ] Subsequent commands are refused with `unexpected_flow`.
- [ ] The fault does not auto-clear; it needs a reboot **and** an explicit clear.
- [ ] The leak path is untouched and still fires on its own signal.
- [ ] The idle detector runs on the telemetry cadence and does not prevent sleep.

## Verification

```bash
cd firmware/esp32-node && cargo test delivery::unexpected
cargo test safety_024
```

## Tests required

- Continued flow after stop, at and past the settle boundary.
- Unauthorised flow from idle.
- The fault surviving an isolation period and replaying.
- Command refusal while latched.
- No auto-clear across a reboot alone.
- Leak and unexpected flow are distinguishable in the record.

## Documentation impact

- `docs/architecture/safety-invariants.md`: SAFETY-024's device half.
- `docs/architecture/failure-model.md`: siphon and stuck-valve rows.
- `docs/protocol/mqtt-v1.md` §5.4: the new event kind's detail.

## Files likely affected

```text
firmware/esp32-node/src/delivery/unexpected.rs
firmware/esp32-node/src/pump/fault.rs
docs/architecture/safety-invariants.md
docs/architecture/failure-model.md
```
