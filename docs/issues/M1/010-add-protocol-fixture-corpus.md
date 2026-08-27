# Issue M1-010 — Create the protocol fixture corpus

**Milestone:** M1 · **PRD:** [PRD 010](../../prd/010-domain-and-mqtt-protocol.md) · **Depends on:** M1-006, M1-007, M1-008

## Context

ADR-008 uses a shared fixture corpus, run by both the host and firmware
workspaces, to detect protocol divergence. Versioning-policy makes fixtures
append-only: a fixture committed for v1 must decode for as long as v1 is
supported.

## Goal

Create the canonical valid and invalid message fixtures and the tests that run them.

## Scope

- `test/fixtures/protocol/valid/` — one per message kind plus partial/forward-compatible cases
- `test/fixtures/protocol/invalid/` — unknown version, device mismatch, out-of-range, missing field, topic injection
- A fixture test in the contract crate that decodes and re-encodes every valid file
- A test asserting every invalid file fails with its documented variant
- A README stating the append-only rule

## Non-goals

- The firmware-side test runner (M9-003).
- Drift capture from the simulator (M2-011).

## Dependencies

- M1-006
- M1-007
- M1-008

## Implementation notes

Valid fixtures must re-encode to an *equivalent* value, not to byte-identical
JSON — key order and float formatting are not part of the contract.

Each invalid fixture is paired with the `DecodeError` variant it must produce,
so the test asserts the specific failure rather than merely 'it failed'.

The README matters: a future contributor's instinct on a failing fixture is to
edit it, which would silently break compatibility.

## Acceptance criteria

- [x] At least twelve valid fixtures covering all twelve message kinds.
- [x] At least five invalid fixtures.
- [x] Every valid fixture decodes into its **concrete payload type**, not a
      generic `Value`, and re-encodes equivalently — every wire field it states
      survives the round trip through that type.
- [x] Every invalid fixture fails with its documented variant.
- [x] The README states the append-only rule.
- [x] The test discovers files automatically — adding a fixture needs no code change.

## Verification

```bash
cargo test -p rhizo-mqtt-contract --test fixtures
```

## Tests required

- Directory-driven decode/re-encode test.
- Directory-driven rejection test.

## Documentation impact

- test/fixtures/protocol/README.md.

## Files likely affected

```text
test/fixtures/protocol/valid/*.json
test/fixtures/protocol/invalid/<expected_variant>/*.json
test/fixtures/protocol/README.md
crates/mqtt-contract/tests/fixtures.rs
```
