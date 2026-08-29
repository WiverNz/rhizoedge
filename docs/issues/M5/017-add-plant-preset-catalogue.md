# Issue M5-017 — Add the versioned offline plant preset catalogue

**Milestone:** M5 · **PRD:** [PRD 050](../../prd/050-plant-model-and-recommendations.md) · **Depends on:** M5-014

## Context

Configuring a plant from nothing means choosing a target moisture band, warning
and critical thresholds, a staleness horizon, and a light and temperature
expectation, per measurement kind. An operator who has just bought a monstera
does not know any of those numbers, and the cost of guessing is either a dead
plant or automation they never enable.

A **plant preset** is a reusable starting configuration for a species: pick
"Rose", "Monstera", or "Basil" and the per-measurement policy is prefilled.

It is a *starting point*, not an authority. Two rules follow, and both are
load-bearing:

- **`PlantProfile` remains a template and a preset remains a template.** Neither
  is runtime state. [ADR-016](../../adr/016-plant-binding-and-policy-model.md)
  made per-plant bindings and `MeasurementPolicy` rows the authoritative
  configuration, and a preset must reach the plant *through* that model, never
  around it.
- **A preset is not a schedule.** No "water every 2 days". A preset stores what
  a species *prefers* — a moisture band, a light level, a temperature range —
  and watering remains a function of measurements and the M6 safety gate. A
  timer would be a second actuation authority that no sensor and no lockout can
  contradict, which is the failure this architecture exists to prevent.

## Goal

A built-in, versioned, curated species catalogue that is queryable offline and
carries its provenance.

## Scope

- A `PlantPreset` type in `rhizo-domain`: species identity, display name, common
  synonyms for search, and per-`MeasurementKind` preferences
- **Preference ranges, not setpoints**: soil-moisture band, light preference,
  temperature range, humidity range, pH range, and a suggested dose class and
  cooldown class
- The catalogue **embedded in the binary** — a versioned data file compiled in,
  with `catalogue_version` and a stable `preset_id` per entry
- Per-entry provenance metadata: `source`, `source_ref`, `license`, and
  `retrieved_at`
- **A `Provenance` discriminator on every value**: `SourceFact` for a figure
  taken from a cited source in its own units, `RhizoDefault` for a starting
  value Rhizo derived
- Catalogue validation at build or test time: unique ids, ranges ordered
  (`min <= max`), every entry carrying licence and source fields
- Read-only query: list, and search by name or synonym
- **Measurement entries name a `MeasurementKind` and a policy intent, and
  nothing physical**: no `device_id`, no `sensor_id`, no `point`, no capability
  identity anywhere in the catalogue

## Non-goals

- Applying a preset to a plant — M5-018.
- Any UI — M12-017.
- Fetching from an external API at runtime, ever. See Implementation notes.
- **Choosing sensors.** A preset expresses what a species prefers, per
  measurement kind. Which physical probe supplies that kind for a given plant is
  a `SensorBinding`, and stays one. See Implementation notes.
- Editing the built-in catalogue at runtime, or user-defined presets. Both are
  plausible later; neither is needed to make the first plant configurable, and
  a writable catalogue raises migration questions this issue should not answer.

## Dependencies

- M5-014

## Implementation notes

**Source facts and derived defaults are different kinds of claim and must not be
stored in the same shape.** A horticultural source saying a species likes
"soil humidity 6" on some vendor's 1-10 scale is a fact about that source. The
volumetric water content Rhizo would target is an interpretation — a guess with
a conversion inside it. Presenting the second as though it were the first is how
a plausible number acquires unearned authority, and it is exactly the sort of
claim an operator would never think to question. `SourceFact` keeps the original
figure and its units; `RhizoDefault` records that Rhizo chose the number and,
where one exists, what it was derived from. The UI shows the difference
(M12-017), and it cannot do that if the domain has already flattened it.

**Offline is not a feature here, it is the architecture.** §2 of the README puts
the cloud at optional and absent-for-a-week; a catalogue behind an HTTP call
would make creating a plant the one operation that needs the internet. Embedding
it also means the catalogue is versioned with the binary, so a given release
always produces the same starting configuration.

**External sources are for import, never for runtime.** Trefle and Perenual are
plausible research inputs for building the curated catalogue offline, as a
tooling step whose output is reviewed and committed. Two cautions: their
licences must be **verified to permit redistribution before any of their data is
committed** — an API being free to query says nothing about redistributing its
contents — and their per-species figures are uneven, so an imported row is a
draft for a human to accept, not a catalogue entry. Record the outcome of the
licence check in the PRD's open questions rather than assuming it.

**A preset describes a plant, not an installation.** Entries are keyed by
`MeasurementKind` — "this species wants soil moisture in this band" — and carry
no device, sensor, point, or capability identity. A catalogue cannot know which
probe is in which pot, and the moment an entry names one it has started
competing with `SensorBinding` for the same decision. Binding a kind to a real
sensor is M5-013's job and remains so; the preset only says what the kind should
be configured to, and M5-018 resolves it against whatever bindings the plant
already has.

Keep the first catalogue small and genuinely curated. Twenty species that are
right is worth more than four hundred scraped rows, and a small catalogue makes
the provenance discipline visible while it is still cheap to establish.

## Acceptance criteria

- [x] `PlantPreset` exists in `rhizo-domain` with per-`MeasurementKind` preferences.
- [x] The catalogue is embedded, has a `catalogue_version`, and needs no network or database.
- [x] Every entry carries `source`, `source_ref`, `license`, and `retrieved_at`.
- [x] Every preference value is either `SourceFact` or `RhizoDefault`; there is no third, unlabelled case.
- [x] **No preset contains an interval, frequency, or schedule field** — asserted by a test over the whole catalogue.
- [x] Search finds a species by display name and by synonym.
- [x] A malformed entry (duplicate id, inverted range, missing licence) fails the catalogue validation test.
- [x] **No catalogue field names a device, sensor, point, or capability** —
      asserted by a test over the whole catalogue, alongside the no-schedule
      assertion.
- [x] `rhizo-domain` stays pure: no I/O, no clock.

## Verification

```bash
cargo test -p rhizo-domain preset::
cargo test -p rhizo-domain catalogue::
```

## Tests required

- Catalogue validation over every entry.
- The no-schedule assertion across the whole catalogue.
- The no-physical-sensor assertion across the whole catalogue.
- Search by name and by synonym, including a miss.
- Provenance is present on every value.

## Documentation impact

- PRD 050 §Data model and §Open questions (the licence verification).

## Files likely affected

```text
crates/domain/src/preset/mod.rs
crates/domain/src/preset/catalogue.rs
crates/domain/data/presets.v1.json
```
