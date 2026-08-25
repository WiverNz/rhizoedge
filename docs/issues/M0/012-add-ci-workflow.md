# Issue M0-012 — Add the CI workflow with the full gate

**Milestone:** M0 · **PRD:** [PRD 000](../../prd/000-platform-foundation.md) · **Depends on:** M0-003, M0-009, M0-011

## Context

Every gate the project relies on must run on every change. A gate that runs
only locally is a gate that eventually stops running.

## Goal

Run fmt, clippy, tests, Compose validation, and doc validation on every push and pull request.

## Scope

- A workflow running the five commands from PRD 000's acceptance criteria
- Cargo registry and target caching
- The pinned toolchain from `rust-toolchain.toml`
- Jobs structured so later milestones append (integration, e2e, firmware, UI) without restructuring

## Non-goals

- Integration tests needing a broker (M3).
- The e2e job (M8-014).
- The firmware job (M9-002).
- Release automation (M13).

## Dependencies

- M0-003
- M0-009
- M0-011

## Implementation notes

Use the toolchain file rather than naming a version in the workflow, so the
pin lives in exactly one place.

Cache keyed on `Cargo.lock` plus the toolchain version. A stale cache after a
toolchain bump produces confusing failures.

Fail fast on `fmt` — it is the cheapest check and a formatting failure makes
every other diff noisy.

## Acceptance criteria

- [ ] The workflow runs on push and pull request.
- [ ] All five gate commands run and must pass.
- [ ] A formatting violation fails CI.
- [ ] A clippy warning fails CI.
- [ ] A failing test fails CI.
- [ ] Caching measurably reduces the second run's time.

## Verification

```bash
# push a branch and observe the run; then locally:
cargo fmt --all --check && cargo clippy --workspace --all-targets --all-features -- -D warnings && cargo test --workspace --all-features && docker compose -f deploy/docker-compose.yml config >/dev/null && cargo run --manifest-path tools/docscheck/Cargo.toml
```

## Tests required

- Deliberately break formatting on a branch, confirm CI fails, revert.

## Documentation impact

- docs/testing/strategy.md CI gates table (already written) verified accurate.

## Files likely affected

```text
.github/workflows/ci.yml
```
