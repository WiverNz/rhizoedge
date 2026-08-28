# Issue M1-001 — Create the no_std mqtt-contract crate skeleton

**Milestone:** M1 · **PRD:** [PRD 010](../../prd/010-domain-and-mqtt-protocol.md) · **Depends on:** M0-002, M0-003

## Context

ADR-001 makes `rhizo-mqtt-contract` the firmware-facing wire crate; ADR-015 later
adds `rhizo-policy` as the second firmware-facing shared crate. Both must remain
`no_std`. Retrofitting `no_std` onto
a crate that grew `std` habits is far harder than starting that way.

## Goal

Establish the crate with `no_std` + `alloc`, a `std` feature, and the module layout.

## Scope

- `#![no_std]` with `extern crate alloc`
- A `std` feature adding only `std::error::Error` impls — nothing semantic
- Module layout: `ids`, `time`, `topic`, `envelope`, `payload`, `safety`, `validation`
- `serde` with `default-features = false`, `features = ["alloc", "derive"]`
- No `chrono`, no `tokio`, no I/O of any kind

## Non-goals

- Any concrete type (M1-002 onward).

## Dependencies

- M0-002
- M0-003

## Implementation notes

The `std` feature must not change behaviour, only add trait impls. A feature
that alters semantics would mean the firmware and the edge disagree about the
protocol, which is exactly what ADR-008 exists to prevent.

Watch the serde feature flags: the default features pull in `std` transitively
and the breakage is invisible until M1-011 runs.

## Acceptance criteria

- [x] `cargo build -p rhizo-mqtt-contract` succeeds.
- [x] `cargo build -p rhizo-mqtt-contract --no-default-features` succeeds.
- [x] The crate depends on no workspace crate.
- [x] `grep -r 'use std::' crates/mqtt-contract/src` returns nothing outside `#[cfg(feature = "std")]`.

## Verification

```bash
cargo build -p rhizo-mqtt-contract --no-default-features
cargo tree -p rhizo-mqtt-contract
```

## Tests required

- A compile-only test asserting the crate builds without default features.

## Documentation impact

- Crate docs stating the no_std constraint and why.

## Files likely affected

```text
crates/mqtt-contract/Cargo.toml
crates/mqtt-contract/src/lib.rs
```
