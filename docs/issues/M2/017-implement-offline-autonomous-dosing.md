# Issue M2-017 — Implement offline autonomous evaluation and dosing

**Milestone:** M2 · **PRD:** [PRD 020](../../prd/020-device-simulator.md) · **Depends on:** M2-016, M2-007

## Context

The simulator must behave like an isolated device: evaluate the persisted policy
on monotonic time and deliver bounded doses through the same actuation gate as a
commanded dose.

## Goal

Make the simulator capable of true offline autonomy, with no relaxed rules.

## Scope

- On loss of MQTT, switch to offline evaluation using `rhizo_policy::evaluate_offline`
- Feed it locally sampled measurements and the persisted `OfflineState`
- Actuate through the **existing** `validate_water_command` path — no second actuation route
- Persist budget, cooldown remaining, confirm elapsed, and dose count
- On reboot assume no time passed: cooldown resumes from its remainder, budget is not replenished

## Non-goals

- A second copy of the offline rules — the evaluator lives in `rhizo-policy`.

## Dependencies

- M2-016
- M2-007

## Implementation notes

There must be **exactly one** actuation call site in the simulator. Offline
dosing routes into the same `validate_water_command` gate that commands use, so
the hard limits, leak veto, tank veto, and pump-fault veto apply identically. An
offline path that bypassed it would make every M6 safety test meaningless
([ADR-008](../../adr/008-shared-code-simulator-and-firmware.md)).

`cooldown_remaining_ms` is stored as a **remaining duration**, never a deadline.
An isolated device may have no wall clock to interpret a deadline against, and a
reboot must never shorten a cooldown (SAFETY-015).

## Acceptance criteria

- [ ] An isolated simulator with a valid enabled policy waters within its bounds.
- [ ] An isolated simulator with no policy **never** waters.
- [ ] Hysteresis prevents repeated dosing at the threshold.
- [ ] Reboot during cooldown resumes the remaining duration; it is never shortened.
- [ ] Reboot does not replenish `budget_used_ml`.
- [ ] `grep -c validate_water_command` still shows exactly one call site.
- [ ] Every refusal reason is produced under its documented condition.

## Verification

```bash
cargo test -p device-simulator offline::
cargo test safety_013 safety_014 safety_015
grep -rn 'validate_water_command' crates/device-simulator/src | wc -l
```

## Tests required

- SCEN-091, SCEN-093, SCEN-096, SCEN-097, SCEN-098, SCEN-099, SCEN-105.
- Single-call-site assertion.

## Documentation impact

- None.

## Files likely affected

```text
crates/device-simulator/src/offline.rs
```
