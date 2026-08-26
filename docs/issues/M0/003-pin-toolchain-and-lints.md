# Issue M0-003 — Pin the Rust toolchain and configure workspace lints

**Milestone:** M0 · **PRD:** [PRD 000](../../prd/000-platform-foundation.md) · **Depends on:** M0-002

## Context

CI runs clippy with `-D warnings`. Without a pinned toolchain, a new lint in
a later Rust release turns a green main branch red with no code change. ADR-014
also requires `unwrap`/`expect` to be denied in library crates.

## Goal

Pin an exact toolchain and establish the lint policy that every later crate inherits.

## Scope

- `rust-toolchain.toml` pinning **`channel = "1.98.0"`** with `rustfmt` and `clippy`
- `[workspace.lints]` denying `clippy::unwrap_used` and `clippy::expect_used`
- `clippy.toml` created with an empty `disallowed-methods` (populated in M1-013)
- `rustfmt.toml` with explicit settings
- `lints.workspace = true` in every member crate

## Non-goals

- The clock-method ban (M1-013 — the domain crate does not exist yet).

## Dependencies

- M0-002

## Implementation notes

`unwrap_used` and `expect_used` are denied for libraries and allowed in
tests via `#![cfg_attr(test, allow(clippy::unwrap_used))]` or a per-module
allow. Where an invariant genuinely cannot be violated, `expect()` is permitted
with a message stating *why* — that message is the documentation.

The pinned version is **1.98.0** (ROADMAP.md §6). Bumping it is a deliberate,
separate change, never a side effect of another issue. The firmware workspace
pins its own toolchain and is unaffected by this file
([ADR-007](../../adr/007-esp32-rust-framework-and-toolchain.md)).

## Acceptance criteria

- [x] `rust-toolchain.toml` names exactly `1.98.0`, not `stable`.
- [x] `cargo clippy --workspace --all-targets --all-features -- -D warnings` exits 0.
- [x] An `unwrap()` added to a library crate fails the clippy run.
- [x] The same `unwrap()` in a `#[cfg(test)]` module does not fail.
- [x] `cargo fmt --all --check` exits 0.

## Verification

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

## Tests required

- Manual: add an `unwrap()` to a lib crate, confirm clippy fails, remove it.

## Documentation impact

- None.

## Files likely affected

```text
rust-toolchain.toml
clippy.toml
rustfmt.toml
Cargo.toml
crates/*/Cargo.toml
```
