# Issue M14-004 — Design the zone and multi-depth data model

**Milestone:** M14 · **PRD:** [PRD 140](../../prd/140-field-readiness.md) · **Depends on:** M14-001

## Context

PRD 140's finding worth recording: the M6 state machine returns
`IssueDose { ml }` rather than `PumpOn { seconds }`, so it is already
actuator-agnostic and should port to valves unchanged.

## Goal

Design the model without building it.

## Scope

- A `zone` entity between plant and device: valve, flow target, measurement points
- Root-zone aggregation over a depth profile, weighted by root density
- How the M6 state machine ports to zones
- The valve-stuck-open failure, which is worse than a stuck pump
- What in the current model would need to change

## Non-goals

- Implementing zones or multi-depth ingestion.

## Dependencies

- M14-001

## Implementation notes

The valve-stuck-open case deserves attention: a valve on a pressurised
supply can drain a reservoir or worse, and unlike a pump it has no natural
duration bound. It will need a hardware-level bound independent of firmware —
the field equivalent of SAFETY-007.

Root-zone aggregation is a domain function, not a schema change, since
`point` already exists.

## Acceptance criteria

- [ ] The zone entity is specified with its relationships.
- [ ] Root-zone aggregation is specified as a domain function.
- [ ] The state machine's portability to zones is analysed and confirmed or corrected.
- [ ] **The valve-stuck-open failure is analysed with its required hardware bound.**
- [ ] Required changes to the current model are listed.
- [ ] **Nothing is implemented.**

## Verification

```bash
cargo run --manifest-path tools/docscheck/Cargo.toml
```

## Tests required

- Review-based.

## Documentation impact

- docs/architecture/zone-model.md.

## Files likely affected

```text
docs/architecture/zone-model.md
```
