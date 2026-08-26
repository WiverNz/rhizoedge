# Issue M14-001 — Verify every reserved expansion point exists in code

**Milestone:** M14 · **PRD:** [PRD 140](../../prd/140-field-readiness.md) · **Depends on:** M13-016

## Context

PRD 140 claims six expansion points are already reserved. A claim verified
against the ADR that proposed it proves nothing — it must be checked against the
actual schema and code.

## Goal

Confirm the reservations are real.

## Scope

- `measurements.measurement_point` exists and is populated
- `plants.device_id` is genuinely many-to-one
- `edge_id` partitions every cloud table
- The MQTT contract carries no transport concern
- Sensor and pump traits are genuinely swappable
- The v2 namespace path is viable

## Non-goals

- Implementing anything.

## Dependencies

- M13-016

## Implementation notes

Check the code, not the documentation. Where a reservation turns out to be
absent or ineffective, say so plainly — an unverified reservation is worse than
a known gap, because it will be relied on.

The trait swappability claim is strongest evidence: M10 and M11 swapped real
hardware in with no edge changes, which is the property being verified.

## Acceptance criteria

- [ ] Each of the six reservations is verified **against code**.
- [ ] Any absent or ineffective reservation is documented as a gap.
- [ ] PRD 140's table is corrected where reality differs.
- [ ] The evidence for each is recorded (file and line, or a test).

## Verification

```bash
sqlite3 data/edge.sqlite '.schema measurements' | grep measurement_point
grep -rn 'reservoir_id\|edge_id' migrations/
```

## Tests required

- Verification is by inspection; the corrected PRD is the artefact.

## Documentation impact

- PRD 140 corrected.

## Files likely affected

```text
docs/prd/140-field-readiness.md
```
