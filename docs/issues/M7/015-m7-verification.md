# Issue M7-015 — M7 verification and exit criteria

**Milestone:** M7 · **PRD:** [PRD 070](../../prd/070-cloud-sync-and-storage.md) · **Depends on:** M7-001, M7-002, M7-003, M7-004, M7-005, M7-006, M7-007, M7-008, M7-009, M7-010, M7-011, M7-012, M7-013, M7-014

## Context

Final gate for M7, enforcing the two invariants that define the project's
thesis: the cloud can vanish without affecting monitoring or safety.

## Goal

Verify every PRD 070 acceptance criterion.

## Scope

- Full gate plus cloud integration tests
- Verify SAFETY-008 and SAFETY-009 specifically
- Update safety-invariants.md and ROADMAP.md; record the report

## Non-goals

- New behaviour.

## Dependencies

- M7-001
- M7-002
- M7-003
- M7-004
- M7-005
- M7-006
- M7-007
- M7-008
- M7-009
- M7-010
- M7-011
- M7-012
- M7-013
- M7-014

## Implementation notes

The defining verification: stop the cloud container for an entire watering
scenario and confirm that ingestion, storage, recommendations, automatic
watering, the API, and metrics all work — and that `/health/ready` stays 200.
An edge that reports itself unready because the cloud is down would contradict
SAFETY-008 and could trigger a pointless restart loop.

## Acceptance criteria

- [x] All gate commands pass.
- [x] With the cloud stopped, every local function works.
- [x] **`/health/ready` returns 200 with the cloud stopped.**
- [x] `pending_cloud_events` grows during the outage and returns to 0 after recovery.
- [x] Every event reaches PostgreSQL exactly once.
- [x] Re-POSTing a batch returns all `duplicate` and creates no rows.
- [x] `safety_009_decisions_identical_with_cloud_down` passes.
- [x] Filling the outbox past the cap preserves every high-tier event.
- [x] `reproject` reproduces identical tables.
- [x] `rhizo-domain` has no cloud dependency.
- [x] safety-invariants.md and ROADMAP.md updated; report recorded.

## Verification

```bash
cargo test --workspace --all-features
cargo test safety_
docker compose stop cloud-api
curl -i localhost:8080/health/ready   # 200
docker compose start cloud-api
cargo run --manifest-path tools/docscheck/Cargo.toml
```

## Tests required

- Full suite including cloud integration.

## Documentation impact

- safety-invariants.md.
- ROADMAP.md.
- Milestone report.

## Files likely affected

```text
docs/architecture/safety-invariants.md
ROADMAP.md
```
