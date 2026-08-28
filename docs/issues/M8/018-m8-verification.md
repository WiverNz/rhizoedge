# Issue M8-018 — M8 verification and exit criteria

**Milestone:** M8 · **PRD:** [PRD 080](../../prd/080-end-to-end-test-environment.md) · **Depends on:** M8-001, M8-002, M8-003, M8-004, M8-005, M8-006, M8-007, M8-008, M8-009, M8-010, M8-011, M8-012, M8-013, M8-014, M8-015, M8-016, M8-017

## Context

Final gate for M8, and the point at which the project has a complete,
reproducible, hardware-free system with its safety claims demonstrated rather
than asserted.

## Goal

Verify every PRD 080 acceptance criterion.

## Scope

- Full gate plus the complete scenario suite
- Run the seven mutations and record results (six from M8-013, one from M8-017)
- Update safety-invariants.md, ROADMAP.md, and README status
- Record the milestone report

## Non-goals

- New behaviour.

## Dependencies

- M8-001
- M8-002
- M8-003
- M8-004
- M8-005
- M8-006
- M8-007
- M8-008
- M8-009
- M8-010
- M8-011
- M8-012
- M8-013
- M8-014
- M8-015
- M8-016
- M8-017

## Implementation notes

This is the milestone where the first major demo becomes real. Verify it
runs from a fresh clone with no local state — the same discipline as M0-013,
because a demo that only works on the author's machine is not a demo.

The mutation results are the headline evidence in the report: they demonstrate
that the safety suite detects the removal of each safety mechanism.

## Acceptance criteria

- [ ] `docker compose up --build` starts all five services from a fresh clone.
- [ ] The full suite runs with one command and exits 0.
- [ ] Total runtime is under 10 minutes.
- [ ] Every `e2e` scenario in failure-scenarios.md is implemented and green.
- [ ] `scenario_first_demo` reproduces all eighteen steps.
- [ ] **Each of the seven mutations turns the suite red** — the six from M8-013
      and M8-017's immediate-publish-to-a-sleeping-device mutation.
- [ ] CI runs the suite.
- [ ] A failing scenario prints database state and MQTT traffic.
- [ ] safety-invariants.md, ROADMAP.md, and README updated; report recorded.

## Verification

```bash
git clone <repo> /tmp/rhizo-m8 && cd /tmp/rhizo-m8
docker compose -f deploy/docker-compose.yml up --build -d
docker compose -f deploy/docker-compose.yml -f deploy/docker-compose.test.yml up --abort-on-container-exit --exit-code-from scenario-runner
cargo run --manifest-path tools/docscheck/Cargo.toml
```

## Tests required

- Full suite plus mutation verification.

## Documentation impact

- safety-invariants.md.
- ROADMAP.md.
- README.md status.
- Milestone report.

## Files likely affected

```text
ROADMAP.md
README.md
docs/architecture/safety-invariants.md
```
