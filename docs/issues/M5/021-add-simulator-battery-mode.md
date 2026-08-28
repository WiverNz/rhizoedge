# Issue M5-021 — Add simulator battery power mode with announced sleep

**Milestone:** M5 · **PRD:** [PRD 050](../../prd/050-plant-model-and-recommendations.md) · **Depends on:** M5-019, M4-013, M2-017

## Context

The post-M4 battery correction teaches the edge to tell an announced sleep from a device that stopped
waking. Nothing can currently produce either, so none of it is testable — and
the firmware that would produce it is four milestones away.

The simulator exists for exactly this ([PRD 020](../../prd/020-device-simulator.md)):
it is the reference device, and it must never be *more permissive* than firmware.
Giving it a battery mode now means SCEN-110…SCEN-117 run with no hardware, and
M9-019 has a behavioural specification to match rather than to invent.

## Goal

A simulator that sleeps, wakes, and announces both, at accelerated virtual time.

## Scope

- `--power-mode always_on|battery` and `--wake-interval-seconds`, also settable
  through the retained config the edge publishes
- The wake cycle of [ADR-018](../../adr/018-battery-and-deep-sleep-device-mode.md)
  §5: wake, simulated peripheral power-on, simulated warm-up delay, sample,
  connect, publish, receive, sleep
- Simulated sleep: disconnect from the broker and stop all publication, honouring
  the accelerated clock so a 900-second interval is sub-second at scale 600
- The sleep announcement — retained `device.status` with `status: "offline"`,
  `reason: "sleeping"`, and the `power` block — published **before** disconnect
- Battery model publishing `battery_voltage` and `battery_percent`, draining per
  wake cycle and per pump-second, with a control-API hook to set the level
- Fault `miss-wake:<n>`: skip `n` wake cycles without announcing, so SCEN-111 has
  a producer
- Fault `sleep-without-announcing`: disconnect uncleanly from battery mode, so
  SCEN-112 has one
- Staying awake for the whole of an active watering cycle, including when the
  awake budget is deliberately shorter than the dose

## Non-goals

- Any offline evaluation or autonomous dose. M2's boundary is unchanged and
  `tests/single_actuation_path.rs` must stay green: no evaluator, no decision
  type, and no dose scheduler appears in `crates/device-simulator/src` before
  M6-019.
- Modelling real current draw or battery chemistry. The battery model exists to
  produce a plausible telemetry series and to be steered by tests, not to predict
  life; the energy budget is measured on hardware in M10-012.
- Deep-sleep RTC-domain semantics. The simulator has no reset reasons to
  distinguish; M9-019 owns that.

## Dependencies

- M4-013 (including its dated battery-compatibility report correction)
- M5-019 (remaining measurement/config contract scope)
- M2-017

## Implementation notes

Sleep is a **clean disconnect that publishes first**. The ordering is the whole
point: announcement, then disconnect. Publishing after disconnecting is not
possible, and disconnecting without announcing is what the
`sleep-without-announcing` fault does on purpose.

The retained sleep announcement replaces the retained online status, so a fresh
subscriber sees a sleeping device rather than a stale online one. It does **not**
replace the Last Will, which stays armed for the abnormal case. This is why an
unclean drop still produces `connection_lost` and therefore `isolated`.

Keep the awake window driven by work rather than by a wall-clock budget:
`awake_budget_seconds` bounds an *idle* wake, and an active watering cycle
extends it. A budget that could truncate a dose would be a way to strand an
energised pump, which the run guard would then have to catch — correct, but a
much worse design than not sleeping mid-dose.

Reuse M2-014's virtual time throughout. A test that sleeps for a real wake
interval is the anti-goal named in
[time-model.md](../../architecture/time-model.md) §8.

## Acceptance criteria

- [ ] `--power-mode battery` produces alternating sleep and wake cycles at the
      configured interval on the accelerated clock.
- [ ] Each wake publishes status and telemetry, and applies `edge.time`, config,
      and policy before sleeping again.
- [ ] The sleep announcement is retained, carries `reason: "sleeping"`, and is
      published before the disconnect.
- [ ] A fresh subscriber after a sleep sees the sleeping status and **nothing**
      on any `commands/*`, `telemetry`, `events`, or `time` topic.
- [ ] `miss-wake:2` produces an edge-side `isolated` state and
      `missed_wake_count == 2`. Note that `missed_wake_count` is *consecutive*
      misses and is reset by any successful wake, so the two misses must not be
      separated by one.
- [ ] `sleep-without-announcing` fires the Last Will and yields `isolated`.
- [ ] A dose delivered at wake completes without the device sleeping, even with
      `awake_budget_seconds` set below the dose duration.
- [ ] `--power-mode always_on` behaviour is unchanged; the whole M2 suite is
      green.
- [ ] `tests/single_actuation_path.rs` is still green.

## Verification

```bash
cargo test -p device-simulator power::
cargo test -p device-simulator --test single_actuation_path
RHIZO_REQUIRE_BROKER=1 cargo test -p device-simulator --test battery_wake_cycle
cargo test safety_021
```

## Tests required

- Wake-cycle timing at an accelerated scale.
- Announcement ordering, retention, and content.
- Both faults, each producing the edge state its scenario expects.
- Awake extension across a watering cycle.
- Always-on regression across the existing M2 suite.
- SCEN-110, SCEN-111, SCEN-112.

## Documentation impact

- [PRD 020](../../prd/020-device-simulator.md) — battery mode and the two faults
  recorded as an M5 addition.
- [simulator-strategy.md](../../testing/simulator-strategy.md).

## Files likely affected

```text
crates/device-simulator/src/power.rs
crates/device-simulator/src/battery.rs
crates/device-simulator/src/mqtt.rs
crates/device-simulator/src/faults.rs
crates/device-simulator/src/control.rs
```
