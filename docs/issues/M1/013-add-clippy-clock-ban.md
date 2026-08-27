# Issue M1-013 — Ban direct clock access inside the domain crate

**Milestone:** M1 · **PRD:** [PRD 010](../../prd/010-domain-and-mqtt-protocol.md) · **Depends on:** M1-012, M0-003

## Context

ADR-001 identifies domain purity erosion as a real risk: someone calls
`Utc::now()` for convenience and the safety property tests quietly stop being
deterministic. A lint makes it a build failure instead.

## Goal

Enforce the clock-free constraint mechanically.

## Scope

- `clippy.toml` `disallowed-methods` for `chrono::Utc::now` and `std::time::SystemTime::now`
- Applied to `rhizo-domain`
- A clear message pointing at ADR-013 and the `Clock` trait

## Non-goals

- Banning them elsewhere — binaries legitimately need a real clock.

## Dependencies

- M1-012
- M0-003

## Implementation notes

`clippy.toml` is workspace-wide, so scoping to the domain crate needs either
a crate-level `#![deny]` with an allow elsewhere, or a per-crate lint table.
Prefer the per-crate approach so the ban is visible in the crate that has the
constraint.

The lint message is the documentation a future contributor will read at the
moment they need it — make it say what to use instead.

## Acceptance criteria

- [x] `Utc::now()` in `rhizo-domain` fails `cargo clippy -- -D warnings`.
- [x] The same call in `edge-controller` does not fail.
- [x] The failure message names the `Clock` trait and ADR-013.
- [x] CI enforces it.

## Verification

```bash
cargo clippy -p rhizo-domain --all-targets -- -D warnings
```

## Tests required

- Manual: add `Utc::now()` to the domain, confirm clippy fails, revert.

## Documentation impact

- ADR-001 and ADR-013 follow-up sections already reference this issue.

## Files likely affected

```text
clippy.toml
crates/domain/Cargo.toml
crates/domain/src/lib.rs
```
