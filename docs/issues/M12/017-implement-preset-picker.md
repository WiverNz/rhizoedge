# Issue M12-017 — Implement the species preset picker and review step

**Milestone:** M12 · **PRD:** [PRD 120](../../prd/120-rust-ui.md) · **Depends on:** M12-013, M12-014, M12-002

## Context

M5-017 and M5-018 give the edge a species catalogue and a way to materialise it.
This is the surface that makes it usable: search for a species, take the
recommended settings, then **review and edit them before anything is saved**.

The review step is the point of the feature, not a courtesy. A preset is a
starting guess drawn from a curated catalogue, and some of its numbers are
Rhizo's interpretation rather than anything a source said. Applying it silently
would hand the operator a configuration they have never seen, for a plant they
are responsible for.

## Goal

Search a species, generate a configuration from it, review and edit it, then
save — with the manual path untouched.

## Scope

- A species search in the plant-creation flow: type a name, match display names
  and synonyms, select one
- **"Use recommended settings"** producing a fully populated draft configuration
- A review screen showing every generated value **before** any request that
  writes, with each field editable in place
- **Provenance shown per value**: a value a source stated is labelled
  differently from one Rhizo derived, in words rather than a bare icon
- The catalogue version and the entry's source and licence visible on the review
  screen
- **"Configure manually"** as an equal, always-available path — not a fallback
  reached by dismissing the preset flow
- Applying to an already-configured plant shows the field-level diff M5-018
  returns and requires confirmation
- A validation rejection from the edge renders its reason against the offending
  field
- **Monitoring-only plants are a normal target**: the flow completes for a plant
  with no actuator, showing its measurement thresholds and **no watering
  control at all** — not a disabled one
- The review screen shows thresholds **per measurement kind**, resolved against
  the plant's existing bindings; it offers no sensor or device picker

## Non-goals

- The catalogue and its application — M5-017, M5-018.
- Editing or authoring presets in the UI.
- Any new watering control. This flow writes configuration and issues nothing.

## Dependencies

- M12-013
- M12-014
- M12-002

## Implementation notes

**Show the difference between a fact and a guess, in words.** "Source: RHS,
soil moisture 'moderate'" and "Rhizo starting value, derived from a moderate
moisture preference" are different sentences, and the second one invites the
edit that the operator is far better placed to make than the catalogue is. An
asterisk or a tooltip is not enough — this is the one screen where the
distinction changes what someone does.

**Nothing is written before the review screen is confirmed.** The draft is built
client-side from the preset the edge returned; the write happens once, on
confirm. A flow that creates the plant first and then edits it leaves a
half-configured plant behind whenever someone changes their mind, and on a
system that will later water things, a half-configured plant is worse than none.

**"Use recommended settings" is a suggestion, and the wording matters.** Not
"optimal", not "correct". The catalogue does not know the pot, the room, the
window, or the season, and the label should not imply otherwise.

**This screen does not bind sensors.** It configures a `MeasurementKind`; which
probe supplies that kind is the binding editor's job (M12-013), and duplicating
it here would give an operator two places to answer the same question
differently. A kind the plant has no binding for is shown as unconfigured with a
route to the binding editor, not silently filled in.

**A plant with no pump is not a broken preset.** Monitoring-only is the common
case, so the flow finishes normally and simply renders no watering control —
consistent with SAFETY-018 and with the existing M12 rule that a monitoring-only
plant shows no watering control rather than a disabled one.

**No override control appears here** — the M12 prohibition is unchanged and
unaffected. This screen writes thresholds; it does not water, and it offers no
path to bypass a lockout. Automation stays off after creation, exactly as
M5-018 leaves it, so enabling it remains a separate deliberate act through the
existing control.

## Acceptance criteria

- [ ] Searching a species matches display name and synonym, and shows a clear empty state on a miss.
- [ ] "Use recommended settings" fills every configurable field and writes nothing yet.
- [ ] Each generated value is individually editable before saving.
- [ ] A source-stated value and a Rhizo-derived value are distinguishable **in words** on screen.
- [ ] Catalogue version, source, and licence are visible on the review screen.
- [ ] "Configure manually" reaches the full editor without going through a preset.
- [ ] Re-applying to a configured plant shows the field-level diff and requires confirmation.
- [ ] A 422 from the edge renders against the field that caused it.
- [ ] Automation is still off after a plant is created from a preset.
- [ ] The flow completes for a plant with **no actuator**, showing thresholds and **no watering control at all**.
- [ ] The review screen offers **no sensor or device picker**; an unbound kind is shown as unconfigured with a route to the binding editor.
- [ ] **No override control and no watering action exists anywhere in this flow.**
- [ ] The flow works with no internet connection.

## Verification

```bash
cd ui/rhizo-ui && cargo test preset
grep -rn 'optimal\|correct settings' ui/rhizo-ui/src  # expect no matches
```

## Tests required

- Search matching, including synonym and empty state.
- The draft is not persisted until confirmation.
- An edited value is what gets sent, not the generated one.
- Provenance rendering for both kinds of value.
- The manual path reaches the editor unchanged.

## Documentation impact

- PRD 120 §User/system flows and §Functional requirements.

## Files likely affected

```text
ui/rhizo-ui/src/views/plant_create.rs
ui/rhizo-ui/src/components/preset_picker.rs
ui/rhizo-ui/src/components/preset_review.rs
```
