# Issue M13-012 — Update the UI for a larger deployment

**Milestone:** M13 · **PRD:** [PRD 130](../../prd/130-multi-plant-home.md) · **Depends on:** M13-011, M12-017

## Context

An overview designed for one plant does not work for twenty. Grouping,
filtering, and bulk actions become necessary.

## Goal

Keep the UI legible and safe at scale.

## Scope

- Grouped overview with collapsible groups
- Filtering by group, state, and lockout
- **Bulk automation toggle with a confirmation listing every affected plant**
- Reservoir view showing dependent plants
- Notification configuration
- Legible at 20 plants

## Non-goals

- Bulk manual watering — deliberately not offered.

## Dependencies

- M13-011
- M12-017

## Implementation notes

Bulk manual watering is omitted on purpose: manual watering is a considered
per-plant act, and a bulk version is a way to make a large mistake quickly.

The bulk automation confirmation must list every affected plant by name, not
just a count — 'enable automation for 12 plants' is not informed consent.

## Acceptance criteria

- [ ] The overview groups plants and stays legible at 20.
- [ ] Filtering works.
- [ ] **Bulk automation toggle lists every affected plant by name before applying.**
- [ ] The reservoir view shows dependent plants.
- [ ] Notifications are configurable.
- [ ] **No bulk manual watering control exists.**

## Verification

```bash
cd ui/rhizo-ui && cargo tauri dev   # with 20 plants configured
```

## Tests required

- Grouping and filtering.
- Bulk confirmation contents.
- Absence of bulk manual watering.

## Documentation impact

- None.

## Files likely affected

```text
ui/rhizo-ui/src/views/overview.rs
ui/rhizo-ui/src/views/reservoirs.rs
```
