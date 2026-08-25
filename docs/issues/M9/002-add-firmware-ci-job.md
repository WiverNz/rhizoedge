# Issue M9-002 — Add the firmware build CI job

**Milestone:** M9 · **PRD:** [PRD 090](../../prd/090-esp32-rust-firmware.md) · **Depends on:** M9-001

## Context

ADR-008: CI must build the firmware whenever the shared contract crate
changes, because that is how a contract edit breaking embedded compatibility is
caught.

## Goal

Build the firmware in CI on relevant changes.

## Scope

- A job triggered by `firmware/**` or `crates/mqtt-contract/**`
- Aggressive caching of the ESP-IDF toolchain
- Separate from the host test job so its slowness does not gate everything
- A documented Linux-container fallback for local Windows builds

## Non-goals

- Flashing or hardware tests in CI.

## Dependencies

- M9-001

## Implementation notes

Cache the ESP-IDF installation, not just Cargo — the SDK download dominates
a cold build.

Keep the job non-blocking for host tests but blocking for merge. The contract
crate's `no_std` check (M1-011) already runs on every change and is fast; this
job is the deeper verification.

## Acceptance criteria

- [ ] The job builds the firmware for `riscv32imc-esp-espidf`.
- [ ] It triggers on the specified paths.
- [ ] Caching keeps warm builds under 5 minutes.
- [ ] A contract change that breaks the firmware fails it.
- [ ] The Linux-container fallback is documented.

## Verification

```bash
# observe a CI run touching crates/mqtt-contract/
```

## Tests required

- Manual: break embedded compatibility, confirm the job fails, revert.

## Documentation impact

- ADR-007 follow-up; local-development.md firmware note.

## Files likely affected

```text
.github/workflows/ci.yml
```
