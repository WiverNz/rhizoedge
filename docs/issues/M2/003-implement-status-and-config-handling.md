# Issue M2-003 — Implement status publication and config application

**Milestone:** M2 · **PRD:** [PRD 020](../../prd/020-device-simulator.md) · **Depends on:** M2-002

## Context

Protocol sections 5.5 and 5.7 define the status heartbeat and the retained
config flow, including the rule that a config with a version at or below the
applied one is ignored — protection against retained-message replay.

## Goal

Publish status correctly and apply retained configuration.

## Scope

- Status published on connect, on config change, and every `5 x telemetry_interval`
- Status carries uptime, heap (simulated), sensor health, `applied_config_version`, and the compile-time limits
- Config validated on receipt; invalid config rejected and the previous kept
- `config_version <= applied` ignored
- Unrecognised config fields ignored
- Applied config persisted (M2-007 provides the store)

## Non-goals

- Publishing config — that is the edge's job (M6-013).

## Dependencies

- M2-002

## Implementation notes

The `limits` block reports `FIRMWARE_MAX_*` from the contract crate. It is
reporting only; assert in a test that no config path can change them.

The version-monotonicity rule is easy to omit and its absence is invisible until
a rollback republishes an old retained config and the device silently regresses.

## Acceptance criteria

- [ ] Status is published on connect and on the heartbeat schedule.
- [ ] `applied_config_version` echoes an applied config.
- [ ] A valid config is applied and reflected in behaviour (e.g. telemetry interval changes).
- [ ] An invalid config is rejected and the previous one retained.
- [ ] A config with a lower `config_version` is ignored.
- [ ] A config containing `max_ml_per_run` has no effect on the reported limits.

## Verification

```bash
cargo test -p device-simulator config::
# integration: publish a retained config, observe applied_config_version
```

## Tests required

- Config validation.
- Version monotonicity.
- Smuggled-limit no-op.
- Integration: retained config delivered to a late-connecting simulator.

## Documentation impact

- None.

## Files likely affected

```text
crates/device-simulator/src/config.rs
crates/device-simulator/src/status.rs
```
