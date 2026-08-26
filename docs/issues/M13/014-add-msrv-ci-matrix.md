# Issue M13-014 — Add the MSRV and current-stable CI matrix

**Milestone:** M13 · **PRD:** [PRD 130](../../prd/130-multi-plant-home.md) · **Depends on:** M13-013

## Context

[ADR-001](../../adr/001-rust-workspace-and-crate-boundaries.md) sets MSRV 1.98.0
while allowing the pin to move forward. Without a check, an accidental MSRV bump
is discovered by a user on an older toolchain.

## Goal

Verify both the MSRV and current stable on every change.

## Scope

- CI job building and testing on **1.98.0** explicitly
- CI job building and testing on current **stable**
- A failure on the MSRV job names the MSRV policy and points at ADR-001
- Document that raising the MSRV is a deliberate decision requiring an ADR update

## Non-goals

- Raising the MSRV.
- Testing every intermediate version.

## Dependencies

- M13-013

## Implementation notes

The MSRV job must not use `rust-toolchain.toml`, which pins the development
toolchain; it overrides with an explicit `1.98.0`. Otherwise both jobs test the
same thing and the matrix proves nothing.

Keep the failure message actionable. "error[E0658]: … stabilised in 1.99" is
accurate but leaves the reader guessing about policy; the job should say that this
raises the MSRV and that doing so requires updating ADR-001, README, and ROADMAP.

## Acceptance criteria

- [ ] Both jobs run on every change.
- [ ] The MSRV job uses 1.98.0 explicitly, overriding the toolchain file.
- [ ] Using a post-1.98.0 feature fails the MSRV job and passes the stable job.
- [ ] The failure message names the policy and the documents to update.
- [ ] The stable job catches new lints without blocking the MSRV job.

## Verification

```bash
cargo +1.98.0 test --workspace --all-features
cargo +stable test --workspace --all-features
```

## Tests required

- Manual: use a post-1.98.0 feature, confirm the MSRV job fails, revert.

## Documentation impact

- ADR-001 MSRV section verified.
- README Rust version note.

## Files likely affected

```text
.github/workflows/ci.yml
```
