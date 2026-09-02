# Issue M8-011 — Implement the cloud failure scenarios

**Milestone:** M8 · **PRD:** [PRD 080](../../prd/080-end-to-end-test-environment.md) · **Depends on:** M8-006

## Context

SCEN-060, -061, -062. SCEN-061 is the differential test — the strongest
statement the project makes about edge-first correctness.

## Goal

Prove cloud independence and idempotent replay end to end.

## Scope

- SCEN-060 cloud down for the entire scenario; everything local works
- SCEN-061 identical seeded runs with the cloud up and down produce identical command sequences
- SCEN-062 cloud recovery drains 500 events exactly once

## Non-goals

- Cloud performance.

## Dependencies

- M8-006

## Implementation notes

SCEN-060 must explicitly assert `/health/ready` returns **200** with the cloud
down. Reporting unready would contradict SAFETY-008 and could trigger a restart
loop in a supervised deployment.

SCEN-062 verifies exactly-once by counting rows in PostgreSQL against the edge's
emitted count, and by re-POSTing a batch and confirming no rows are created.

## Acceptance criteria

- [x] SCEN-060: all local functions work; **`/health/ready` is 200**.
- [x] SCEN-061: command sequences identical modulo ids and timestamps.
- [x] SCEN-061: every lockout occurs in both runs.
- [x] SCEN-062: `pending_cloud_events` returns to 0.
- [x] SCEN-062: PostgreSQL row count matches the emitted count exactly.
- [x] SCEN-062: re-POSTing creates no rows.

## Verification

```bash
... run --rm scenario-runner --scenario scenario_cloud_outage_recovery scenario_cloud_independence
```

## Tests required

- SCEN-060, -061, -062.

## Documentation impact

- None.

## Files likely affected

```text
test/scenarios/cloud.rs
```
