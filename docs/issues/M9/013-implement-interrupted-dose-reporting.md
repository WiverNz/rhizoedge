# Issue M9-013 — Implement interrupted dose detection and reporting

**Milestone:** M9 · **PRD:** [PRD 090](../../prd/090-esp32-rust-firmware.md) · **Depends on:** M9-011, M9-007

## Context

SAFETY-011. An interrupted dose delivered an unknown volume, and treating
unknown as either zero or full-success would be wrong in a dangerous direction.

## Goal

Detect and report a dose interrupted by a restart.

## Scope

- On boot, after `pump_off()`, check NVS for an unfinished dose record
- Publish `command.result` with `status: "interrupted"`, **`delivered_ml: null`**
- Clear the record after the result is acknowledged
- **Never resume an interrupted dose**
- The edge credits `requested_ml` conservatively (M6-010)

## Non-goals

- Edge-side handling, planned in prerequisite M6-010.

## Dependencies

- M9-011
- M9-007

## Implementation notes

`delivered_ml: null` means genuinely unknown. Reporting 0.0 would let the
edge grant the full budget again; reporting the requested volume from the device
would be a guess. Null lets the edge apply its own conservative policy.

Never resume: the correct response to an unknown partial delivery is to report
and let the edge re-evaluate with fresh soil data.

## Acceptance criteria

- [x] An unfinished NVS record on boot produces an `interrupted` result.
- [x] `delivered_ml` is **null**, not 0.
- [x] The pump is off before the check runs.
- [x] The dose is **not** resumed.
- [x] The record clears after acknowledgement.
- [x] The result survives a failure to publish and is retried next boot.

## Verification

```bash
cd firmware/esp32-node && cargo test interrupted::
cargo test safety_011
```

## Tests required

- **`safety_011_interrupted_dose_reported`.**
- Null delivered volume.
- No resumption.
- Record clearing.

## Documentation impact

- None.

## Files likely affected

```text
firmware/esp32-node/src/app/recovery.rs
```
