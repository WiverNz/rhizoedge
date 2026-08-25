# Issue M6-013 — Implement device config publication

**Milestone:** M6 · **PRD:** [PRD 060](../../prd/060-irrigation-control-and-safety.md) · **Depends on:** M6-009, M4-006

## Context

ADR-011 layer L3: the edge owns device config and publishes it retained, so a
device booting days later receives current desired state with no liveness
tracking.

## Goal

Publish versioned device configuration.

## Scope

- `PUT /devices/{id}/config` validating and bumping `config_version`
- Publish retained on the config topic, QoS 1
- Persist `desired_config_version`
- Republish all configs if the broker appears to have lost retained state
- Validate against firmware hard limits and **reject** violations

## Non-goals

- Drift detection, which landed in M4-006.

## Dependencies

- M6-009
- M4-006

## Implementation notes

Config is retained; commands are not. This is the one place the edge sets
`retain = true`, alongside nothing else — worth a comment at the call site.

Detect lost retained state by an absent retained status on resubscribe
(protocol section 8) and republish, otherwise a broker that lost its persistence
leaves every device on stale config indefinitely.

The config payload must contain no safety limit field (M1-007).

## Acceptance criteria

- [ ] `PUT` validates, bumps the version, and publishes retained.
- [ ] A late-connecting device receives the current config.
- [ ] `config_version` increases monotonically.
- [ ] A config violating a firmware limit is rejected with 422.
- [ ] Lost retained state triggers republication.
- [ ] The published payload contains no safety limit field.

## Verification

```bash
cargo test -p edge-controller config::publish
cargo test --test integration retained_config_late_device
```

## Tests required

- Version monotonicity.
- Retained delivery to a late subscriber.
- Hard-limit rejection.
- Republication after retained loss.

## Documentation impact

- None.

## Files likely affected

```text
crates/edge-controller/src/control/config.rs
crates/edge-controller/src/api/device_config.rs
```
