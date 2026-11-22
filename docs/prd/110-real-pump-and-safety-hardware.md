# PRD 110 — Real Pump and Safety Hardware

## Prototype scope — supersession, 2026-11-22

M11 is **READY** for adapter implementation against existing firmware; it
does not wait for all of M10. Closed-loop watering needs real valid readings.
Use the requested DFR0523 with its integrated driver, not an assumed two-wire
pump plus generic MOSFET. Resolve actual pulse timing and electrical off-state
before energizing the motor; a pull-down or static LOW is not sufficient proof.
Do not add driver hardware unless an observed safety need requires it.

Required now: independent duration guard, conservative per-dose/cumulative volume
and cooldown/frequency limits, calibration for small doses, fault/stop handling,
physical required tank/leak inputs, and safe startup/reset/watchdog/control loss.
Keep absent or invalid required inputs fail-closed. Perform cup checks then one
supervised plant experiment, with reachable power disconnect. No fixed repetition
matrix, month-long plant trial or exhaustive invariant re-verification is required.
F-110-11/13's fixed five-run statistical campaign is deferred; obtain enough small
measured doses to establish a conservative usable conversion and check it once.
Unstable or uncontrolled delivery must be fixed, not averaged away.

The remaining detailed design is reference scope; current prototype acceptance
and testing policy supersede its full-campaign gates. Volume remains an estimate
without a delivery witness; M16 is deferred and is not needed for this experiment.

See [ROADMAP](../../ROADMAP.md) and [testing policy](../testing/strategy.md).


**Milestone:** M11 · **Status:** READY · **Depends on:** Existing M9 interfaces; M10 readings before closed-loop watering

## Summary

Connect a real peristaltic pump, a reservoir level sensor, and a leak sensor.
Calibrate small doses, check essential limits with physical water, and complete
the first hardware demo.

This is the milestone where a software defect becomes a wet floor.

## Problem

Every safety invariant so far has been proven against a simulator and against
firmware with fake adapters. Real hardware introduces failure modes that no
amount of software testing produces: a relay that welds shut, a tube that kinks,
a pump that runs dry, a float switch that sticks.

## Goals

1. A pump driver behind the existing `Pump` trait.
2. Calibrated delivery within documented tolerance.
3. Tank level and leak sensors integrated.
4. A run-duration guard **independent of the MQTT task**.
5. Boot-safe and fail-closed behaviour verified electrically.
6. Essential protections demonstrated on the actual prototype; deeper validation later.

## Non-goals

- Multiple pumps or zones (M13/M14).
- Flow-meter-based verification (M14) — delivery is verified by calibration and,
  where a scale exists, by weight.
- Fertiliser dosing. Permanently out of V1 scope.
- Automatic recovery from a pump fault. A hardware fault needs hands.

## User/system flows

```text
wire pump + driver + external supply → HIL-1 boot safety → HIL-3 calibration
   → store ml_per_second in device config
   → HIL-4 command safety (measuring cup)
   → HIL-5 lockouts (leak, tank)
   → HIL-6 full dry cycle (measuring cup, then soil)
   → HIL-7 supervised plant
```

Essential electrical/control checks gate actuation, cup checks precede plant
use, and the extended campaign is deferred. See
[hardware-in-the-loop.md](../testing/hardware-in-the-loop.md).

## Functional requirements

### Pump

| ID | Requirement |
|---|---|
| F-110-01 | Real adapter implements the **existing** `Pump` trait unchanged |
| F-110-02 | DFR0523 integrated driver receives verified control pulses; pump supply is separate from the ESP32 rail |
| F-110-03 | The actual driver must remain off with un-driven/absent control during startup/reset; verify electrical off-state rather than assuming a GPIO pull-down proves it |
| F-110-04 | Duration computed as `effective_ml / ml_per_second`, clamped to `FIRMWARE_MAX_RUN_SECONDS` |
| F-110-05 | A run-duration guard on a **separate timer/task from MQTT**; a hung MQTT task cannot leave the pump energised |
| F-110-06 | Hardware watchdog enabled; a watchdog reset leaves the pump off |
| F-110-07 | Overrun detected → pump `faulted`, further commands refused until reboot, `pump_fault` published |
| F-110-08 | Actual run duration measured and reported in `command.result` |

### Calibration

