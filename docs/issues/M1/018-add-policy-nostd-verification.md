# Issue M1-018 — Add no_std build verification for rhizo-policy

**Milestone:** M1 · **PRD:** [PRD 010](../../prd/010-domain-and-mqtt-protocol.md) · **Depends on:** M1-016, M1-011

## Context

M1-011 guards the contract crate's `no_std` compatibility. `rhizo-policy` is the
second crate the firmware imports and needs the same guard, for the same reason:
a `std`-only dependency breaks the ESP32 build while every host test stays
green.

## Goal

Catch `no_std` regressions in `rhizo-policy` on every change.

## Scope

- Extend the existing CI step to build `rhizo-policy` for `thumbv7em-none-eabi` with default features off
- Document the command alongside the contract crate's in local-development.md

## Non-goals

- The ESP32 firmware build (M9-002).

## Dependencies

- M1-016
- M1-011

## Implementation notes

Reuse the same bare-metal target as M1-011: it installs in seconds, needs no C
toolchain, and catches the regression precisely. Running both crates in one step
keeps the CI addition to a few seconds.

This runs on **every** change, not only firmware changes, because the regression
is introduced by edits to these crates rather than to the firmware.

## Acceptance criteria

- [ ] CI builds `rhizo-policy` for the bare-metal target.
- [ ] Adding a `std`-only dependency to `rhizo-policy` fails CI.
- [ ] The command is documented for local use.
- [ ] The check adds under a minute to the run.

## Verification

```bash
cargo build -p rhizo-policy --no-default-features --target thumbv7em-none-eabi
```

## Tests required

- Manual: add a `std` dependency, confirm failure, revert.

## Documentation impact

- docs/testing/local-development.md §12.

## Files likely affected

```text
.github/workflows/ci.yml
docs/testing/local-development.md
```
