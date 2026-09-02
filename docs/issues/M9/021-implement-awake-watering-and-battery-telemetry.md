# Issue M9-021 — Keep the device awake for watering, and report battery state

**Milestone:** M9 · **PRD:** [PRD 090](../../prd/090-esp32-rust-firmware.md) · **Depends on:** M9-019, M9-020, M9-013

## Context

M9-019 gives the device a wake budget and a sleep call. A dose takes seconds and
must be watched continuously while it runs — the run guard, the leak sensor, and
the tank level are all live inputs during actuation
([PRD 110](../../prd/110-real-pump-and-safety-hardware.md)). A device that slept
in the middle of that would leave an energised pump with nothing watching it,
which the independent run guard would eventually catch, but only after the exact
kind of delay this project refuses to design in.

The operator also needs to know a battery device is running out, before it stops
waking rather than after.

## Goal

Watering holds the device awake until it is finished and reported, and battery
state reaches the edge as ordinary telemetry.

## Scope

- An awake-hold acquired before actuation and released after the
  `command.result` PUBACK, so the wake cycle cannot enter sleep while it is held
- `awake_budget_seconds` bounding only an *idle* wake; a held wake extends until
  the hold is released or `FIRMWARE_MAX_RUN_SECONDS` forces the run to end
- Post-dose ordering: pump de-energised → result published and acknowledged →
  buffered events flushed → sleep announced → sleep
- An interrupted dose reported at the next wake exactly as M9-013 already
  defines, with `wake_reason` distinguishing a power cut from a timer wake
- Battery voltage sampled through an ADC channel behind a `BatterySensor` trait,
  with a fake for host tests
- `battery_voltage` and `battery_percent` published in the ordinary telemetry
  batch, with `battery_percent` absent when no chemistry curve is configured
- A `battery_low` device event at a configured threshold
- Battery fields omitted entirely on hardware that cannot measure them —
  **absent, never zero, never a guess**

## Non-goals

- Any use of battery state in a watering decision. Explicitly forbidden by
  [ADR-018](../../adr/018-battery-and-deep-sleep-device-mode.md) §7: a low
  battery raises an alert and refuses nothing, and no power field is an input to
  the safety gate.
- Low-battery prediction or a remaining-life estimate (M13-016, on the edge,
  where there is history to trend).
- Charge-controller or solar telemetry (M14-009 planning only).
- Calibrating the ADC divider against a reference meter. That belongs with the
  other hardware measurement in M10-012.

## Dependencies

- M9-019
- M9-020
- M9-013

## Implementation notes

Make the hold a **guard object**, not a boolean. A flag that must be cleared on
every path is a flag that will be left set on some path, and a stuck hold means a
battery device that never sleeps again — a silent, expensive failure that looks
like nothing at all until the battery is flat. A guard whose `Drop` releases the
hold gets the error paths for free.

The hold gates sleep; it does not gate the run guard. `FIRMWARE_MAX_RUN_SECONDS`
still de-energises the pump on its independent timer regardless of anything the
wake cycle believes (F-090-37). The two mechanisms are unrelated and must stay
that way: one bounds how long water can flow, the other bounds when the device
may stop paying attention.

Publish the result **before** flushing buffered events and before announcing
sleep. A result that is still in flight when the radio goes down for fifteen
minutes turns a completed dose into an unknown one, and the edge's no-delivery
detection (M6-017) then has to reason about a device it cannot reach.

Battery percentage from voltage is chemistry-dependent and, for LiFePO4,
famously flat across most of the discharge curve. Publish `battery_voltage`
always and `battery_percent` only from a configured curve — a fabricated
percentage is worse than an absent one, and this is the same rule M10-006 applies
to an uncalibrated soil probe publishing `null`.

## Acceptance criteria

- [x] The device does not sleep while a dose is in progress, including when
      `awake_budget_seconds` is shorter than the dose.
- [x] The hold is released on every path, including a refused command, a failed
      publish, and a panic-free error return.
- [ ] `command.result` is acknowledged before the sleep announcement.
- [ ] `FIRMWARE_MAX_RUN_SECONDS` still ends a run independently of the hold.
- [ ] A power cut mid-dose yields a boot with the pump off and
      `status: "interrupted"`, `delivered_ml: null`, reported at the next wake.
- [ ] `battery_voltage` appears in the telemetry batch on capable hardware and is
      **absent** on hardware without the divider.
- [x] `battery_percent` is absent unless a chemistry curve is configured.
- [ ] `battery_low` is raised once per crossing, not once per sample.
- [x] No battery field appears in `IrrigationInputs` or in any argument to
      `validate_water_command`, checked structurally.

## Verification

```bash
cd firmware/esp32-node
cargo test --target x86_64-unknown-linux-gnu -p app awake_hold::
cargo test --target x86_64-unknown-linux-gnu -p app battery::
cargo build --release
grep -rn 'battery' src/app/irrigation/   # expect no matches
```

## Tests required

- Hold acquisition and release across every actuation outcome.
- Sleep refused while held; sleep permitted once released.
- Ordering of result, flush, announcement, sleep.
- Battery field absence on incapable hardware.
- Threshold-crossing event de-duplication.
- SCEN-117.

## Documentation impact

- [PRD 090](../../prd/090-esp32-rust-firmware.md) — awake hold and battery
  telemetry.
- [safety-invariants.md](../../architecture/safety-invariants.md) SAFETY-011
  cross-reference.

## Files likely affected

```text
firmware/esp32-node/src/app/awake_hold.rs
firmware/esp32-node/src/app/watering.rs
firmware/esp32-node/src/hal/battery.rs
```
