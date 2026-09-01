# Issue M16-011 — Integrate delivery outcomes with the budget and lockouts

**Milestone:** M16 · **PRD:** [PRD 160](../../prd/160-verified-watering.md) · **Depends on:** M16-010

## Context

This is the one issue in M16 that changes safety arithmetic, and it should be
reviewed as such. Everything before it records evidence; this decides what the
evidence costs.

The temptation to resist is obvious: a witness reporting 28 ml for a 40 ml dose
looks like 12 ml of budget the plant did not spend. Trusting that number in the
permissive direction means a drifting or broken sensor reading low becomes a
licence to water more — which converts a verification feature into a way around
SAFETY-006.

## Goal

Budget accounting and lockouts for every delivery outcome, tightening one rule
and loosening none.

## Scope

- `budget::credited_ml` extended: when both `estimated_ml` and `measured_ml`
  exist, charge `max(estimated_ml, measured_ml)`.
- `OutcomeUnknown` charges the full `effective_ml`.
- Explicit-clear lockouts for `NoFlow`, `UnexpectedFlow`, and `OverDelivery`,
  added to `LockoutReason`.
- `UnexpectedFlow` locks **every** plant bound to the actuator, not only the one
  being watered.
- `creates_watering_event` unchanged: only a delivered outcome asserts that water
  reached the plant.
- No retry, in any mode, for any failed delivery.

## Non-goals

- **Reducing any charge.** `NoFlow` still charges the full `effective_ml` even
  though the witness says nothing moved. That is deliberately wasteful and
  deliberately safe; PRD 160 §Open questions 6 records the case for trusting a
  verified zero as needing its own ADR.
- Changing the rolling window, the cap, the cooldown, the cycle dose limit, any
  clamp, or any firmware ceiling.
- Auto-clearing any new lockout.

## Dependencies

- M16-010

## Implementation notes

`max(estimated, measured)` is the entire arithmetic change and it can only ever
charge **more**. Write it that way explicitly — not as a conditional preference
for the measured value — so a reviewer can see in one line that the permissive
direction is unreachable. The property test is the real guarantee: for arbitrary
evidence, the charge is never below what the same result would be charged today
without a witness.

`no_delivery_detected` is **not** replaced. It answers a different question — did
the plant respond — over a different timescale, and it still fires on two
unresponsive doses. Verified Watering reaches the hydraulic conclusion on the
first dose; the biological check stays where it is. Where they now disagree is
informative: a *verified* delivery with no soil response means the water reached
the pot and not the probe, which is a placement or wrong-place fault rather than
a pump fault. Recording that distinction is future work, and this issue must not
collapse the two.

New `LockoutReason` variants go through the existing explicit-clear machinery
(F-060-41). None auto-clears, for M11-003's reason: a hardware fault needs
hands, and a lockout that clears when the symptom passes lets a failing
component oscillate.

`UnexpectedFlow` locking every bound plant is the one place this issue widens a
blast radius, deliberately. Unauthorised water movement is a property of the
hydraulic path, not of one plant's schedule, and the plant that is quietly being
flooded is not necessarily the one whose command was in flight.

## Acceptance criteria

- [ ] `credited_ml` charges `max(estimated_ml, measured_ml)` when both exist.
- [ ] No outcome charges less than today's rules would.
- [ ] `OutcomeUnknown` charges the full `effective_ml`.
- [ ] `NoFlow`, `UnexpectedFlow`, and `OverDelivery` set explicit-clear lockouts.
- [ ] `UnexpectedFlow` locks every plant bound to the actuator.
- [ ] No new lockout auto-clears.
- [ ] `creates_watering_event` is unchanged.
- [ ] No failed delivery is retried, including in offline autonomous mode.
- [ ] The rolling window, cap, cooldown, cycle limit, and every clamp are
      unchanged.
- [ ] `no_delivery_detected` is unchanged and still fires on its own rule.

## Verification

```bash
cargo test -p rhizo-domain irrigation::budget
cargo test -p rhizo-domain delivery::
cargo test safety_006
cargo test safety_
cargo test -p edge-controller delivery::budget
```

## Tests required

- Property: for arbitrary evidence, the charge is finite and never below the
  pre-M16 charge for the same result.
- Property: no witness value can raise a plant's 24-hour total above
  `max_daily_ml`.
- Each new lockout: set, persist, refuse to auto-clear, and clear explicitly.
- `UnexpectedFlow` across several plants bound to one actuator.
- Budget totals after each uncertain outcome.

## Documentation impact

- `docs/architecture/safety-invariants.md`: SAFETY-006's note on the
  `max(estimated, measured)` rule.
- PRD 060 §Data model: the new lockout reasons.
- `docs/prd/160-verified-watering.md` §Failure modes, if a charge deviates.

## Files likely affected

```text
crates/domain/src/irrigation/budget.rs
crates/domain/src/state.rs
crates/domain/src/delivery/mod.rs
crates/edge-controller/src/control/irrigation.rs
docs/architecture/safety-invariants.md
docs/prd/060-irrigation-control-and-safety.md
```
