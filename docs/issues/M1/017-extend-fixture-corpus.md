# Issue M1-017 — Extend the protocol fixture corpus for the new payloads

**Milestone:** M1 · **PRD:** [PRD 010](../../prd/010-domain-and-mqtt-protocol.md) · **Depends on:** M1-010, M1-014, M1-015

## Context

The fixture corpus is the mechanism that keeps the simulator and the firmware
honest about the wire format
([ADR-008](../../adr/008-shared-code-simulator-and-firmware.md)). The batched
telemetry, capabilities, policy, and event payloads all need coverage before M2
starts consuming them.

## Goal

Cover every v1 payload with valid and invalid fixtures.

## Scope

- `valid/`: telemetry batch (full and partial), status with capabilities, status without actuators, policy set, policy disabled, event replay batch, gap event
- `invalid/`: empty sample batch, unit/kind mismatch, unknown measurement kind (must be **accepted**, so it belongs in `valid/`), policy with dose above the hard limit, policy with `resume_above <= trigger_below`, event batch with a duplicate `event_id`
- Fixture README updated with the append-only rule reaffirmed

## Non-goals

- Firmware-side fixture running (M9-003 already covers it).

## Dependencies

- M1-010
- M1-014
- M1-015

## Implementation notes

The unknown-measurement-kind case is deliberately a **valid** fixture, not an
invalid one: an older receiver must store an unrecognised kind as advisory rather
than reject the batch ([ADR-017](../../adr/017-extensible-measurement-model.md)).
Putting it in `invalid/` would encode the opposite behaviour and quietly make
forward compatibility a lie.

A status fixture with an empty `actuators` array is required, because
monitoring-only devices are a first-class case (SAFETY-018) and the parser must
not treat the absence as an error.

## Acceptance criteria

- [x] Every new payload kind has at least one valid fixture.
- [x] An unknown measurement kind decodes successfully and is marked advisory.
- [x] A status with no actuators parses cleanly.
- [x] Each invalid fixture fails with its documented error variant.
- [x] The corpus is discovered automatically — adding a file needs no code change.
- [x] The README states the append-only rule.

## Verification

```bash
cargo test -p rhizo-mqtt-contract fixtures::
```

## Tests required

- Directory-driven decode and re-encode.
- Directory-driven rejection.
- An explicit test that an unknown kind is advisory, not rejected.

## Documentation impact

- test/fixtures/protocol/README.md.

## Files likely affected

```text
test/fixtures/protocol/valid/*.json
test/fixtures/protocol/invalid/*.json
crates/mqtt-contract/tests/fixtures.rs
```
