# Hardware-in-the-Loop Testing

Applies from M9 onward. These tests are manual, checklist-driven, and performed
in a fixed order. Each stage gates the next.

**The governing rule:**

> The first automatic watering test targets a measuring cup, never a plant.

A plant that is drowned is not a failed test case; it is a dead plant. There is
no version of this work where a real specimen is the first subject.

---

## 1. Bench setup

```text
        ┌──────────────┐
        │  ESP32-C3    │
        └──┬────┬───┬──┘
           │    │   │
   soil probe   │   └── pump driver ──► peristaltic pump
   (in a cup    │                            │
    of soil)    │                       silicone tubing
                │                            │
        tank sensor ──► reservoir ◄──────────┘
        leak sensor  (on the bench surface)
                          │
                     measuring cup  ◄── pump outlet during stages 1–4
```

Required for every stage:

- an absorbent towel under everything
- the reservoir on a tray
- a physical means of cutting pump power that does not depend on firmware — a
  switch in the pump's power line, within arm's reach

That last item is not optional. It is the only safety mechanism in the room that
cannot have a software bug.

---

## 2. Stage gate order

```text
HIL-1 boot safety      ──► HIL-2 telemetry ──► HIL-3 pump calibration
   ──► HIL-4 command safety ──► HIL-5 lockouts ──► HIL-6 dry cycle
   ──► HIL-7 supervised plant
```

A stage is not attempted until the previous one passes completely. A failure
sends you back to the bench, not forward with a note.

---

## HIL-1 — Boot safety (no water in the system)

**Proves** SAFETY-011. **Prerequisite:** tubing disconnected, reservoir empty.

- [ ] Power on with a multimeter on the pump driver input. **The pump line never
      goes active during boot**, including the bootloader window before `main`
      runs.
- [ ] Flash while powered. The pump does not pulse during flashing.
- [ ] Press reset repeatedly, 20 times. The pump never actuates.
- [ ] Trigger a watchdog reset (a deliberate hang). The pump line goes inactive
      and stays inactive.
- [ ] Pull power mid-boot, 10 times. No actuation on any restart.

**Pass criterion:** the pump driver input is never asserted except by an explicit
validated command. If the pump so much as twitches during a reset, the pull-down
on the driver gate is wrong — fix the hardware before continuing.

---

## HIL-2 — Telemetry (still no water)

**Proves** the M9/M10 telemetry path end to end.

- [ ] Device connects to Wi-Fi and appears online in the edge API.
- [ ] Edge time sync applies; status reports `clock_synced: true`. Record how long this takes after connect.
- [ ] Soil readings appear and are physically plausible: probe in dry air reads
      low, in a glass of water reads high, in damp soil reads between.
- [ ] Retained status survives an edge restart.
- [ ] LWT fires within the keepalive window when power is cut.
- [ ] Retained config is received and `applied_config_version` echoes it.
- [ ] Disconnect Wi-Fi for 10 minutes. The device reconnects with backoff and
      does **not** flush a large telemetry backlog.
- [ ] Withhold `edge.time` (stop the edge, or block the `time` topic). Confirm
      `clock_synced: false` is reported after the max age and telemetry continues.

---

## HIL-3 — Pump calibration (water, into a measuring cup)

**Prerequisite:** outlet in a measuring cup. HIL-1 passed.

- [ ] Prime the line manually; confirm no leaks at any joint.
- [ ] `POST /devices/{id}/commands/calibrate { "run_seconds": 10 }` five times,
      recording delivered volume each run.
- [ ] Compute mean and standard deviation of `ml/second`.
- [ ] Store the mean as `pump.ml_per_second` in the device config.
- [ ] Verify: request 40 ml, measure the delivered volume.

**Pass criterion:** delivered volume within ±10 % of requested across five runs,
and standard deviation below 5 % of the mean. Higher variance means an air lock,
a partially occluded tube, or a failing pump head — investigate rather than
averaging it away.

Record the calibration date. Peristaltic tubing hardens and flow degrades; this
is expected and is what `calibration_drift` detection exists for.

---

## HIL-4 — Command safety (measuring cup)

**Proves** SAFETY-002, SAFETY-007. Publish directly to the MQTT command topic,
bypassing the edge entirely — this stage tests the device's independent veto.

- [ ] `requested_ml: 10000` → clamped to `FIRMWARE_MAX_ML_PER_RUN` or rejected.
      **Measure the cup.** Never more than the hard limit.
- [ ] `expires_at` in the past → rejected with `expired`, pump silent.
- [ ] Same `command_id` published three times → **one** actuation, three results,
      two of them the stored replay.
- [ ] `requested_ml: -5` and `requested_ml: 0` → rejected as malformed.
- [ ] Commands totalling more than `FIRMWARE_MAX_DAILY_ML` → rejected with
      `over_daily_max` once the cap is reached.
- [ ] Power-cycle the device, then repeat a previously executed `command_id` →
      still deduplicated (the NVS ring survived).
- [ ] Withhold `edge.time`, reboot, then send a valid command → rejected with
      `clock_unsynced`.

**Pass criterion:** every one of these behaves identically to the simulator. Any
divergence is a bug in the shared validator's integration, and it invalidates
every simulator-based safety test until resolved.

---

