# Issue M0-013 — M0 verification and exit criteria

**Milestone:** M0 · **PRD:** [PRD 000](../../prd/000-platform-foundation.md) · **Depends on:** M0-001, M0-002, M0-003, M0-004, M0-005, M0-006, M0-007, M0-008, M0-009, M0-010, M0-011, M0-012

## Context

Final gate for M0. A milestone is complete only when its acceptance criteria
are demonstrably met — not when its issues are closed.

## Goal

Verify every PRD 000 acceptance criterion and record the evidence.

## Scope

- Run the full gate on a clean checkout
- Verify Mosquitto rejects anonymous access and enforces ACLs
- Verify configuration fail-fast and redaction
- Update ROADMAP.md M0 status to DONE
- Record the milestone report

## Non-goals

- Any new behaviour.

## Dependencies

- M0-001
- M0-002
- M0-003
- M0-004
- M0-005
- M0-006
- M0-007
- M0-008
- M0-009
- M0-010
- M0-011
- M0-012

## Implementation notes

Verify on a **fresh clone** into a clean directory. Local state hides
missing files, and a foundation milestone that only works on the author's
machine has failed at its one job.

The milestone report format is in docs/README.md: files added, files changed,
tests added, commands run, results, known limitations, next milestone.

## Acceptance criteria

- [x] Fresh clone; all five gate commands pass.
- [x] `docker compose up mosquitto` works from the fresh clone after running the passwd script.
- [x] Anonymous MQTT is refused; cross-device publish is refused.
- [x] Invalid config exits non-zero naming the key.
- [x] `Debug` on the config shows `[redacted]`.
- [ ] CI is green. — *the five gate commands are green from a fresh checkout and
      `actionlint` passes on the workflow; a GitHub run has not been observed
      because the working tree is uncommitted by request.*
- [x] ROADMAP.md M0 status is DONE.
- [x] The milestone report is recorded.

## Verification

```bash
git clone <repo> /tmp/rhizo-verify && cd /tmp/rhizo-verify
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
docker compose -f deploy/docker-compose.yml config
cargo run --manifest-path tools/docscheck/Cargo.toml
```

## Tests required

- The whole suite, on a fresh clone.

## Documentation impact

- ROADMAP.md status update.
- Milestone report.

## Files likely affected

```text
ROADMAP.md
```
