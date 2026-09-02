# Issue M8-014 — Add the end-to-end CI job

**Milestone:** M8 · **PRD:** [PRD 080](../../prd/080-end-to-end-test-environment.md) · **Depends on:** M8-012

## Context

A suite that only runs locally eventually stops running. PRD 080 requires the
scenarios on every change to `crates/**` or `deploy/**`.

## Goal

Run the scenario suite in CI.

## Scope

- A CI job running the full suite
- Triggered on changes to `crates/**` or `deploy/**`
- Docker layer caching
- Artefacts uploaded on failure: logs and database dumps
- A total time budget under 15 minutes including the build

## Non-goals

- Running the suite on documentation-only changes.

## Dependencies

- M8-012

## Implementation notes

**Never retry a failing scenario to green.** PRD 080 is explicit: a
genuinely flaky scenario is quarantined and fixed. An automatically retried flaky
safety test is a safety test that does not work.

Uploading the failure dump is what makes a CI failure actionable without local
reproduction.

## Acceptance criteria

- [x] The job runs on the specified paths.
- [x] The full suite passes in CI.
- [x] Failures upload logs and database dumps.
- [x] Total time is under 15 minutes.
- [x] **No automatic retry of failed scenarios.**

## Verification

```bash
# observe a CI run on a branch touching crates/
```

## Tests required

- The CI job itself.

## Documentation impact

- docs/testing/strategy.md CI table verified.

## Files likely affected

```text
.github/workflows/ci.yml
```
