# Issue M9-012 — Implement device configuration handling

**Milestone:** M9 · **PRD:** [PRD 090](../../prd/090-esp32-rust-firmware.md) · **Depends on:** M9-011

## Context

Protocol section 5.7. Includes the version-monotonicity rule that protects
against retained-message replay after a rollback.

## Goal

Apply and persist device configuration.

## Scope

- Receive retained config, validate, apply, persist to NVS
- Echo `applied_config_version` in status
- **Ignore config with `config_version <= applied`**
- Invalid config rejected; the previous config retained
- Unrecognised fields ignored

## Non-goals

- Publishing config — the edge's job.

## Dependencies

- M9-011

## Implementation notes

Ignoring unrecognised fields is what makes adding a config field
non-breaking across mixed firmware versions — and it also means an attempt to
smuggle a safety limit through the config topic has no effect. Assert that.

The monotonicity rule is easy to omit and its absence is invisible until a
rollback republishes an old retained config.

## Acceptance criteria

- [ ] A valid config is applied and persisted.
- [ ] `applied_config_version` is echoed in status.
- [ ] A config with a lower version is ignored.
- [ ] An invalid config is rejected and the previous retained.
- [ ] A config containing `max_ml_per_run` has no effect on the reported limits.
- [ ] Config survives a reboot.

## Verification

```bash
cd firmware/esp32-node && cargo test config::
```

## Tests required

- Apply and persist.
- Monotonicity.
- Invalid rejection.
- Smuggled-limit no-op.

## Documentation impact

- None.

## Files likely affected

```text
firmware/esp32-node/src/app/config.rs
```