## HIL-5 — Safety hardware lockouts (measuring cup)

**Proves** SAFETY-003, SAFETY-004.

- [ ] Wet the leak sensor → `Lock(Leak)` within one control tick; a queued
      automatic dose does not run.
- [ ] With the leak still wet, `POST /plants/{id}/water` → **409**.
- [ ] Attempt to clear the lockout while still wet → **409**.
- [ ] Dry the sensor, clear explicitly → lockout clears; watering possible again.
- [ ] Trigger a leak **during** an active dose → the pump stops. Measure how much
      was delivered and confirm it is recorded.
- [ ] Drain the reservoir below minimum → `Lock(TankLow)`; the device refuses
      independently of the edge.
- [ ] Disconnect the tank sensor entirely → lockout (unknown ≠ permitted).
- [ ] Disconnect the soil probe → `Lock(SensorFault)`; automatic watering
      disabled; **manual watering still permitted** and still blocked by leak.

---

## HIL-6 — Full dry cycle (measuring cup, no plant)

Run the complete automatic cycle against a cup of soil with the outlet in a
**separate measuring cup**, so the soil does not actually receive the water.
This decouples "the control logic behaves" from "the water goes somewhere
sensible".

- [ ] Dry the soil sample (a heat lamp or simply time) until below `target_min`.
- [ ] Confirm the state sequence matches SCEN-002.
- [ ] Confirm each dose's delivered volume in the cup matches the requested
      volume within calibration tolerance.
- [ ] Confirm the absorption wait is honoured in real time.
- [ ] Confirm the cycle stops at `max_doses_per_cycle` and locks with
      `MaxDosesReached` if moisture never recovers.
- [ ] Confirm the rolling daily total is respected across cycles.

Then repeat with the outlet **in the soil**, so absorption is real:

- [ ] Moisture rises after a dose; the overshoot-then-settle behaviour resembles
      the simulator's model.
- [ ] The cycle terminates on recovery rather than on the dose limit.

---

## HIL-7 — Supervised plant

Only after HIL-1 through HIL-6 pass completely.

- [ ] Choose a **robust, inexpensive, easily replaced plant.** Not a specimen,
      not a gift, not something irreplaceable.
- [ ] Set `max_daily_ml` to roughly half the value you believe is correct.
- [ ] Set `dose_ml` small — 20–30 ml.
- [ ] Run with `auto_watering_enabled = false` for **at least one week**,
      generating recommendations only. Compare every recommendation against your
      own judgement.
- [ ] Only then enable automatic watering, and only while you are home and can
      observe.
- [ ] Watch the first three automatic cycles directly.
- [ ] After a month, review the watering history against the plant's condition
      before raising any limit.

**Pass criterion:** the plant is visibly healthy after one month, the watering
history is plausible, and no unexplained lockout occurred.

---

## 3. Recurring checks

| Check | Frequency |
|---|---|
| Pump calibration re-verification | quarterly, or after tubing replacement |
| Leak sensor function test | monthly |
| Tank sensor accuracy | monthly |
| Soil probe against a reference reading | quarterly |
| Tubing inspection for kinks, algae, hardening | monthly |
| Reservoir cleaning | monthly |

Peristaltic tubing hardening is the most common slow failure: flow degrades
gradually and the system silently under-waters while believing it delivered the
requested volume. The `calibration_drift` detector catches this only when a
scale is fitted; without one, the quarterly re-verification is the only defence.

---

## 4. Recording results

Each HIL run is recorded in `docs/testing/hil-runs/YYYY-MM-DD.md` with:

- firmware version and git commit
- hardware revision, pump model, sensor models
- calibration figures with all five raw measurements
- every checklist item with pass/fail
- photographs of the bench setup
- anything surprising, however minor

The calibration figures in particular are the historical record that makes drift
detectable across months.

---

## 5. What is deliberately not tested with hardware

- **Cloud sync** — no hardware dependency; covered by M7/M8.
- **Long-term soil probe accuracy** — requires laboratory reference measurement
  and is out of V1 scope.
- **Pump lifetime** — a manufacturer specification, not something to discover.
- **Sustained flood behaviour** — the whole design exists to prevent it; testing
  it would mean deliberately defeating SAFETY-007, which is not a test, it is a
  flood.

## 6. Bench measurements — not HIL gates

M10-011 and M10-012 need the same bench and the same board, and their results are
recorded alongside HIL runs in `docs/testing/hil-runs/`. They are deliberately
**not** numbered as HIL stages, because a HIL stage gates the next stage and
these gate a *claim*:

| Measurement | Issue | Gates |
|---|---|---|
| Sensor power-on → time until stable usable reading | M10-011 | the configured `sensor_warmup_ms`, and an input to the energy budget |
| Complete-system sleep current and wake-cycle energy | M10-012 | **every autonomy figure in the repository** — until it exists, all of them stay labelled targets ([ADR-018](../adr/018-battery-and-deep-sleep-device-mode.md) §8) |

Neither involves water, and neither is on the HIL-1…HIL-7 critical path. M10-012
does need an instrument the HIL bench does not otherwise require: sleep current
is microamps while wake current is hundreds of milliamps within the same second,
and a handheld multimeter either burdens the supply at the low end or averages
the peaks away at the high end.
