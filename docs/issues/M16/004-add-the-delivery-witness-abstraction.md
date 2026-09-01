# Issue M16-004 — Add the delivery witness abstraction and fakes

**Milestone:** M16 · **PRD:** [PRD 160](../../prd/160-verified-watering.md) · **Depends on:** M16-003

## Context

M9-005 established the pattern: hardware behind traits, with fakes, so the
firmware's logic is testable on a host and a real driver is a later, smaller
change. The witness is a new instance of that pattern, and getting the trait
right matters more than the first implementation — an inline flow meter must be
able to arrive later as a driver rather than as a redesign.

## Goal

`DeliveryWitness`, its health model, `NullWitness`, and a controllable fake.

## Scope

- `trait DeliveryWitness` in the firmware workspace:
  `cumulative_ml(&mut self) -> Option<f32>` — monotonic since the last baseline,
  `None` when unusable — and `health(&self) -> WitnessHealth`.
- `WitnessHealth`: `Healthy`, `Degraded { reason }`, `Faulted { reason }`,
  `Absent`.
- `NullWitness` — the no-witness-fitted case, always `Absent`, always `None`.
- `FakeWitness` — scriptable for tests: normal delivery, no flow, slow start,
  partial, over-delivery, residual flow, non-monotonic, non-finite, and
  mid-dose failure.
- Wiring into the device's hardware bundle, defaulting to `NullWitness`.

## Non-goals

- The reservoir scale driver. M16-005.
- The execution state machine that consumes it. M16-007.
- A flow-meter implementation. Reserved, not built.

## Dependencies

- M16-003

## Implementation notes

**Cumulative, not rate.** A cumulative volume since a baseline is what both a
scale and a pulse counter naturally produce, it is robust to a missed poll, and
it makes the target-volume stop a comparison rather than an integration. A
rate-based trait would force the scale implementation to differentiate and then
force the state machine to re-integrate, adding error at both ends.

`Option<f32>`, not `f32`. A witness that cannot currently answer must say so, and
`0.0` is a specific, dangerous lie — it is indistinguishable from "no water
moved", which is the exact conclusion that latches an actuator and calls a
person.

Monotonicity is the trait's contract and must be enforced by the implementation,
not assumed by the caller: a decrease is a fault, not a negative delivery. The
fake must be able to violate it so M16-007 can be tested against a witness that
does.

`NullWitness` is the default so that the entire feature is opt-in at the
hardware level. A node built before M16 keeps behaving identically, which is
F-160-19 expressed in the constructor.

## Acceptance criteria

- [ ] The trait is cumulative, returns `Option`, and exposes health separately.
- [ ] `NullWitness` is the default and always answers `Absent` / `None`.
- [ ] `FakeWitness` can produce every scripted failure listed in scope.
- [ ] A decreasing cumulative reading is reported as a fault, never as a negative
      volume.
- [ ] The trait has no `std` dependency and compiles for the firmware target.
- [ ] No production code outside the witness module reads a raw sensor value.

## Verification

```bash
cd firmware/esp32-node && cargo test witness::
cd firmware/esp32-node && cargo build --target riscv32imc-esp-espidf
```

## Tests required

- Each scripted fake behaviour.
- Monotonicity enforcement.
- `NullWitness` leaves every existing firmware test unchanged.

## Documentation impact

- PRD 160 §Interfaces, if the trait shape deviates.
- `docs/adr/008-shared-code-simulator-and-firmware.md`, if the fake is shared.

## Files likely affected

```text
firmware/esp32-node/src/witness/mod.rs
firmware/esp32-node/src/witness/null.rs
firmware/esp32-node/src/witness/fake.rs
firmware/esp32-node/src/hardware.rs
```
