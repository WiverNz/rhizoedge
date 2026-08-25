# Issue M14-005 — Document the weather integration boundary

**Milestone:** M14 · **PRD:** [PRD 140](../../prd/140-field-readiness.md) · **Depends on:** M14-004

## Context

PRD 140's rule: weather is an input to the recommendation engine, **never to
the safety gate**. Rain forecast may say 'do not irrigate'; it may never say
'irrigate despite a leak'.

## Goal

Define where weather may and may not influence decisions.

## Scope

- Weather as a recommendation input only
- **The safety gate stays local and physical** — no remote data may relax it
- Evapotranspiration estimation from weather
- Forecast uncertainty handling
- Behaviour when the weather source is unavailable — degrade to no weather, never to permissive

## Non-goals

- Implementing a weather client.

## Dependencies

- M14-004

## Implementation notes

The unavailable-source rule is the SAFETY-012 application: a missing
forecast must degrade to 'decide without weather', never to 'assume no rain and
irrigate'. Absence of a forecast is not evidence about the sky.

Structurally, weather data must not reach `IrrigationInputs`' safety fields —
the same discipline that keeps cloud state out (SAFETY-009).

## Acceptance criteria

- [ ] Weather is specified as a recommendation input only.
- [ ] **The gate is explicitly excluded from weather influence.**
- [ ] Evapotranspiration estimation is outlined.
- [ ] Forecast uncertainty handling is specified.
- [ ] Source unavailability degrades to no-weather, never to permissive.
- [ ] The structural separation mirrors SAFETY-009's.

## Verification

```bash
cargo run --manifest-path tools/docscheck/Cargo.toml
```

## Tests required

- Review-based.

## Documentation impact

- docs/architecture/weather-boundary.md.

## Files likely affected

```text
docs/architecture/weather-boundary.md
```
