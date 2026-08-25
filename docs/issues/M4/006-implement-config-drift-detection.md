# Issue M4-006 — Implement configuration drift detection

**Milestone:** M4 · **PRD:** [PRD 040](../../prd/040-device-registry-and-health.md) · **Depends on:** M4-001

## Context

ADR-011: silent configuration drift is the failure mode the versioned-config
design exists to prevent. Desired versus applied must be visible.

## Goal

Detect and surface a device not running its intended configuration.

## Scope

- Compare `desired_config_version` with `applied_config_version`
- Raise `config_drift` after two telemetry intervals of disagreement
- Expose `drift` in the device API
- Clear the condition when the versions match

## Non-goals

- Publishing config (M6-013).

## Dependencies

- M4-001

## Implementation notes

The two-interval grace period avoids flagging the normal window between
publishing a config and the device applying it.

Drift is a warning, not a lockout — a device on an older telemetry interval is
still reporting truthfully. The value is in the operator knowing.

## Acceptance criteria

- [ ] Matching versions report `drift: false`.
- [ ] A mismatch persisting beyond two intervals raises `config_drift`.
- [ ] A brief mismatch during application does not raise it.
- [ ] The condition clears when versions match.
- [ ] `drift` is exposed in the device API.

## Verification

```bash
cargo test -p edge-controller device::drift
```

## Tests required

- Drift detection timing.
- No false positive during normal application.
- Clearing.

## Documentation impact

- None.

## Files likely affected

```text
crates/edge-controller/src/device/config_drift.rs
```
