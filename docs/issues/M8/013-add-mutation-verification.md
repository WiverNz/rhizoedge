# Issue M8-013 — Add mutation verification of the safety tests

**Milestone:** M8 · **PRD:** [PRD 080](../../prd/080-end-to-end-test-environment.md) · **Depends on:** M8-012

## Context

**PRD 080's most important requirement.** A test suite that stays green when
the safety logic is removed is decoration. This issue proves it is not.

## Goal

Demonstrate that each safety mechanism's removal turns the suite red.

## Scope

- Six documented mutations, each applied and reverted:
- remove the leak check -> SCEN-040 must fail
- use `device_time_ms` for staleness -> SCEN-022/070 must fail
- make the outbox drain blocking -> SCEN-060 must fail
- re-publish commands on restart -> SCEN-051 must fail
- use a calendar day for the cap -> SCEN-034 must fail
- let the simulator skip the validator -> SCEN-032 must fail

## Non-goals

- A general mutation testing framework.

## Dependencies

- M8-012

## Implementation notes

Run these once during M8 acceptance and record the results in the milestone
report. Automating them permanently would mean maintaining six broken variants of
the codebase, which is not worth it — but running them once is what converts the
test suite from assumed-effective to demonstrated-effective.

If any mutation does **not** turn the suite red, that is a finding: the
corresponding scenario is not actually testing what it claims.

## Acceptance criteria

- [ ] Each of the six mutations is applied and reverted.
- [ ] Each turns its named scenario red.
- [ ] Results are recorded in the milestone report.
- [ ] Any mutation that does not fail is investigated and the scenario strengthened.

## Verification

```bash
# per mutation: apply, run the suite, confirm the expected failure, revert
git stash && ... run scenario-runner && git stash pop
```

## Tests required

- The six mutation runs, documented.

## Documentation impact

- Milestone report records each mutation and its outcome.

## Files likely affected

```text
docs/testing/mutation-verification.md
```
