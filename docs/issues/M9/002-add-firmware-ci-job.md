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
- **A build per board profile**: `board-devkitm1` in M9, and every profile that
  exists thereafter, from the same application code. A matrix over one entry now
  is what makes the second entry a one-line change
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

Enumerate the board profiles in a matrix even while there is one of them. The
point of ADR-007's board layer is that a second ESP32-C3 board costs a profile
and not a refactor, and a CI job that hardcodes `--features board-devkitm1`
quietly makes the second profile someone's problem later. The ESP-IDF cache is
shared across matrix legs, so the marginal cost of a leg is a compile, not a
toolchain download.

## Acceptance criteria

- [ ] The job builds the firmware for `riscv32imc-esp-espidf`.
- [ ] It builds every declared board profile, from the same application code.
- [ ] Adding a board profile is a one-line matrix change.
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
