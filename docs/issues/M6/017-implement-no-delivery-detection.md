# Issue M6-017 — Implement no-delivery detection

**Milestone:** M6 · **PRD:** [PRD 060](../../prd/060-irrigation-control-and-safety.md) · **Depends on:** M6-010

## Context

Failure-model 5.1: a pump that runs but delivers nothing (air lock, kinked
tube, dry line) is the most damaging plausible failure in this class of system,
because the naive response is to escalate doses.

## Goal

Stop escalating when doses produce no response.

## Scope

- After a dose, check moisture against `recovery_delta_vwc` and, where a scale exists, weight
- Two consecutive doses with no moisture **and** no weight response: `Lock(NoDeliveryDetected)`
- Requires an **explicit** operator clear
- `watering_failures_total{reason="no_delivery"}` and an alert-level log

## Non-goals

- Diagnosing the physical cause.

## Dependencies

- M6-010

## Implementation notes

Requiring both signals to be absent avoids false positives: soil near field
capacity may not show a moisture rise, but the pot weight will. Where no scale
is fitted, moisture alone is used and the threshold is necessarily less certain.

Explicit clear rather than auto-clear: reaching this state means the physical
system is wrong, and resuming automatically would repeat the mistake.

## Acceptance criteria

- [ ] Two doses with no moisture and no weight response lock the plant.
- [ ] One unresponsive dose does not.
- [ ] A weight rise without a moisture rise does **not** trigger it.
- [ ] The lockout requires an explicit clear.
- [ ] The failure counter and an alert-level log are produced.
- [ ] Escalation stops — no third dose is issued.

## Verification

```bash
cargo test -p rhizo-domain no_delivery::
cargo test --test integration pump_no_delivery
```

## Tests required

- Two-dose threshold.
- Weight-only response suppresses detection.
- Explicit clear required.
- SCEN-044.

## Documentation impact

- None.

## Files likely affected

```text
crates/domain/src/irrigation/no_delivery.rs
```
