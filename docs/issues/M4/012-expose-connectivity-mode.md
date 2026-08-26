# Issue M4-012 — Expose device connectivity mode and isolation history

**Milestone:** M4 · **PRD:** [PRD 040](../../prd/040-device-registry-and-health.md) · **Depends on:** M4-004, M4-011

## Context

[connectivity-modes.md](../../architecture/connectivity-modes.md) §7 requires the
operator to be told which mode a device is in, rather than inferring it from
silence. An isolated device that watered on its own is a very different situation
from one that simply stopped reporting.

## Goal

Make connectivity mode and isolation periods first-class, queryable state.

## Scope

- Derive mode from liveness plus the device's self-reported `connectivity` block
- Record isolation periods with start, end, and duration
- Expose `connectivity` in the device API: `connected` | `isolated` | `reconciling`
- Raise `device.isolated` and `device.reconciled` events
- Metric `devices_isolated` gauge

## Non-goals

- Reconciliation itself (M6-020).

## Dependencies

- M4-004
- M4-011

## Implementation notes

`reconciling` is a real state, not a cosmetic one: it is the window in which the
edge must not issue a dose (SAFETY-016). Modelling it here means M6-020 has
somewhere to hang that rule rather than inventing an ad-hoc flag.

Derive the mode from the edge's own liveness data primarily. The device's
self-report is useful context — it knows how long it was alone — but a device
that claims to be connected while the edge hears nothing is not connected.

## Acceptance criteria

- [ ] Mode is exposed and correct in all three states.
- [ ] Isolation periods are recorded with accurate start and duration.
- [ ] `device.isolated` and `device.reconciled` events are raised.
- [ ] The device's self-report is treated as advisory, not authoritative.
- [ ] The gauge reflects reality.

## Verification

```bash
cargo test -p edge-controller connectivity::
curl -s localhost:8080/api/v1/devices/plant-node-01 | jq .connectivity
```

## Tests required

- Mode transitions.
- Isolation period recording.
- Advisory self-report handling.

## Documentation impact

- http-api-boundaries.md device response shape.

## Files likely affected

```text
crates/edge-controller/src/device/connectivity.rs
```
