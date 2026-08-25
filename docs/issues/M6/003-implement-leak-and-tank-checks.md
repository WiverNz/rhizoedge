# Issue M6-003 — Implement the leak and tank gate checks

**Milestone:** M6 · **PRD:** [PRD 060](../../prd/060-irrigation-control-and-safety.md) · **Depends on:** M6-002

## Context

SAFETY-003 and SAFETY-004. The leak check is unusual in that it blocks
**manual** watering too: the operator who would click 'water anyway' is exactly
the person who has not yet looked at the floor.

## Goal

Implement the first two gate checks with their sticky-lockout semantics.

## Scope

- Leak detected: lockout in **all** modes including manual
- Leak unknown: lockout (SAFETY-012)
- Tank at or below `tank_min_percent`: lockout
- Tank unknown or stale: lockout
- Leak lockout requires an **explicit** operator reset after the signal clears
- Tank lockout clears automatically when refilled

## Non-goals

- The clear endpoint (M6-016).

## Dependencies

- M6-002

## Implementation notes

The clearing asymmetry is deliberate and must be encoded: leak is
explicit-clear, tank is auto-clear. A leak that dries out does not silently
re-enable watering, because the underlying cause (a burst joint, a cracked pot)
has not necessarily been fixed.

Implement leak as a state with an `AwaitingReset` phase distinct from `Clear`.

## Acceptance criteria

- [ ] A detected leak blocks automatic **and** manual watering.
- [ ] An unknown leak state blocks.
- [ ] A low tank blocks; a refill clears it automatically.
- [ ] An unknown tank level blocks.
- [ ] A cleared leak signal moves to `AwaitingReset`, not `Clear`.
- [ ] Only an explicit reset with the signal absent returns to `Clear`.

## Verification

```bash
cargo test -p rhizo-domain gate::leak gate::tank
cargo test safety_003 safety_004
```

## Tests required

- `safety_003_leak_blocks_all_modes` property test over random states and modes.
- `safety_004_tank_unknown_or_low_blocks` property test.
- AwaitingReset semantics.
- Tank auto-clear.

## Documentation impact

- None.

## Files likely affected

```text
crates/domain/src/irrigation/gate.rs
```
