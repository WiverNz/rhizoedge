# Issue M9-001 — Verify and correct the ESP32 toolchain documentation

**Milestone:** M9 · **PRD:** [PRD 090](../../prd/090-esp32-rust-firmware.md) · **Depends on:** M8-018

## Context

ADR-007 records toolchain commands verified against upstream documentation in
August 2026, with an explicit note to re-verify at the start of M9. Anything
that fails on first contact is a bug in the ADR, not in the developer.

## Goal

Execute every documented command on a real machine and correct ADR-007.

## Scope

- Run `espup install`, source the export script, add the target
- Build the `esp-idf-template` hello-world for ESP32-C3
- Record the exact Windows procedure including the PowerShell export script path
- Record exact `esp-idf-svc`/`esp-idf-hal`/`esp-idf-sys` versions that build
- **Determine whether stock Rust 1.98.0 builds `riscv32imc-esp-espidf`**, or whether the espup channel / `-Z build-std` is required; pin the answer in `firmware/esp32-node/rust-toolchain.toml`
- Correct ADR-007's toolchain section from what actually happened

## Non-goals

- Writing firmware (M9-003 onward).

## Dependencies

- M8-018

## Implementation notes

The Windows path is the risk: the primary machine is Windows and ESP-IDF is
better exercised on Linux. If it proves painful, the documented fallback is
building in a Linux container and flashing from the host — cover that in M9-002
rather than fighting it here.

Record versions that actually build, not the ones the ADR guessed. Pin them.

## Acceptance criteria

- [ ] Every command in ADR-007 has been executed.
- [ ] ADR-007 is corrected to match reality, including on Windows.
- [ ] A hello-world builds for `riscv32imc-esp-espidf`.
- [ ] Exact crate versions are recorded and pinned.
- [ ] The firmware Rust version is resolved: either `1.98.0` or a named
      ESP-specific channel, pinned in the firmware workspace and recorded in
      ADR-007's embedded-exception section.
- [ ] The host workspace toolchain is **unchanged** at 1.98.0.
- [ ] Build time from cold is measured and noted.
- [ ] The re-verify note in ADR-007's Status is updated with the date.

## Verification

```bash
espup install
cargo build --release --target riscv32imc-esp-espidf
```

## Tests required

- Manual execution; the ADR is the artefact.

## Documentation impact

- ADR-007 corrected.

## Files likely affected

```text
docs/adr/007-esp32-rust-framework-and-toolchain.md
```
