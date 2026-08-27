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

- [x] `--time-scale 600` makes a 15-minute absorption wait take ~1.5 s.
- [x] Published timestamps are plausible UTC values.
- [x] `--time-scale 1` behaves as real time.
- [x] `GET /sim/scale` reports the factor.
- [x] `grep -rn 'Utc::now' crates/device-simulator/src` returns only the clock implementation.
- [x] A full dose/absorption/recheck sequence completes in under 10 s at scale 600.

## Verification

```bash
cargo test -p device-simulator --lib clock::
cargo test -p device-simulator --test integration a_full_cycle_completes_in_under_ten_seconds_at_scale_six_hundred
cargo test -p device-simulator --test single_actuation_path nothing_outside_the_clock_module_reads_a_clock
```

`grep -rn 'Utc::now' crates/device-simulator/src` returns **nothing at all**,
which is stronger than "only the clock implementation": the device's wall clock
comes solely from `edge.time`, so the simulator has no wall-clock dependency to
call. The structural test asserts that, and additionally that the only
`Instant::now` outside `clock.rs` is the MQTT shutdown drain — a *network*
timeout that must stay in real time, since a 5 s drain scaled by 600 would be
8 ms and would turn a clean stop back into a will.

The broker-backed cycle test measures wall time directly: dose, absorption, and
recheck complete in about **1.4 s** at `--time-scale 600`.

## Tests required

- Scale arithmetic.
- Timer acceleration.
- A grep-based test or lint asserting no direct clock calls.

## Documentation impact

- `docs/testing/simulator-strategy.md` §4: the scaled quantity is the device's
  *monotonic* clock, with wall time derived from the last `edge.time` — after
  the 2026-08-26 pass a device has no wall clock of its own to anchor. Also
  records that a tick is applied in bounded sub-steps so the physical model
  behaves identically at every scale.

## Files likely affected

```text
crates/device-simulator/src/clock.rs
```
