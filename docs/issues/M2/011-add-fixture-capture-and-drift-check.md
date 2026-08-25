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

- [ ] `--capture-fixtures` produces one file per published kind.
- [ ] The drift check passes against the current corpus.
- [ ] Adding a field to a payload without updating the corpus fails the check.
- [ ] The check ignores ids, timestamps, and float noise.
- [ ] It runs in CI.

## Verification

```bash
cargo run -p device-simulator -- --device-id plant-node-01 --capture-fixtures /tmp/cap --duration 60
python tools/check_fixture_drift.py /tmp/cap test/fixtures/protocol/valid
```

## Tests required

- The drift check against a deliberately modified payload.

## Documentation impact

- test/fixtures/protocol/README.md notes the drift check.

## Files likely affected

```text
crates/device-simulator/src/capture.rs
tools/check_fixture_drift.py
.github/workflows/ci.yml
```
