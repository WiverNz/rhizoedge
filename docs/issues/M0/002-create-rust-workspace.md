# Issue M0-002 — Create the Rust workspace and shared dependency table

**Milestone:** M0 · **PRD:** [PRD 000](../../prd/000-platform-foundation.md) · **Depends on:** M0-001

## Context

ADR-001 specifies three workspaces, with `firmware/` and `ui/` excluded from
the root so `cargo test --workspace` means exactly 'all host code'. Shared
dependency versions live in `[workspace.dependencies]` so two crates cannot
disagree on a `chrono` version and stop unifying types.

## Goal

Create the root workspace with its member crates and a shared dependency table.

## Scope

- Root `Cargo.toml` with `resolver = "2"`, `members`, and `exclude = ["firmware", "ui"]`
- `[workspace.dependencies]` for tokio, serde, serde_json, chrono, uuid, thiserror, anyhow, tracing, tracing-subscriber, sqlx, axum, rumqttc, reqwest, proptest
- `[workspace.package]` with shared version, edition, license, and rust-version
- Empty-but-compiling crate skeletons for all nine members

## Non-goals

- Real crate content — each crate is filled by its own milestone.
- The firmware or UI workspaces (M9, M12).

## Dependencies

- M0-001

## Implementation notes

Members: `crates/mqtt-contract`, `crates/domain`, `crates/storage`,
`crates/telemetry`, `crates/cloud-client`, `crates/testkit`,
`crates/edge-controller`, `crates/device-simulator`, `crates/cloud-api`.

Package names are prefixed (`rhizo-domain`) while directory names are not, per
ADR-001. Every member writes `tokio = { workspace = true }` rather than a
version literal.

`exclude` is load-bearing: without it, `cargo build` at the root would attempt
the ESP-IDF build for every developer, including those who never touch hardware.

## Acceptance criteria

- [ ] `cargo build --workspace` succeeds.
- [ ] `cargo test --workspace` succeeds (no tests yet, but the command works).
- [ ] Every dependency version appears exactly once, in `[workspace.dependencies]`.
- [ ] `cargo tree -d` reports no duplicate versions of a shared dependency.
- [ ] `firmware/` and `ui/` are excluded and their absence does not break the build.

## Verification

```bash
cargo build --workspace
cargo test --workspace
cargo tree -d
```

## Tests required

- A trivial `#[test]` in one crate proving the harness runs.

## Documentation impact

- None; ADR-001 already specifies this layout.

## Files likely affected

```text
Cargo.toml
crates/*/Cargo.toml
crates/*/src/lib.rs
crates/{edge-controller,device-simulator,cloud-api}/src/main.rs
```
