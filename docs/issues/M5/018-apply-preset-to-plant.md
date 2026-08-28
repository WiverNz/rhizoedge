# Issue M5-018 — Apply a preset to a plant

**Milestone:** M5 · **PRD:** [PRD 050](../../prd/050-plant-model-and-recommendations.md) · **Depends on:** M5-017, M5-014, M5-016

## Context

M5-017 gives the edge a catalogue. This issue is the one operation that uses it:
turning a chosen species into the per-plant configuration the rest of the system
already understands.

The whole value is in what it does **not** do. It does not introduce a second
configuration path, it does not become the plant's authority, and it does not
survive as a link that later edits have to fight. It writes ordinary
`MeasurementPolicy` rows and ordinary defaults, exactly as a hand-configured
plant would have, and then gets out of the way.

## Goal

Materialise a preset into editable per-plant configuration, through the existing
binding and policy model.

## Scope

- `POST /plants` accepting an optional `preset_id`, and
  `POST /plants/{id}/apply-preset` for an existing plant
- Materialisation **exactly once, at the moment of application**, into ordinary
  `MeasurementPolicy` rows plus optional automation starting defaults — the same
  rows and the same validation as a hand-configured plant. There is no second
  materialisation: not on restart, not on a catalogue upgrade, not on any tick
- Resolution **against the plant's existing `SensorBinding` rows**: a preset
  names a `MeasurementKind`, and the binding decides which physical sensor
  supplies it. Applying a preset never creates, chooses, or edits a binding
- Automation defaults from the preset's dose and cooldown classes, resolved
  against pot volume where the preset expresses a class rather than a figure
- `auto_watering_enabled` stays **`false`** (SAFETY: unchanged by this issue)
- Record `applied_preset_id` and `applied_catalogue_version` on the plant as
  **provenance only**
- Every materialised value is editable afterwards through the existing M5-014
  endpoints, with no preset-owned field and no re-application on restart
- Applying to a plant that already has policies requires an explicit
  `overwrite` and reports what changed
- Preset-derived measurement kinds with no binding produce **no policy row**
- **A monitoring-only plant — no `ActuatorBinding` — is a fully supported
  target.** Application succeeds, measurement policies are created normally, and
  any dose or cooldown default is recorded as an inert starting value that
  creates no actuation path and no actuator binding. SAFETY-018 is untouched

## Non-goals

- The catalogue itself — M5-017.
- Any UI — M12-017.
- Issuing commands. M5 issues none, and this issue does not change that.
- Offline-policy authoring, which stays M5-016's; a preset may supply starting
  numbers for it, but validation and activation remain unchanged.

## Dependencies

- M5-017
- M5-014
- M5-016

## Implementation notes

**`applied_preset_id` is provenance, not a foreign key with behaviour.** It
exists so an operator can see where the numbers came from and so a later
catalogue version can offer them a diff — never so configuration can be
re-derived behind their back. Specifically, and testably, it is **not read by
recommendation, by the safety gate, by irrigation control, or by offline-policy
evaluation**. Those four consume `MeasurementPolicy` rows, bindings, and
measurements, exactly as they do for a hand-configured plant, and they cannot
tell the two apart — which is the property that makes a preset a starting point
rather than a second configuration authority.

That is worth stating as a prohibition rather than an intention, because the
convenient shortcut is real: once a decision can ask "which preset was this?",
the plant has two owners, and the operator's edit is the one that silently
loses. The moment applying a preset becomes something the system redoes on its
own, the same thing happens more slowly.

**A preset must not be able to widen a safety limit.** Materialised values pass
through the same validation as a hand-entered one, and a preset asking for a
dose above the profile's hard limit is rejected by M5-003's existing check
rather than clamped. A curated catalogue is not a trusted input; it is an input.

**The dose and cooldown classes are deliberately classes.** A preset cannot know
the pot, and millilitres without a pot volume are meaningless. Resolving a class
against `pot_volume_ml` at application time keeps the catalogue free of a number
it has no way to know, and keeps the plant's dose a property of the plant.

