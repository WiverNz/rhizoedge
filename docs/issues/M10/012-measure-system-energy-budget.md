# Issue M10-012 — Measure complete-system sleep current and the wake-cycle energy budget

**Milestone:** M10 · **PRD:** [PRD 100](../../prd/100-real-soil-sensor.md) · **Depends on:** M10-011

## Context

[ADR-018](../../adr/018-battery-and-deep-sleep-device-mode.md) §8 makes one
commitment about power: **no autonomy figure is stated as a specification until
the complete-system figure has been measured.** This issue is that measurement,
and it gates the target in [PRD 140](../../prd/140-field-readiness.md).

The distinction it exists to enforce:

```text
ESP32-C3 chip deep-sleep current      a datasheet figure for the die alone
complete board/system sleep current   what the assembled thing actually draws
```

The second includes regulator quiescent current, load-switch and level-shifter
leakage, the RS485 transceiver's off-state, pull-ups, the sensor rail's residual,
and any indicator LED. It is routinely an order of magnitude or more above the
first, and it is the only one that predicts battery life.

## Goal

Measure what the assembled device actually consumes, and build the energy budget
from measurements rather than from datasheets.

## Scope

- Complete-system sleep current, measured on the assembled reference device with
  the sensor and RS485 attached, at a stated supply voltage
- Chip deep-sleep current measured or cited **separately**, and labelled as such,
  so the two are never conflated in any later document
- Wake-cycle energy, integrated over a full cycle and broken into phases:
  rail-on and warm-up (using M10-011's figure), sampling, Wi-Fi association,
  MQTT connect and publish, receive window, sleep entry
- The same for a wake that includes a watering cycle
- At least 20 cycles, reported as a distribution — Wi-Fi association time is the
  most variable term and a median hides it
- The budget model, written down with every input labelled measured or assumed:

  ```text
  daily charge = wakes_per_day × energy_per_wake + 86400 s × I_sleep
  life_days    = usable_capacity × (1 − reserve) / daily_charge
  ```

- The dominant term identified explicitly, and the sensitivity of battery life to
  each input stated
- A verdict on the ≥ 6-month target at a 15-minute interval: reachable as built,
  reachable with named changes, or not reachable — with the numbers behind it
- Results in `docs/testing/hil-runs/`, raw traces included

## Non-goals

- Designing a low-power PCB. M14-009 plans it; nothing is fabricated.
- Optimising anything. This issue measures; changes it motivates are separate
  work, prioritised by the measured dominant term.
- Solar production measurement. M14-009 owns the outdoor side.
- Publishing a battery-life figure anywhere as a specification. Until this issue
  is done, every autonomy number in the repository stays labelled a target.

## Dependencies

- M10-011

## Implementation notes

A multimeter will not do this. Sleep current is microamps and wake current is
tens to hundreds of milliamps within the same second, and a handheld meter
either burdens the supply at the low end or averages the peaks away at the high
end. Use a purpose-built low-side or high-side measurement with a wide dynamic
range and a stated burden voltage, or a bench supply with integration; record
which instrument was used and its resolution, because a reader's ability to trust
the figure depends on it.

Measure the assembled device including everything a deployed unit would have. A
figure taken from a bare module with the sensor unplugged is the chip figure
wearing a disguise, and producing one is the specific mistake this issue exists
to prevent.

**Expect awake time to dominate, and confirm or refute it explicitly.** A quick
sanity model: a 3000 mAh cell over 183 days allows about 16 mAh per day. If
complete-system sleep is 50 µA that is 1.2 mAh/day, leaving roughly 15 mAh/day
across 96 wakes — about 155 µAh each, which at ~100 mA average is under six
seconds of awake time per wake. **These numbers are illustrative arithmetic with
no measured input; they are here to show the shape of the budget, not to predict
it.** If they survive contact with the meter, the levers worth pursuing are Wi-Fi
association time and warm-up overlap, not sleep current — and if they do not, the
finding is more valuable than the target.

Note the levers this measurement may recommend, without committing to any:
persisted BSSID and channel to skip a scan, a static IP to skip DHCP, sampling on
several wakes but transmitting on one, and a longer interval. Each trades
something real — the last two trade freshness, which is a safety-relevant input
under SAFETY-005 — so none is a free win and none is chosen here.

## Acceptance criteria

- [ ] Complete-system sleep current measured on the assembled device with sensor
      and RS485 attached, at a stated supply voltage, with the instrument named.
- [ ] Chip deep-sleep current reported separately and explicitly labelled as not
      being the system figure.
- [ ] Wake-cycle energy integrated and broken into named phases.
- [ ] A watering wake measured separately from a sampling wake.
- [ ] At least 20 cycles; results given as a distribution.
- [ ] Every input to the budget model labelled measured or assumed.
- [ ] The dominant term identified, with a sensitivity statement per input.
- [ ] An explicit verdict on ≥ 6 months at a 15-minute interval, with numbers.
- [ ] Raw traces recorded in `docs/testing/hil-runs/`.
- [ ] Every autonomy figure elsewhere in the repository updated to match, or
      still labelled a target where this issue could not settle it.

## Verification

```bash
# bench measurement with a wide-dynamic-range current instrument
# results recorded in docs/testing/hil-runs/energy-budget.md
cargo run -p rhizo-docscheck
grep -rn 'months' docs/prd/140-field-readiness.md   # every figure labelled
```

## Tests required

- Manual bench measurement; the recorded traces are the artefact.
- A review check that no autonomy figure is stated as a specification anywhere it
  is not backed by this issue's results.

## Documentation impact

- `docs/testing/hil-runs/energy-budget.md` — new.
- [PRD 140](../../prd/140-field-readiness.md) — hardware targets confirmed or
  corrected.
- [ADR-018](../../adr/018-battery-and-deep-sleep-device-mode.md) §8.
- [deployment-model.md](../../architecture/deployment-model.md) §2b.

## Files likely affected

```text
docs/testing/hil-runs/energy-budget.md
docs/prd/140-field-readiness.md
docs/architecture/deployment-model.md
```
