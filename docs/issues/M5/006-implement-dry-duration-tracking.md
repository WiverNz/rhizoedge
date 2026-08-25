# Issue M5-006 — Implement dry-duration tracking

**Milestone:** M5 · **PRD:** [PRD 050](../../prd/050-plant-model-and-recommendations.md) · **Depends on:** M5-005

## Context

The `dry_confirm_minutes` debounce is what separates `Drying` from
`DryConfirmed`. It exists so a momentary dip does not trigger a dose.

## Goal

Track continuous time below the target minimum.

## Scope

- Accumulate time below `target_min`
- **Reset on any valid sample at or above it**
- A gap in samples does not silently accumulate dryness
- Persisted so a restart does not lose or fabricate duration

## Non-goals

- The state transition itself (M6-006).

## Dependencies

- M5-005

## Implementation notes

The gap case is the subtle one. If samples stop for two hours and resume dry,
those two hours must not count as confirmed dryness — we do not know what
happened. Track duration from observed samples, and treat a gap longer than the
staleness threshold as a reset.

Persisting it means a restart mid-debounce neither loses progress nor invents it.

## Acceptance criteria

- [ ] Continuous dryness accumulates correctly.
- [ ] One sample at or above target resets it.
- [ ] A sample gap longer than the staleness threshold resets it.
- [ ] It survives a restart.
- [ ] An invalid sample neither accumulates nor resets.

## Verification

```bash
cargo test -p rhizo-domain dry_duration::
```

## Tests required

- Accumulation.
- Reset on recovery.
- **Gap does not accumulate.**
- Restart persistence.
- Invalid sample handling.

## Documentation impact

- None.

## Files likely affected

```text
crates/domain/src/dry_duration.rs
crates/storage/src/repo/plant.rs
```
