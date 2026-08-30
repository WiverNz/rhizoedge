# Issue M6-007 — Implement the rolling 24-hour water cap

**Milestone:** M6 · **PRD:** [PRD 060](../../prd/060-irrigation-control-and-safety.md) · **Depends on:** M6-006, M3-003

## Context

SAFETY-006, and the last line of defence against a logic bug anywhere else in
the machine. ADR-013: rolling, not calendar-day, because a midnight boundary
would permit two full daily allowances a few hours apart.

## Goal

Compute and enforce the rolling daily cap from persisted rows.

## Scope

- Sum `watering_events.delivered_ml` over `now - 24h`
- **Derived from rows, never a counter** — a restart cannot reset it
- `mode IN ('automatic','recommended')`; `manual` and `detected` excluded
- Checked before issuing, and again with the dose included
- `interrupted` and `failed` credit `requested_ml` conservatively

## Non-goals

- The device-side daily cap, which is separate and counts everything (M9-011).

## Dependencies

- M6-006
- M3-003

## Implementation notes

Deriving from rows rather than maintaining a counter is what makes SAFETY-006
survive restarts, crashes, and clock steps for free. A counter would need its
own persistence, its own recovery, and its own bugs.

Crediting the full `requested_ml` for an interrupted dose over-counts when the
interruption was early. That is deliberate: over-counting reduces the next dose,
under-counting could permit an extra one. The conservative direction is the safe
one.

Check twice — before selecting a dose and again with the dose added — so a dose
that would cross the cap is not issued at all.

## Acceptance criteria

- [x] The sum comes from `watering_events`, with no counter anywhere.
- [x] Only automatic and recommended modes count.
- [x] A restart does not reset the total.
- [x] A dose that would cross the cap is not issued.
- [x] `interrupted` credits the full `requested_ml`.
- [x] **`safety_006_rolling_24h_cap_never_exceeded` passes at 10 000 cases.**

## Verification

```bash
cargo test -p rhizo-domain daily_cap::
PROPTEST_CASES=10000 cargo test safety_006
```

## Tests required

- **`safety_006_rolling_24h_cap_never_exceeded`** — the flagship property test, generating adversarial histories with restarts, clock steps, and interrupted doses.
- Mode exclusion.
- Restart survival.
- Conservative interrupted crediting.

## Documentation impact

- None.

## Files likely affected

```text
crates/domain/src/irrigation/budget.rs
crates/storage/src/repo/watering.rs
```
