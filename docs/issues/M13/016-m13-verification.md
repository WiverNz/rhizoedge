# Issue M13-016 — M13 verification and exit criteria

**Milestone:** M13 · **PRD:** [PRD 130](../../prd/130-multi-plant-home.md) · **Depends on:** M13-001, M13-002, M13-003, M13-004, M13-005, M13-006, M13-007, M13-008, M13-009, M13-010, M13-011, M13-012, M13-013, M13-014, M13-015

## Context

Final gate for M13, and the point at which the system is a supportable
household deployment rather than a single-plant demonstration.

## Goal

Verify every PRD 130 acceptance criterion.

## Scope

- Full gate plus multi-device scenarios
- Install on a real Pi, reboot, verify recovery
- Verify backup and restore fidelity
- Update ROADMAP.md; record the report

## Non-goals

- Field features (M14).

## Dependencies

- M13-001
- M13-002
- M13-003
- M13-004
- M13-005
- M13-006
- M13-007
- M13-008
- M13-009
- M13-010
- M13-011
- M13-012
- M13-013
- M13-014
- M13-015

## Implementation notes

Two verifications carry the weight: cross-plant isolation (a new class of bug
at this scale) and a real Pi reboot recovery (the difference between a
deployment and a demonstration).

Note plainly in the report that the Edge API still has no authentication — at 20
plants across a household, that limitation becomes more consequential and should
be the first thing addressed for any non-trusted network.

## Acceptance criteria

- [ ] 5 simulated devices and 10 plants operate independently.
- [ ] **SCEN-080 passes.**
- [ ] `rhizo-provision new` produces working credentials in one command.
- [ ] Provisioning refuses to reuse a device id without `--force`.
- [ ] A leak produces exactly one notification.
- [ ] A dead notification channel does not delay the control loop.
- [ ] The system survives a Pi reboot and resumes.
- [ ] Backup and restore reproduce identical row counts and watering history.
- [ ] Two devices on one reservoir: the lowest reading governs.
- [ ] 20 plants evaluate within one tick period.
- [ ] The UI is legible at 20 plants.
- [ ] ROADMAP.md updated; report records the authentication limitation.

## Verification

```bash
docker compose up --scale device-simulator=5
cargo test --test integration multi_device
# on a Pi: reboot and verify recovery
```

## Tests required

- Full suite plus multi-device scenarios and the Pi checklist.

## Documentation impact

- ROADMAP.md.
- Milestone report.

## Files likely affected

```text
ROADMAP.md
```
