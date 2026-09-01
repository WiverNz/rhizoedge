# Issue M16-007 — Implement the device-side delivery execution state machine

**Milestone:** M16 · **PRD:** [PRD 160](../../prd/160-verified-watering.md) · **Depends on:** M16-006

## Context

This is where the feature becomes safety-relevant, and it belongs on the device
for one reason: latency. A no-flow condition that must stop a pump in under a
second cannot make a round trip to the edge, so the observation, the decision,
and the actuator all have to be on the same side of the network.

The layer already exists — M11-002's independent run guard, M11-003's latched
fault, F-110-33's one-second leak stop. This extends it; it does not sit beside
it.

## Goal

The execution state machine of PRD 160 §State model: baseline, startup
detection, target stop, settle check, and classification.

## Scope

- States: `Accepted` → `BaselineTaken` → `Actuating` → `Stopping` → `Settling` →
  a terminal outcome.
- Startup: no observed flow within `FLOW_START_TIMEOUT_MS` stops the pump and
  yields `NoFlow`.
- Target: reaching the target measured volume stops the pump, never later than
  the calibrated run duration.
- Settle: flow must cease within `FLOW_SETTLE_MS` after shutdown.
- Classification through the shared `rhizo_domain::delivery::classify`.
- The `delivery` object attached to the `command.result`.
- The in-flight NVS record extended with the baseline, so a reboot mid-dose can
  say whether it still knows anything.

## Non-goals

- Unexpected and continued flow outside a dose. M16-008, because it is a
  different trigger with a different severity.
- Any edge-side accounting. M16-010 and M16-011.
- Retrying anything. ADR-020 §9.

## Dependencies

- M16-006

## Implementation notes

**Order, and it is not negotiable.** The gate's step 13 — persist the in-flight
record to NVS — stays first. The baseline is taken **after** it and **before**
step 14. A baseline written before the in-flight record would leave a dose that
actuated with no record of having done so, which is the failure step 13 exists to
prevent.

**The witness may only stop a pump earlier, never run one longer.** The target
stop is `min(target_reached, calibrated_run_ms)`, and the independent run guard
remains the outer bound regardless. Write the comparison so that a witness
reporting nonsense — a stuck cumulative value, a reading that never reaches the
target — cannot extend the run by a single millisecond. A test that drives the
state machine with a witness that always returns `Some(0.0)` must still stop at
the calibrated duration.

**`None` from the witness is not zero.** A witness that goes unreadable mid-dose
degrades the record to `Actuated` and lets the calibrated duration finish the
dose; it does not conclude `NoFlow`. `NoFlow` is a *measured* condition and
requires a working witness saying nothing moved.

Reuse `classify` rather than re-deriving the outcome here. There is one
classifier, in the pure crate, for the same reason there is one
`validate_water_command`: a second implementation of the rules makes every
host-side test worthless.

The existing faults matter more than the new ones: a leak asserting mid-dose
still stops the pump within one second (F-110-33) and yields
`LeakDuringDelivery`; the run guard firing still yields `PumpTimeout`. Neither
is replaced by a witness check.

## Acceptance criteria

- [ ] The baseline is taken after the NVS write and before actuation.
- [ ] No flow within the startup timeout stops the pump and yields `NoFlow`.
- [ ] The target stop can only shorten a run, never lengthen one.
- [ ] A witness stuck at `Some(0.0)` still stops at the calibrated duration.
- [ ] A witness returning `None` mid-dose degrades to `Actuated` and does not
      yield `NoFlow`.
- [ ] Flow continuing past the settle window is not resolved here — it is raised
      to M16-008's path.
- [ ] The independent run guard and watchdog are unchanged and still outermost.
- [ ] Leak-during-dose and run-guard outcomes are unchanged in timing.
- [ ] Outcomes come from `rhizo_domain::delivery::classify`, with no second
      implementation.
- [ ] A reboot mid-dose can distinguish "baseline still valid" from "baseline
      lost".

## Verification

```bash
cd firmware/esp32-node && cargo test delivery::
cargo test -p rhizo-domain delivery::classify
cargo test safety_007
cargo test safety_011
```

## Tests required

- Every terminal outcome, driven by `FakeWitness`.
- The run-extension property: no witness behaviour lengthens a run.
- Reboot with a valid baseline, and reboot with a lost one.
- Leak and run-guard paths unchanged.

## Documentation impact

- `docs/protocol/mqtt-v1.md` §5.8: the baseline step, documented between steps
  13 and 14 as a device-internal action with no wire representation.
- PRD 160 §State model, if the machine deviates.

## Files likely affected

```text
firmware/esp32-node/src/delivery/machine.rs
firmware/esp32-node/src/pump/mod.rs
crates/domain/src/delivery/classify.rs
docs/protocol/mqtt-v1.md
```
