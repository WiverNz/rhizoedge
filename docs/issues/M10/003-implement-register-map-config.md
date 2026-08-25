# Issue M10-003 — Implement register maps as configuration data

**Milestone:** M10 · **PRD:** [PRD 100](../../prd/100-real-soil-sensor.md) · **Depends on:** M10-002

## Context

PRD 100 F-100-04: register maps are **data, not code**, so supporting a new
probe model is a configuration entry rather than a firmware change.

## Goal

Make probe models configurable.

## Scope

- `RegisterMap` with per-field address, count, scale, offset, and signedness
- Named maps selectable from device config
- A built-in map for a generic 3-in-1 probe
- Validation of a map at load

## Non-goals

- A map editor or discovery.

## Dependencies

- M10-002

## Implementation notes

Signedness and scaling are where probe datasheets differ most, and getting
them wrong produces plausible-looking wrong numbers — the most dangerous kind.
Validate ranges after applying a map and reject a map that produces impossible
values on a known input.

## Acceptance criteria

- [ ] A register map is selectable by name from config.
- [ ] Scale, offset, and signedness are applied correctly.
- [ ] A generic 3-in-1 map is built in.
- [ ] An invalid map is rejected at load.
- [ ] Adding a model requires no code change.

## Verification

```bash
cd firmware/esp32-node && cargo test modbus::map
```

## Tests required

- Map application including signed and scaled values.
- Validation rejection.

## Documentation impact

- Documented procedure for adding a probe model.

## Files likely affected

```text
firmware/esp32-node/src/sensors/modbus/map.rs
```
