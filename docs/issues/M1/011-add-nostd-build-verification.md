# Issue M1-011 — Add no_std build verification to CI

**Milestone:** M1 · **PRD:** [PRD 010](../../prd/010-domain-and-mqtt-protocol.md) · **Depends on:** M1-001, M0-012

## Context

ADR-001 identifies a `no_std` regression in the contract crate as a risk that
the default CI job cannot see: a `std`-only transitive dependency breaks the
firmware build while every host test stays green.

## Goal

Catch `no_std` regressions on every change, without an ESP toolchain.

## Scope

- A CI step building the contract crate for `thumbv7em-none-eabi` with default features off
- `rustup target add` in the workflow
- The same command documented in local-development.md

## Non-goals

- The ESP32 firmware build (M9-002) — that needs the heavy toolchain.

## Dependencies

- M1-001
- M0-012

## Implementation notes

`thumbv7em-none-eabi` is chosen deliberately: it is a bare-metal target that
`rustup` installs in seconds and needs no C toolchain, so the check costs almost
nothing while catching the regression precisely.

It runs on **every** change, not only firmware changes, because the regression
is introduced by contract-crate edits.

## Acceptance criteria

- [x] CI builds the contract crate for the bare-metal target.
- [x] Adding a `std`-only dependency to the contract crate fails CI.
- [x] The check adds under a minute to the run.
- [x] The command is documented for local use.

## Verification

```bash
rustup target add thumbv7em-none-eabi
cargo build -p rhizo-mqtt-contract --no-default-features --target thumbv7em-none-eabi
```

## Tests required

- Manual: add `std::collections::HashMap` to the crate, confirm the build fails, revert.

## Documentation impact

- docs/testing/local-development.md section 12 already documents it; verify accurate.

## Files likely affected

```text
.github/workflows/ci.yml
```
