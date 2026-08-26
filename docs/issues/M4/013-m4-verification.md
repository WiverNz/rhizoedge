# Issue M4-013 — M4 verification and exit criteria

**Milestone:** M4 · **PRD:** [PRD 040](../../prd/040-device-registry-and-health.md) · **Depends on:** M4-001, M4-002, M4-003, M4-004, M4-005, M4-006, M4-007, M4-008, M4-009, M4-010, M4-011, M4-012

## Context

Final gate for M4. The staleness computation established here is the input
SAFETY-005 depends on in M6.

## Goal

Verify every PRD 040 acceptance criterion.

## Scope

- Full gate plus integration tests
- Verify the timer-driven stale detection and the no-plant registration
- Update ROADMAP.md and record the report

## Non-goals

- New behaviour.

## Dependencies

- M4-001
- M4-002
- M4-003
- M4-004
- M4-005
- M4-006
- M4-007
- M4-008
- M4-009
- M4-010
- M4-011
- M4-012

## Implementation notes

Two verifications carry the weight: staleness must be detected by the timer
with no inbound message, and auto-registration must leave `plants` empty. Both
are properties that a later convenience change could quietly break.

## Acceptance criteria

- [ ] All gate commands pass.
- [ ] Killing the simulator moves the device offline and records an event.
- [ ] Restarting yields a new `boot_id` with the sequence restart **not** flagged.
- [ ] Stopping telemetry while connected produces a stale indication from the timer.
- [ ] An unknown device registers with **no plant attached**.
- [ ] `/health/ready` is 200 with the cloud stopped and 503 with the broker stopped.
- [ ] PATCH changes the name; nothing changes `device_id`.
- [ ] ROADMAP.md updated and the report recorded.

## Verification

```bash
cargo test --workspace --all-features
cargo test --test integration
curl -s localhost:8080/api/v1/devices | jq
cargo run --manifest-path tools/docscheck/Cargo.toml
```

## Tests required

- Full suite.

## Documentation impact

- ROADMAP.md.
- Milestone report.

## Files likely affected

```text
ROADMAP.md
```
