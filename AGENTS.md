# Rhizo Edge — Codex Instructions

Rhizo Edge is an offline-first Rust platform for plant monitoring and fail-safe irrigation: devices communicate over MQTT with an edge controller backed by SQLite; cloud synchronization is optional and outside the local control path.

## Current project state

Planning and M0–M3 are complete. M4 is READY; the next executable issue is **M4-001 — Apply ingested device status to the registry**. Verify this from `ROADMAP.md`, the dependency graph, Git history, and the working tree at the start of every implementation session. Status summaries in navigation or agent files can become stale.

Do not begin a milestone merely because its predecessor is complete. Implement only the milestone and issues the user requested, then stop after reporting completion.

## Source-of-truth precedence

Use each source for its intended question:

1. The current issue defines implementation scope, dependencies, acceptance criteria, and literal verification commands.
2. Normative protocol documents, especially `docs/protocol/mqtt-v1.md` and the versioning policy, define wire behavior.
3. The milestone PRD defines required product behavior.
4. Accepted ADRs define architectural decisions and rationale.
5. `docs/architecture/safety-invariants.md` defines non-negotiable safety properties and their evidence.
6. Other architecture and testing documents define component boundaries and validation strategy.
7. `ROADMAP.md` defines milestone state, exit criteria, and the current starting point; the dependency graph defines executable order.

Historical prompt and project-plan documents are provenance, not current specifications. If issue prose conflicts with a later accepted ADR or normative protocol, verify the contradiction, update the stale planning text, and implement one consistent current design. Never implement both versions.

## Required reading before implementation

Before coding, read `ROADMAP.md`, `docs/architecture/dependency-graph.md`, the complete current issue and its dependencies, and the milestone PRD. Read the relevant crate boundary in `docs/architecture/component-model.md`.

Before changing protocol, safety, actuation, persistence semantics, clocks, or offline behavior, also read the relevant protocol specification, ADRs, safety invariants, failure/time/connectivity model, and existing tests. Use actual shared contract APIs and fixtures; do not reconstruct wire formats from memory.

## Architecture constraints

- Production/runtime implementation is Rust. Do not add Go, Node.js, TypeScript, npm, or `package.json` without an explicit architecture change. The UI uses Tauri/Leptos with Cargo/Trunk.
- The host MSRV is Rust 1.98.0. Respect `rust-toolchain.toml`; never silently raise the MSRV or downgrade host Rust for embedded tooling. Firmware may use its separately documented ESP toolchain.
- `rhizo-domain` is pure and obtains time through `Clock`; no direct wall-clock calls.
- `rhizo-mqtt-contract` and `rhizo-policy` remain `no_std` compatible and are the shared firmware-facing crates.
- While connected, the Edge owns high-level decisions and authoritative state. The device is always the final hardware safety boundary.
- Offline autonomy uses exactly one pure shared `rhizo_policy::evaluate_offline` implementation. Do not create simulator- or firmware-specific copies.
- The UI talks to Edge REST APIs, never directly to MQTT or hardware.
- Cloud is an optional append-only history sink and must never enter the local control path or originate commands.
- Persist-before-publish and documented transactional deduplication boundaries are safety properties, not implementation details.

## Safety rules

- Uncertainty fails closed. Missing, stale, invalid, unknown, or contradictory safety input never becomes permission to actuate.
- Never weaken safety behavior to make a test pass. Investigate disagreements between tests, implementation, and normative documentation; fail-closed behavior is not permission to silently diverge from the specification.
- Do not add force, debug, override, raw-pump, or bypass actuation paths.
- Firmware hard limits cannot be raised through API, configuration, MQTT, UI, or cloud input.
- Keep the single shared command validator and single shared offline evaluator; consumers have exactly the documented call path.
- Edge safety freshness uses Edge `received_at`, never advisory device timestamps.
- Preserve negative-control and mutation-testing discipline for safety-critical boundaries. Run applicable temporary mutations, prove the intended tests fail, and fully revert them.

## Issue workflow

Work one issue at a time unless the user explicitly requests a larger issue set:

1. Read the whole issue and its dependencies.
2. Confirm dependencies are complete in the repository.
3. Implement only the issue scope; do not leak later milestone behavior.
4. Add the required unit, integration, property, or real-broker tests.
5. Run the issue's literal `Verification` commands.
6. Fix failures before continuing.
7. Tick only acceptance criteria actually demonstrated.
8. Keep factual documentation synchronized with behavior.

Compilation alone never completes an issue or milestone. A milestone becomes DONE only after all current issues and ROADMAP exit criteria are demonstrably green, its report is recorded, and milestone/status pointers are synchronized.

## Rust and toolchain policy

Use workspace dependencies and lints. Preserve explicit error classification, `-D warnings`, the clock restrictions in `clippy.toml`, and `no_std` compatibility. Do not introduce an ORM over SQLx or weaken lint/test configuration to obtain green output. Keep files LF and do not bulk-regenerate issue files.

## Testing and verification

Run each issue's commands plus the current project gate before milestone completion:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo build -p rhizo-mqtt-contract --no-default-features --target thumbv7em-none-eabi
cargo build -p rhizo-policy --no-default-features --target thumbv7em-none-eabi
docker compose -f deploy/docker-compose.yml config
cargo run -p rhizo-docscheck
```

When SQLx query macros or migrations change, run the repository's documented `cargo sqlx prepare --workspace --check` flow; never hand-edit `.sqlx` metadata.

Broker-dependent verification must use real Mosquitto. For required integration/milestone verification, set `RHIZO_REQUIRE_BROKER=1` and run the applicable broker suites so missing infrastructure fails rather than silently skips. Do not replace broker/SQLite integration tests with mocks or weaken/ignore tests merely to make a milestone green. After safety-domain changes, run `cargo test safety_` and any named invariant tests.

## Documentation discipline

Reference architecture, ADRs, PRDs, protocols, issues, and tests rather than duplicating them here. Update factual docs in the same change as behavior. Do not rewrite accepted ADRs for implementation detail unless a real contradiction or open decision is resolved. Run docscheck after changing identifiers, links, issue state, or milestone state. Never claim a command ran when it did not.

## Git rules

Read-only Git inspection and requested working-tree edits are allowed. Preserve unrelated user changes. Unless the user explicitly requests it, do not run `git commit`, `git push`, `git tag`, `git rebase`, `git reset --hard`, `git cherry-pick`, force-push, create a PR/release, or rewrite history.

At the end of substantial work, report Git status, files changed, actual tests/commands run, deviations, and a suggested commit message when useful. Do not execute the commit.

## Milestone boundaries

Use `ROADMAP.md` and dependency declarations rather than numeric intuition to choose executable work. Keep READY/PLANNED milestones free of production implementation until explicitly requested. After completing the requested issue or milestone, report the exact next executable issue from the dependency graph and **STOP**; do not automatically begin it.