| ID | Requirement |
|---|---|
| F-110-10 | `command.calibrate` maps duration to requested/effective volume and runs only after the same full shared device actuation gate used by watering; it has no bypass or subset validator |
| F-110-11 | Five runs, mean and standard deviation recorded |
| F-110-12 | `ml_per_second` stored in device config, versioned |
| F-110-13 | Accepted only if σ < 5 % of the mean; higher variance is a wiring or hardware problem, not a number to average |
| F-110-14 | Calibration date recorded; drift detection where a scale exists |

### Tank level

| ID | Requirement |
|---|---|
| F-110-20 | `TankSensor` adapter (float switch, ultrasonic, or resistive) |
| F-110-21 | Level published as a percentage; **absent or failed reads publish `null`** |
| F-110-22 | Device refuses commands at or below `tank.min_percent` independently of the edge |
| F-110-23 | `null` is treated as Unknown → refusal, never as permission |
| F-110-24 | A float switch reports 0 or 100 only; documented as coarse rather than interpolated |

### Leak sensor

| ID | Requirement |
|---|---|
| F-110-30 | `LeakSensor` adapter (conductive pad or optical) |
| F-110-31 | A leak transition is published **immediately**, not at the next telemetry interval |
| F-110-32 | The device refuses all water commands while a leak is asserted |
| F-110-33 | A leak asserted **during** a dose stops the pump within 1 second |
| F-110-34 | Sensor absent or failed → `null` → Unknown → refusal |
| F-110-35 | Leak state requires an explicit operator reset after the signal clears |

### Verification

| ID | Requirement |
|---|---|
| F-110-40 | Every HIL stage completed and recorded in `docs/testing/hil-runs/` |
| F-110-41 | Delivered volume measured physically for every hard-limit test |
| F-110-42 | A physical pump power cut-off switch is present during all testing |

## Interfaces

Traits unchanged from M9. Device config gains:

```json
"pump": {
  "ml_per_second": 8.2,
  "enabled": true,
  "calibrated_at": "2026-09-14T10:22:00Z"
},
"tank": { "kind": "float" | "ultrasonic" | "resistive", "min_percent": 15.0,
          "empty_raw": 320, "full_raw": 2880 },
"leak": { "kind": "conductive", "active_low": true }
```

No MQTT protocol change. No edge change. The whole point.

## Data model

No schema change. `watering_events.delivered_ml` now carries a physically
measured value; `commands.status` gains real `interrupted` and `failed`
occurrences rather than injected ones.

## State model

```text
Pump: Idle ──command accepted──► Running(deadline) ──completed──► Idle
        ▲                            │
        │                    deadline exceeded
        │                            ▼
        └──── reboot only ────── Faulted   (refuses all commands)

Leak: Clear ◄── explicit reset (only when signal absent) ──┐
        │                                                   │
        └── signal asserted ──► Detected ──signal clears──► AwaitingReset
```

`AwaitingReset` is a distinct state from `Clear`. A leak that dries out does not
silently re-enable watering — SAFETY-003 requires a human to look at the floor.

## Failure modes

| Failure | Detection | Behaviour |
|---|---|---|
| Pump runs, no water (air lock, kink, dry line) | no moisture and no weight response after 2 doses | `Lock(NoDeliveryDetected)`, explicit clear required |
| Relay welded / MOSFET shorted | run timer exceeded | de-energise, `Faulted`, refuse until reboot |
| Reservoir empty mid-dose | tank telemetry | dose completes (already short), next refused |
| Tube kinked | same as no-delivery | as above |
| Leak during a dose | leak sensor | pump stops within 1 s, partial delivery reported |
| Leak sensor stuck asserted | permanent lockout | correct behaviour; operator investigates |
| Leak sensor disconnected | `null` | Unknown → refusal |
| Float switch stuck full | tank never reads low | **undetectable in V1** — mitigated by the daily cap and the reservoir being physically small |
| Pump supply fails | pump runs 0 ml | no-delivery detection |
| Calibration drift (tubing hardening) | quarterly re-verification; automatic where a scale exists | `calibration_drift` warning |

The stuck-float row is an honest gap: a level sensor that always reads "full"
cannot be distinguished from a full tank. The mitigations are architectural —
`FIRMWARE_MAX_DAILY_ML` bounds total delivery, and a physically small reservoir
bounds the worst case regardless of what any sensor says. Choosing a reservoir
no larger than a day's safe delivery is a **deployment recommendation**, not a
software feature.

