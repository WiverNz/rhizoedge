# Issue M9-018 — Implement monotonic budget and cooldown persistence

**Milestone:** M9 · **PRD:** [PRD 090](../../prd/090-esp32-rust-firmware.md) · **Depends on:** M9-016

## Context

SAFETY-015: a reboot must never replenish the water budget or shorten a cooldown.
An isolated device has no trustworthy wall clock, so both are tracked against the
monotonic timer and persisted conservatively.

## Goal

Make the offline budget and cooldown survive reboots in the safe direction.

## Scope

- Persist `budget_used_ml` and `window_started_monotonic`
- Persist `cooldown_remaining_ms` as a **remaining duration**, never a deadline
- On boot without a trustworthy wall clock, assume no time has passed
- Replenish the budget only from observed monotonic elapsed time, or from a trusted wall clock
- Accept an authoritative budget baseline from the edge after reconciliation
- Handle monotonic counter overflow safely

## Non-goals

- The evaluator's use of these values (M9-016).

## Dependencies

- M9-016

## Implementation notes

Storing the cooldown as a remaining duration rather than a deadline is the whole
trick. A deadline is meaningless to a device that cannot interpret absolute time
after a reboot; a remainder is always interpretable and always conservative.

"Assume no time passed" is deliberately pessimistic. A device power-cycling every
few minutes therefore never earns budget, which is exactly right: a reboot loop is
not evidence that a day went by.

After reconciliation the edge pushes back its row-derived budget, which is
authoritative. Accept it and reset the local accumulator to match.

## Acceptance criteria

- [ ] A reboot does not replenish `budget_used_ml`.
- [ ] A reboot does not shorten a cooldown.
- [ ] The budget replenishes as the window genuinely advances.
- [ ] A trusted wall clock, once Edge synchronisation is re-established, allows correct window advancement.
- [ ] The edge's post-reconciliation baseline is accepted.
- [ ] Monotonic overflow does not grant budget or clear a cooldown.
- [ ] Host tests cover repeated reboots at random points.

## Verification

```bash
cd firmware/esp32-node && cargo test budget::
cargo test safety_015
```

## Tests required

- Reboot-does-not-replenish property test.
- Cooldown remainder preservation.
- Overflow safety.
- Baseline acceptance.

## Documentation impact

- None.

## Files likely affected

```text
firmware/esp32-node/src/app/budget.rs
```
