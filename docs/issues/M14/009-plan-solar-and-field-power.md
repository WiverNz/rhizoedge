# Issue M14-009 — Plan solar and outdoor field power, and validate the autonomy claims

**Milestone:** M14 · **PRD:** [PRD 140](../../prd/140-field-readiness.md) · **Depends on:** M14-002, M14-006

## Context

[ADR-018](../../adr/018-battery-and-deep-sleep-device-mode.md) closed the
sleeping-Wi-Fi-device problem inside v1 and treated solar as an optional power
source rather than a separate architecture. What it deliberately did not do was
design the outdoor deployment: the panel, the charger, the enclosure, the winter
case, and the arithmetic that decides whether any of it works at a given
latitude.

This issue is that planning, and — like the rest of M14 — it produces
documentation and no implementation.

It also carries an obligation the rest of the roadmap depends on: **the autonomy
targets stay labelled as targets until M10-012's measurements say otherwise**,
and this issue is where the labelling is audited.

## Scope

- The power chain, specified to the level of part *classes* rather than part
  numbers:

  ```text
  solar panel → LiFePO4-compatible charger/controller → battery
              → low-Iq regulation → load switches → ESP32 / sensors / pump
  ```

- Why LiFePO4: cell voltage that suits a 3.3 V regulator with minimal headroom,
  a wide temperature range that matters outdoors, and cycle life that suits a
  daily shallow discharge — stated as reasoning, with the alternatives and their
  drawbacks
- The **energy-neutrality** definition, and the discipline of using it:

  ```text
  energy-neutral operation = measured production exceeds measured consumption
                             over a stated period, at a stated location and
                             season, with a stated reserve margin
  ```

- Worked seasonal arithmetic for a stated reference location: panel rating,
  realistic winter insolation and its derating, days of autonomy required to
  survive a run of overcast days, and the resulting battery and panel sizing —
  every input labelled measured, cited, or assumed
- The pump's energy cost sized separately: a dose is a large, infrequent, bursty
  load and dominates a day on which it happens
- Outdoor enclosure and environmental constraints: ingress, condensation, cold
  charging (LiFePO4 must not be charged below freezing without protection, which
  is a charger requirement rather than an afterthought), UV, and thermal
- Why no watering or safety decision may read solar or charge state, restated in
  the same terms as the other gate inputs, with the failure it prevents made
  concrete
- A future low-power PCB sketch, **not designed and not fabricated**:

  ```text
  ESP32-C3 module
  low-Iq regulator
  load switch → RS485
  load switch → soil sensor
  MOSFET → pump
  ```

- An audit: every autonomy, battery-life, and solar figure in the repository
  located and confirmed to be labelled a target requiring measurement, or backed
  by M10-012

## Non-goals

- Any schematic, layout, BOM, or part number. The sketch above is a block
  diagram; producing a board is out of scope for V1 and for this issue.
- Purchasing or testing hardware.
- MPPT versus a simple charge controller as a decided question. State the
  trade-off and what would decide it.
- Any code. `git diff` must show no implementation, which is M14's whole exit
  criterion.
- Solar telemetry kinds or a charge-state field in the protocol. Nothing
  produces them and nothing may consume them.

## Dependencies

- M14-002
- M14-006

## Implementation notes

Do the seasonal arithmetic for a **named** location and season, not in general.
"A 5 W panel is plenty" is the kind of claim that is true in July at 40° and
false in December at 55°, and the difference is roughly a factor of ten. Naming
the location makes the number checkable and makes its inapplicability elsewhere
obvious.

Size for the worst realistic case rather than the average: a week of overcast
December weather, where production is a small fraction of the seasonal mean.
Battery days-of-autonomy is what covers that, and it is the term most often left
out of a cheerful calculation.

Be careful with the pump. A dose is seconds of hundreds of milliamps to amps, and
a device that waters daily has a materially different budget from one that
monitors only. Size both, and note that a monitoring-only outdoor node is a much
easier problem — which is worth saying, because it is likely to be the first one
actually deployed.

Keep the prohibition in §7 concrete. The failure it prevents is specific: a
device with a full battery and bright sun "having margin to spare" and watering
more freely than one on a cloudy day, which would make irrigation a function of
weather through the least defensible possible route. Battery voltage is
telemetry, it may raise a maintenance alert, and it grants nothing.

The audit at the end is the part most likely to be skipped and the part with the
most value. Grep the repository for autonomy figures, and check each one is
either labelled a target or traceable to `docs/testing/hil-runs/energy-budget.md`.
The word "infinite" should appear nowhere near solar.

## Acceptance criteria

- [ ] The power chain is specified at the level of part classes with reasoning.
- [ ] LiFePO4's selection is justified against named alternatives.
- [ ] Energy neutrality is defined as a measured, bounded, seasonal claim and
      used that way throughout.
- [ ] Seasonal arithmetic is worked for a **named** location and season, with
      every input labelled measured, cited, or assumed.
- [ ] Days-of-autonomy sizing covers a stated run of overcast days.
- [ ] The pump's contribution is sized separately from the monitoring baseline.
- [ ] Cold-charging protection is stated as a charger requirement.
- [ ] The prohibition on solar or charge state entering any decision is restated
      with its concrete failure mode.
- [ ] The PCB block sketch is present and explicitly not a design.
- [ ] Every autonomy figure in the repository is audited: labelled a target, or
      traceable to M10-012.
- [ ] No claim of indefinite or infinite autonomy appears anywhere.
- [ ] **`git diff` shows no implementation** — no schematic, no part number, no
      solar field in any payload or table.

## Verification

```bash
cargo run -p rhizo-docscheck
git diff --stat -- crates/ firmware/ migrations/     # expect empty
grep -rniE 'infinite|forever|unlimited' docs/ | grep -i -E 'solar|power|battery'
grep -rniE 'month|autonomy' docs/prd/140-field-readiness.md
```

## Tests required

- Review-based; the recorded analysis is the artefact.
- The audit is itself checkable: every figure found by the greps above resolves
  to a label or a measurement.

## Documentation impact

- [PRD 140](../../prd/140-field-readiness.md) — solar and field power section.
- [deployment-model.md](../../architecture/deployment-model.md) §6 field
  topology.
- [ADR-018](../../adr/018-battery-and-deep-sleep-device-mode.md) §7 and §8
  cross-referenced.

## Files likely affected

```text
docs/prd/140-field-readiness.md
docs/architecture/deployment-model.md
docs/architecture/field-power.md
```