## Safety implications

M11 is where SAFETY-003, -004, -007, and -011 are verified against physics.

| Invariant | Verified by |
|---|---|
| SAFETY-003 | HIL-5: wet the sensor, confirm automatic **and** manual refusal, confirm reset requires a dry sensor |
| SAFETY-004 | HIL-5: drain the reservoir, confirm device-side refusal; disconnect the sensor, confirm refusal |
| SAFETY-007 | HIL-4: publish `requested_ml: 10000` directly to the broker and **measure the cup** |
| SAFETY-011 | HIL-1: 20 resets, a watchdog reset, and 10 mid-boot power cuts with a multimeter on the pump line |

Three requirements are load-bearing in a way software cannot compensate for:

- **F-110-03** — hardware pull-down. If the pump is on when the pin floats, no
  firmware correctness helps, because the dangerous window is before any code
  runs.
- **F-110-05** — the run guard must not share a task with MQTT. A hung network
  task with the pump energised is the worst reachable state in this system.
- **F-110-42** — a physical cut-off switch during testing. It is the only safety
  mechanism in the room that cannot have a bug.

## Observability

```text
watering_delivered_ml_total{mode}
watering_failures_total{reason="no_delivery|pump_fault|tank_low|leak"}
pump_run_duration_seconds
pump_calibration_error_ratio        gauge, where a scale exists
```

Events: `pump_fault`, `no_delivery`, `leak`, `calibration_drift`,
`tank_low`. `leak` is the highest-severity event the system produces.

Every dose logs at INFO with requested, effective, delivered, duration, and
whether it was clamped.

## Testing strategy

- Host unit: duration arithmetic and clamping; run-guard timeout logic;
  leak-during-dose interruption; `AwaitingReset` transitions; tank
  interpolation and `null` handling.
- Integration with fakes: leak asserted mid-dose stops the pump; tank `null`
  refuses; overrun sets `Faulted`.
- **Hardware:** the full HIL-1 → HIL-7 sequence, each stage gating the next,
  every result recorded.
- Every hard-limit test measured with a **measuring cup**, not inferred from
  logs.

## Acceptance criteria

- [ ] DFR0523 signal timing/off-state and startup/reset/watchdog/control-loss behavior checked before motor enable; safe wiring and supply verified.
- [ ] Small cup doses establish a conservative calibration and actual runtime is reported.
- [ ] Runtime, per-dose/cumulative volume and frequency limits hold; MQTT loss cannot defeat the independent run guard.
- [ ] Required tank/leak inputs work physically; invalid/missing/stale inputs refuse watering and a leak stops an active dose.
- [ ] Manual stop and reachable pump-power disconnect work; restart does not replay watering or reset budgets permissively.
- [ ] One supervised real-plant cycle succeeds through the local system and minimal interface; short result and limitations recorded.

This is prototype evidence only. Extended HIL matrices and production validation
remain deferred; do not relabel an entire invariant physically verified from one
observed case.

## Dependencies

- Existing M9 pump/config/command interfaces; real M10 readings before closed-loop actuation.
- DFR0523 with its integrated driver, verified safe power/wiring, tubing, reservoir, a way to measure small doses and an accessible power disconnect.
- Actual required tank/leak inputs under the existing safety contract; do not fake healthy readings.
- Check the local inventory before choosing equipment. No generic extra MOSFET, flow meter or reservoir scale is required by this plan.

## Open questions

Resolve the DFR0523 control waveform and signal-loss behavior against its
actual driver before power. Do not assume a generic MOSFET GPIO command or
copy conflicting frequency/pulse-width specifications. Confirm safe connectors
and a means of measuring small doses before the cup test. Any concrete safety
gap blocks that operation, not sensor implementation.

## Future work

- Direct delivery verification (M16) — a reservoir load cell, an explicit
  outcome taxonomy, and unexpected flow as a fault
  ([PRD 160](160-verified-watering.md)). M16 depends on this milestone's pump
  and HIL bench, and adds **HIL-8** after HIL-6.
- Multiple pumps per device (M13).
- Solenoid valves and zones (M14).
- Automatic calibration from the pot scale (post-V1).
