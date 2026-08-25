# Issue M14-002 — Document the connectivity assumptions that break at field scale

**Milestone:** M14 · **PRD:** [PRD 140](../../prd/140-field-readiness.md) · **Depends on:** M14-001

## Context

PRD 140's central finding: five of the six high-severity breakages are one
problem wearing five hats — **the V1 architecture assumes a device that is
awake**, and that assumption is load-bearing for Last Will, command TTL, and
therefore SAFETY-002.

## Goal

Make the breakage specific and actionable.

## Scope

- Each assumption traced to the code that depends on it
- The consequence stated concretely
- SAFETY-002 and SAFETY-005's dependence on wakefulness analysed
- Duty-cycle arithmetic for LoRaWAN worked through with real numbers

## Non-goals

- Solving them.

## Dependencies

- M14-001

## Implementation notes

Working the duty-cycle numbers concretely is what turns 'LoRaWAN is
constrained' into a design input: at typical EU duty-cycle limits and a ~50-byte
payload, compute how many messages per hour are actually possible and what
telemetry interval that implies. That number determines whether the current
staleness model is adjustable or needs replacing.

## Acceptance criteria

- [ ] Each assumption is traced to specific code.
- [ ] Consequences are concrete, not general.
- [ ] SAFETY-002's dependence on a synced, awake device is analysed in detail.
- [ ] Duty-cycle arithmetic is worked through with real numbers.
- [ ] The analysis is specific enough to design against.

## Verification

```bash
cargo run --manifest-path tools/docscheck/Cargo.toml
```

## Tests required

- Review-based.

## Documentation impact

- PRD 140 expanded; a supporting analysis document if warranted.

## Files likely affected

```text
docs/prd/140-field-readiness.md
docs/architecture/field-constraints.md
```
