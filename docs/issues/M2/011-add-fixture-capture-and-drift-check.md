# Issue M2-011 — Add fixture capture and drift detection

**Milestone:** M2 · **PRD:** [PRD 020](../../prd/020-device-simulator.md) · **Depends on:** M2-006, M1-010

## Context

ADR-008 identifies fixture rot as a risk: fixtures stop reflecting what the
simulator actually publishes, and the corpus quietly stops proving anything.

## Goal

Detect drift between real simulator output and the committed fixtures.

## Scope

- `--capture-fixtures <dir>` writing one file per message kind
- A CI check diffing a capture against `test/fixtures/protocol/valid/`
- Structural comparison ignoring ids, timestamps, and float noise

## Non-goals

- Auto-updating fixtures — a drift is a decision, not a fix.

## Dependencies

- M2-006
- M1-010

## Implementation notes

Compare structure and field presence, not values: `message_id` and
timestamps differ every run, and comparing them would make the check useless.

A drift must **fail** rather than rewrite the fixture. Versioning-policy makes
fixtures append-only precisely because the instinct on a failure is to edit
them.

## Acceptance criteria

- [x] `--capture-fixtures` produces one file per published kind.
- [x] The drift check passes against the current corpus.
- [x] Adding a field to a payload without updating the corpus fails the check.
- [x] The check ignores ids, timestamps, and float noise.
- [x] It runs in CI.

## Verification

```bash
cargo run -p device-simulator -- --device-id plant-node-01 --capture-fixtures /tmp/cap --duration 60
cargo test -p device-simulator --test fixture_drift
```

**Deviation, deliberate.** The drift check is `crates/device-simulator/tests/fixture_drift.rs`
rather than `tools/check_fixture_drift.py`, for the same reason `rhizo-docscheck`
is Rust: it runs inside `cargo test` with no second toolchain in CI, it cannot
drift from the capture code it checks, and it needs no broker. `--capture-fixtures`
is unchanged and writes the same files — it is how a person inspects a drift once
the test reports one.

Running it found a real corpus gap: `actuator.json` and `command-result.json`
described device→edge messages without `boot_id`, `sequence`, `device_time_ms`,
or `clock_synced`, which protocol §4 requires. Two fixtures were **added** —
`actuator-running.json` and `command-result-interrupted.json` — rather than the
existing ones edited, because the corpus is append-only.

## Tests required

- The drift check against a deliberately modified payload.

## Documentation impact

- test/fixtures/protocol/README.md notes the drift check.

## Files likely affected

```text
crates/device-simulator/src/capture.rs
crates/device-simulator/tests/fixture_drift.rs
test/fixtures/protocol/valid/actuator-running.json
test/fixtures/protocol/valid/command-result-interrupted.json
.github/workflows/ci.yml
```