**Monitoring-only is a normal target, not an error case.** Most plants in a real
home have no pump ([README](../../../README.md) §The pump is optional), and a
preset is at its most useful for exactly those: it is how someone gets sensible
warning and critical bands for a fern they cannot water automatically. So
application must succeed with no `ActuatorBinding` present, and the preset's dose
and cooldown classes are simply recorded as inert defaults — they neither create
an `ActuatorBinding`, nor require one, nor enable automation, nor cause the
application to fail. SAFETY-018 continues to hold for the resulting plant by
construction: nothing here writes to `actuator_bindings`, so the actuation path
is still absent and `POST /water` still returns 422 `no_actuator_bound`.

**A preset names a kind; a binding names a sensor.** Materialisation reads the
plant's existing `SensorBinding` rows and configures the kinds they cover. It
does not pick a probe, invent a binding, or reorder roles — a catalogue has no
idea which sensor is in which pot, and a preset that guessed would be overriding
the one part of the configuration the operator definitely knows better.

**Watering stays measurement-driven.** Nothing here schedules. The preset moves
a target band into a `MeasurementPolicy`; when the plant is actually watered
remains a question for the thresholds, the trends, and — from M6 — the safety
gate. If a field ever appears here that would let a plant be watered without a
reading, it is this rule being broken.

## Acceptance criteria

- [ ] Creating a plant with `preset_id` writes `MeasurementPolicy` rows indistinguishable from hand-configured ones.
- [ ] Every materialised value can afterwards be edited, and the edit survives a restart unchanged.
- [ ] `auto_watering_enabled` is still `false` on a plant created from a preset.
- [ ] Materialisation happens **exactly once**; no restart, catalogue upgrade, or tick re-derives a value.
- [ ] `applied_preset_id` and `applied_catalogue_version` are recorded, and are **not read by recommendation, the safety gate, irrigation control, or offline-policy evaluation** — asserted structurally, not only by behaviour.
- [ ] Recommendation and threshold evaluation produce identical results for a preset-configured plant and a hand-configured plant with the same values.
- [ ] Applying a preset to a plant with **no `ActuatorBinding` succeeds**, creates its measurement policies, creates no actuator binding, and leaves `POST /water` returning 422 `no_actuator_bound` (SAFETY-018 intact).
- [ ] Materialisation uses the plant's existing `SensorBinding` rows and creates, selects, or edits none.
- [ ] Applying to a configured plant without `overwrite` is refused; with it, the response names each changed field.
- [ ] A preset value violating a profile hard limit is **rejected with 422**, not clamped.
- [ ] A preset kind with no matching binding creates no policy row and is reported.
- [ ] Creating a plant with no `preset_id` behaves exactly as before this issue.
- [ ] **No MQTT command is published by any preset operation.**

## Verification

```bash
cargo test -p edge-controller preset::
cargo test -p edge-controller plants::
```

## Tests required

- Materialised rows equal hand-configured rows for the same numbers.
- An edit after application persists and is never reverted.
- Overwrite refused, then accepted with a change list.
- A limit-violating preset rejected with 422.
- The manual path unchanged when `preset_id` is absent.
- Zero commands published.
- A monitoring-only plant: application succeeds, policies exist, `actuator_bindings` is still empty, and `safety_018_no_actuator_no_command` still passes.
- Identical recommendation output for preset-configured and hand-configured plants carrying the same values.
- A structural assertion that `applied_preset_id` appears in no recommendation, safety, irrigation, or offline-policy module.

## Documentation impact

- PRD 050 §Interfaces and §Data model.
- [safety-invariants.md](../../architecture/safety-invariants.md) SAFETY-018 —
  add the preset path to its covered failure scenarios.

## Files likely affected

```text
crates/edge-controller/src/api/plants.rs
crates/edge-controller/src/plants/preset.rs
migrations/edge/
```
