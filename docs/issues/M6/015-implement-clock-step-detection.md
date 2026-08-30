# Issue M6-015 — Implement edge clock step detection

**Milestone:** M6 · **PRD:** [PRD 060](../../prd/060-irrigation-control-and-safety.md) · **Depends on:** M6-007

## Context

[ADR-013](../../adr/013-clock-and-time-semantics.md) §"Edge clock steps": a
forward step on the **edge host** — which is genuinely NTP-synced, unlike the
devices — drops older watering events out of the rolling window early,
potentially permitting an extra dose. The asymmetry is deliberate: backwards is
safe, forwards locks out.

## Goal

Detect wall-clock steps and respond conservatively.

## Scope

- Sample `Instant` alongside the wall clock each tick
- Divergence beyond threshold is a step
- **Forward step > 10 minutes: all plants `Lock(Uncertain)` for one cooldown**
- Backward step: logged only; the window naturally becomes conservative
- A `clock_step` event recorded in both cases

## Non-goals

- Device clock skew, which is a separate condition (M4).
- Device time synchronisation, which does not use NTP at all (M4-001, M4-004).

## Dependencies

- M6-007

## Implementation notes

The monotonic reference is what makes detection possible: comparing the wall
clock against itself cannot reveal a step.

Locking out on a forward step is heavy-handed and correct. The alternative is
accepting that the daily cap can be bypassed by an NTP correction, which is
exactly the class of subtle safety hole SAFETY-012 exists to close.

A backward step makes the window include more history, so the cap becomes more
conservative — safe, and logged for diagnosis only.

## Acceptance criteria

- [x] A forward step beyond 10 minutes is detected.
- [x] It places all plants in `Lock(Uncertain)` for one cooldown.
- [x] A backward step is logged but causes no lockout.
- [x] A `clock_step` event is recorded with the direction and magnitude.
- [x] Normal clock drift does not trigger it.
- [x] The lockout clears after the cooldown.

## Verification

```bash
cargo test -p edge-controller clock_step::
cargo test --test integration clock_forward_step
```

## Tests required

- SCEN-071 forward step.
- SCEN-072 backward step.
- No false positive on drift.
- Lockout expiry.

## Documentation impact

- None.

## Files likely affected

```text
crates/edge-controller/src/control/clock_step.rs
```
