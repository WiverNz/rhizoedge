# Issue M9-014 — Add the simulator/firmware conformance test

**Milestone:** M9 · **PRD:** [PRD 090](../../prd/090-esp32-rust-firmware.md) · **Depends on:** M9-011, M9-013

## Context

ADR-008's mechanism 5, and the one that catches behavioural divergence the
type system cannot: the same scenario driven against the simulator and against
firmware-with-fakes must produce identical published message sequences.

## Goal

Prove the simulator and firmware behave identically at the protocol level.

## Scope

- A shared scenario script
- Run against the simulator and against the firmware app with fake adapters
- Compare published message sequences **modulo ids and timestamps**
- Cover: telemetry, status, config apply, command accept, command reject, duplicate command, interrupted dose

## Non-goals

- Timing equivalence — the physical models differ deliberately.

## Dependencies

- M9-011
- M9-013

## Implementation notes

Compare sequences of (topic, kind, key fields), not raw bytes. Ids and
timestamps necessarily differ; a byte comparison would fail always and prove
nothing.

The reject cases matter most: an oversized command, an expired command, and a
duplicate must produce the same reason from both, because that is what makes
M6's simulator-based safety tests transfer to hardware.

## Acceptance criteria

- [ ] The scenario runs against both implementations.
- [ ] Published sequences match modulo ids and timestamps.
- [ ] Command reject reasons are identical for every refusal case.
- [ ] A deliberate divergence fails the test.
- [ ] It runs on the host with no board.

## Verification

```bash
cd firmware/esp32-node && cargo test conformance::
```

## Tests required

- The conformance scenario.
- A negative check with an injected divergence.

## Documentation impact

- None.

## Files likely affected

```text
firmware/esp32-node/tests/conformance.rs
test/scenarios/conformance_script.rs
```
