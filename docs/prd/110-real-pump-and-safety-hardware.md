# PRD 110 — Real Pump and Safety Hardware

**Milestone:** M11 · **Status:** PLANNED · **Depends on:** M10

## Summary

Connect a real peristaltic pump, a reservoir level sensor, and a leak sensor.
Calibrate delivery, verify every hard limit against physical water, and complete
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
6. Every SAFETY invariant re-verified with real water.

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

Every stage gates the next. See
[hardware-in-the-loop.md](../testing/hardware-in-the-loop.md).

## Functional requirements

### Pump

| ID | Requirement |
|---|---|
| F-110-01 | Real adapter implements the **existing** `Pump` trait unchanged |
| F-110-02 | GPIO drives a MOSFET or relay module; the pump has its own supply, never the ESP32 rail |
| F-110-03 | **The gate is pulled down in hardware** so an un-driven pin is pump-off — covering reset and the bootloader window |
| F-110-04 | Duration computed as `effective_ml / ml_per_second`, clamped to `FIRMWARE_MAX_RUN_SECONDS` |
| F-110-05 | A run-duration guard on a **separate timer/task from MQTT**; a hung MQTT task cannot leave the pump energised |
| F-110-06 | Hardware watchdog enabled; a watchdog reset leaves the pump off |
| F-110-07 | Overrun detected → pump `faulted`, further commands refused until reboot, `pump_fault` published |
| F-110-08 | Actual run duration measured and reported in `command.result` |

### Calibration

| ID | Requirement |
|---|---|
| F-110-10 | `command.calibrate` runs the pump for a fixed duration |
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

- [ ] HIL-1 passes: the pump line never asserts across 20 resets, a watchdog
      reset, and 10 mid-boot power cuts.
- [ ] Calibration: five runs, σ < 5 % of the mean, recorded.
- [ ] A 40 ml request delivers 40 ml ± 10 %, measured.
- [ ] `requested_ml: 10000` published directly to the broker delivers no more
      than `FIRMWARE_MAX_ML_PER_RUN` — **measured in a cup**.
- [ ] Wetting the leak sensor stops an in-progress dose within 1 second.
- [ ] With a leak asserted, `POST /plants/{id}/water` returns 409 and the pump
      stays silent.
- [ ] Clearing the lockout with the sensor still wet returns 409.
- [ ] Draining the reservoir prevents watering; the device refuses independently.
- [ ] Disconnecting the tank sensor prevents watering.
- [ ] A simulated relay-stuck condition de-energises at
      `FIRMWARE_MAX_RUN_SECONDS`.
- [ ] HIL-6 full dry cycle completes correctly into a measuring cup, then into soil.
- [ ] `docs/testing/hil-runs/` contains a complete record.

## Dependencies

- M10 (real soil readings to close the loop).
- M9 (firmware, trait boundaries, boot safety).
- Hardware: peristaltic pump, MOSFET/relay driver module with gate pull-down,
  external pump supply, silicone tubing, reservoir, tank sensor, leak sensor,
  measuring cup, multimeter, towels, and an in-line power switch.

## Open questions

1. **Relay vs MOSFET.** MOSFET preferred: no mechanical contacts to weld, faster
   turn-off, and a gate pull-down is trivial. A relay is acceptable if the pump
   needs AC. Decided at purchase.
2. **Tank sensor type.** A float switch is cheapest and most reliable but binary;
   ultrasonic gives a real percentage but is sensitive to reservoir geometry and
   foam. Starting with a float switch and documenting the coarseness.
3. **Whether a flow meter is worth adding early.** It would make delivery
   verification direct rather than inferred. Deferred to M14 — inexpensive flow
   meters are unreliable at these very low flow rates, so it may not help.

## Future work

- Flow-meter verification (M14).
- Multiple pumps per device (M13).
- Solenoid valves and zones (M14).
- Automatic calibration from the pot scale (post-V1).
