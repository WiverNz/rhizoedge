# Issue M11-014 — M11 verification and exit criteria

**Milestone:** M11 · **PRD:** [PRD 110](../../prd/110-real-pump-and-safety-hardware.md) · **Depends on:** M11-001, M11-002, M11-003, M11-004, M11-005, M11-006, M11-007, M11-008, M11-009, M11-010, M11-011, M11-012, M11-013

## Context

Final gate for M11 and the first hardware demo. From here the system can
water a real plant, so the verification is physical and the record matters.

## Goal

Verify every PRD 110 acceptance criterion.

## Scope

- HIL-1 through HIL-6 complete and recorded
- Update safety-invariants.md statuses to ENFORCED for the hardware invariants
- Update ROADMAP.md; record the report
- HIL-7 (supervised plant) begun but not required for milestone completion

## Non-goals

- HIL-7 completion — it takes a month by design.

## Dependencies

- M11-001
- M11-002
- M11-003
- M11-004
- M11-005
- M11-006
- M11-007
- M11-008
- M11-009
- M11-010
- M11-011
- M11-012
- M11-013

## Implementation notes

HIL-7 deliberately spans a month: one week of recommendations only, then
supervised automatic operation, then a review before raising any limit. It
starts here but does not gate the milestone, because gating a milestone on a
month of plant observation would be false precision.

The hil-runs records are the artefact that makes the safety claims auditable.

## Acceptance criteria

- [ ] HIL-1 through HIL-6 all pass and are recorded.
- [ ] A 40 ml request delivers within ±10%, measured.
- [ ] An oversized command delivers no more than the hard limit, measured.
- [ ] A leak stops an in-progress dose within 1 second.
- [ ] All lockouts behave as specified.
- [ ] The full cycle completes correctly into soil.
- [ ] safety-invariants.md marks SAFETY-003, -004, -007, -011 ENFORCED.
- [ ] ROADMAP.md updated; report recorded.
- [ ] HIL-7 has begun with a robust, inexpensive plant and halved limits.

## Verification

```bash
# the complete HIL checklist series, recorded
```

## Tests required

- HIL-1 through HIL-6.

## Documentation impact

- safety-invariants.md.
- ROADMAP.md.
- hil-runs records.
- Milestone report.

## Files likely affected

```text
ROADMAP.md
docs/architecture/safety-invariants.md
docs/testing/hil-runs/
```
