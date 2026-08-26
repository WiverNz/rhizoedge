# Issue M3-018 — M3 verification and exit criteria

**Milestone:** M3 · **PRD:** [PRD 030](../../prd/030-edge-ingestion-and-storage.md) · **Depends on:** M3-001, M3-002, M3-003, M3-004, M3-005, M3-006, M3-007, M3-008, M3-009, M3-010, M3-011, M3-012, M3-013, M3-014, M3-015, M3-016, M3-017

## Context

Final gate for M3. The dedup transaction established here is the mechanism
SAFETY-001 and SAFETY-010 rely on for the rest of the project.

## Goal

Verify every PRD 030 acceptance criterion.

## Scope

- Full gate plus integration tests with a real broker
- Verify the restart and duplicate behaviours specifically
- Update ROADMAP.md and record the report

## Non-goals

- New behaviour.

## Dependencies

- M3-001
- M3-002
- M3-003
- M3-004
- M3-005
- M3-006
- M3-007
- M3-008
- M3-009
- M3-010
- M3-011
- M3-012
- M3-013
- M3-014
- M3-015
- M3-016
- M3-017

## Implementation notes

Two verifications carry the weight: restarting the edge mid-stream must
preserve history exactly, and restarting the **broker** must result in
re-subscription — not merely reconnection. The second is the one most likely to
be quietly broken, because the connection metric looks healthy either way.

## Acceptance criteria

- [ ] All gate commands pass.
- [ ] Simulator telemetry appears in `measurements` with edge-stamped `received_at`.
- [ ] A duplicate `message_id` produces one row.
- [ ] Edge restart preserves history and restores the registry.
- [ ] Broker restart reconnects **and re-subscribes**; telemetry resumes.
- [ ] A partially invalid message stores good fields and nulls the bad one.
- [ ] Invalid JSON is quarantined and the next message processes.
- [ ] SIGTERM exits 0; a task panic exits non-zero.
- [ ] ROADMAP.md updated and the report recorded.

## Verification

```bash
cargo test --workspace --all-features
cargo test --test integration
docker compose restart mosquitto  # confirm resubscription
cargo run --manifest-path tools/docscheck/Cargo.toml
```

## Tests required

- Full suite including integration.

## Documentation impact

- ROADMAP.md.
- Milestone report.

## Files likely affected

```text
ROADMAP.md
```
