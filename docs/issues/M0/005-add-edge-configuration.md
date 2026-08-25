# Issue M0-005 — Implement layered edge configuration with fail-fast validation

**Milestone:** M0 · **PRD:** [PRD 000](../../prd/000-platform-foundation.md) · **Depends on:** M0-002, M0-004

## Context

ADR-011 specifies configuration layered as defaults -> TOML -> `RHIZO_EDGE__*`
env -> flags, with secrets never read from the file. An edge that starts with a
silently-substituted default is worse than one that refuses to start.

## Goal

Load, validate, and expose the edge instance configuration (layer L2).

## Scope

- `EdgeConfig` type covering edge_id, mqtt, storage, control, cloud, api, and log sections
- Layered loading with `__` as the nesting separator
- Validation at startup; invalid configuration exits non-zero with the offending key
- `Debug` impl redacting `password`, `token`, `secret`
- A warning when a password-shaped key appears in the TOML file
- `--config` and `--log-level` flags

## Non-goals

- Device config (L3, M6-013).
- Plant profiles (L4, M5-003).

## Dependencies

- M0-002
- M0-004

## Implementation notes

Choose between `figment` and `config` by comparing the error message for a
malformed key — that is the deciding criterion, since the whole point is a
legible failure. Record the choice in a code comment.

The MQTT password field must exist **only** in the env layer. A password written
into the TOML must be ignored and warned about, not honoured — config files get
pasted into bug reports.

Defaults: `cloud.enabled = false`, `api.bind = 127.0.0.1:8080`,
`control.tick_interval_seconds = 30`, `control.command_ttl_seconds = 120`.

## Acceptance criteria

- [ ] Defaults load with no file and no env.
- [ ] A TOML file overrides defaults; env overrides the file; flags override env.
- [ ] `RHIZO_EDGE__MQTT__BROKER_URL` reaches `config.mqtt.broker_url`.
- [ ] An invalid value exits non-zero naming the key.
- [ ] `format!("{:?}", config)` contains `[redacted]` and no secret.
- [ ] A `password` key in the TOML logs a warning and is ignored.
- [ ] `cloud.enabled` defaults to `false`.

## Verification

```bash
cargo test -p edge-controller config::
cargo run -p edge-controller -- --config /nonexistent/bad.toml  # exits non-zero
```

## Tests required

- Layer precedence, one test per layer pair.
- Redaction in `Debug`.
- Invalid value exits with the key named.
- Password-in-file warning.

## Documentation impact

- `.env.example` updated with every variable.

## Files likely affected

```text
crates/edge-controller/src/config.rs
crates/edge-controller/src/main.rs
.env.example
deploy/edge/edge.toml
```
