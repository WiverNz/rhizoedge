# Issue M6-002 — Implement the safety gate

**Milestone:** M6 · **PRD:** [PRD 060](../../prd/060-irrigation-control-and-safety.md) · **Depends on:** M6-001

## Context

**The most safety-critical function in the project.** ADR-006: the gate runs
first, always, from the single entry point, with exhaustive matches and no
catch-all arm — so a new enum variant fails to compile until someone classifies
it.

## Goal

Implement the ordered safety gate with compile-time exhaustiveness.

## Scope

- `safety_gate(&IrrigationInputs) -> Option<LockoutReason>`
- Ordered: leak, tank, sample validity, sample freshness, daily cap
- **Exhaustive matches with no `_ =>` arm on any safety input**
- `None` and `Unknown` map to a lockout, never to permission
- Called at the top of `evaluate`, from nowhere else

## Non-goals

- Mode-specific relaxation (M6-005 handles the manual exception).

## Dependencies

- M6-001

## Implementation notes

The no-catch-all rule is the compile-time half of SAFETY-012. Write the
matches so that adding a `LeakState::Degraded` variant later breaks the build
until someone decides what it means.

`evaluate` must be the only public entry point, and the gate must be its first
statement. A second decision function that skipped the gate would be
undetectable in review.

Order: leak → leak-unknown → tank-unknown → tank-low → tank-stale → sample-absent
→ sample-invalid → sample-stale → daily-cap.

## Acceptance criteria

- [ ] Each lockout reason is returned under its documented condition.
- [ ] `leak = Unknown` returns a lockout, not `None`.
- [ ] `tank = None` returns a lockout.
- [ ] `latest_soil = None` returns a lockout.
- [ ] **No `_ =>` arm exists on any safety match.**
- [ ] Adding an enum variant fails to compile until classified.
- [ ] The gate is the first statement in `evaluate`.
- [ ] There is no other public decision function.

## Verification

```bash
cargo test -p rhizo-domain safety_gate::
cargo test safety_012
grep -n '_ =>' crates/domain/src/irrigation/gate.rs   # expect no safety matches
```

## Tests required

- One test per lockout reason in isolation.
- **`safety_012_missing_input_never_waters`** as a property test over inputs with random fields None.
- A compile-fail test (trybuild) for an unclassified new variant.

## Documentation impact

- Doc comment citing SAFETY-003, -004, -005, -006, -012.

## Files likely affected

```text
crates/domain/src/irrigation/gate.rs
```
