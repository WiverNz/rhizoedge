# Issue M2-010 — Assert no retained messages on command or telemetry topics

**Milestone:** M2 · **PRD:** [PRD 020](../../prd/020-device-simulator.md) · **Depends on:** M2-008

## Context

ADR-002 calls a retained command topic the single most damaging mistake
available in this protocol: the broker would redeliver it on every reconnect
indefinitely, causing repeated watering. It is easy to introduce while debugging.

The `time` topic is the same trap in a subtler form. A retained `edge.time` would
set every reconnecting device's clock back to the publication instant, which makes
long-expired commands look valid and silently defeats SAFETY-002
([mqtt-v1.md](../../protocol/mqtt-v1.md) §5.12).

## Goal

Make the mistake impossible to commit unnoticed.

## Scope

- An integration test running a full command cycle, then subscribing fresh
- Assert retained messages exist on `status` and `config`; M2-016 adds the
  retained-policy assertion once policy handling exists
- Assert **no** retained message on `telemetry`, `actuator`, `events`, any
  `commands/*`, or `time`, following the normative MQTT retention table

## Non-goals

- Enforcing it in the broker — Mosquitto has no such control.

## Dependencies

- M2-008

## Implementation notes

Subscribe with a fresh client after the cycle completes and collect
everything delivered before the first live message. Anything arriving on a
command, telemetry, event or time topic in that window is retained state and
fails the test.

This is SCEN-015.

## Acceptance criteria

- [ ] The test runs a command cycle and then subscribes fresh.
- [ ] Retained `status` and `config` are received.
- [ ] Nothing is received on `commands/*`.
- [ ] Nothing is received on `telemetry` or `actuator`.
- [ ] Nothing is received on `events`.
- [ ] Nothing is received on `time`, after at least one `edge.time` has been
      published in the cycle — so the assertion cannot pass vacuously.
- [ ] Deliberately setting retain on a command publish fails the test.
- [ ] Deliberately setting retain on an `edge.time` publish fails the test.

## Verification

```bash
cargo test -p device-simulator --test integration retained_topics
```

## Tests required

- SCEN-015.
- Negative check: set retain on a command, confirm the test fails, revert.
- Negative check: set retain on `edge.time`, confirm the test fails, revert.

## Documentation impact

- None.

## Files likely affected

```text
crates/device-simulator/tests/integration.rs
```
