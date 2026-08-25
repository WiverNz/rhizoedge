# Issue M2-014 — Implement accelerated virtual time

**Milestone:** M2 · **PRD:** [PRD 020](../../prd/020-device-simulator.md) · **Depends on:** M2-004, M2-009

## Context

ADR-013 specifies accelerated time so a multi-hour watering cycle becomes a
six-second test. Without it, the end-to-end suite is not something anyone runs.

## Goal

Run the whole simulator on a scalable virtual clock.

## Scope

- `AcceleratedClock`: `anchor_real + (real_now - anchor_real) * scale`
- `--time-scale`, default 1.0
- Every timer, model step, and timestamp derived from it
- The scale reported at startup and by `GET /sim/scale`
- One clock per process

## Non-goals

- Edge-side virtual time (M3) — configured separately, asserted to agree in M8-004.

## Dependencies

- M2-004
- M2-009

## Implementation notes

The anchor is a real epoch instant so virtual timestamps remain plausible
UTC values that store and chart normally.

Nothing may call `Utc::now()` directly once this lands — a single stray call
produces a component that ages at a different rate from the rest of the process,
and the resulting bug is extremely confusing.

At `--time-scale 600`, ten simulated minutes pass per real second.

## Acceptance criteria

- [ ] `--time-scale 600` makes a 15-minute absorption wait take ~1.5 s.
- [ ] Published timestamps are plausible UTC values.
- [ ] `--time-scale 1` behaves as real time.
- [ ] `GET /sim/scale` reports the factor.
- [ ] `grep -rn 'Utc::now' crates/device-simulator/src` returns only the clock implementation.
- [ ] A full dose/absorption/recheck sequence completes in under 10 s at scale 600.

## Verification

```bash
cargo test -p device-simulator clock::
time cargo run -p device-simulator -- --device-id plant-node-01 --time-scale 600 --duration 10
```

## Tests required

- Scale arithmetic.
- Timer acceleration.
- A grep-based test or lint asserting no direct clock calls.

## Documentation impact

- None.

## Files likely affected

```text
crates/device-simulator/src/clock.rs
```
