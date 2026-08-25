# Issue M9-003 — Create the firmware workspace and module layout

**Milestone:** M9 · **PRD:** [PRD 090](../../prd/090-esp32-rust-firmware.md) · **Depends on:** M9-001

## Context

ADR-001 and ADR-007. A separate workspace with a path dependency on the
contract crate — one source of truth on disk, so the two cannot drift.

## Goal

Establish the firmware project structure.

## Scope

- `firmware/esp32-node/` as its own workspace with its own toolchain pin
- `rhizo-mqtt-contract` by **path**, `default-features = false`
- `.cargo/config.toml` with target, `ldproxy` linker, `espflash` runner
- `build.rs`, `sdkconfig.defaults`
- Module layout from ADR-007: `board`, `net`, `sensors`, `pump`, `safety`, `nvs`, `app`
- The fixture test harness running on the host

## Non-goals

- Any behaviour (M9-004 onward).

## Dependencies

- M9-001

## Implementation notes

`app/` must import no `esp_idf_*` symbols — it is the host-testable
orchestration layer, and that constraint is what lets SAFETY-002, -007, and -011
be covered by `cargo test` without a board.

The path dependency, not a version dependency: the firmware workspace is not
independently publishable, which nothing requires.

## Acceptance criteria

- [ ] `cargo build --release` succeeds with no board attached.
- [ ] The contract crate is a path dependency with default features off.
- [ ] `cargo test` runs host tests in the firmware workspace.
- [ ] The protocol fixture corpus runs and passes here too.
- [ ] `grep -r esp_idf firmware/esp32-node/src/app` returns nothing.

## Verification

```bash
cd firmware/esp32-node && cargo build --release && cargo test
```

## Tests required

- The shared fixture corpus, run in this workspace.

## Documentation impact

- None.

## Files likely affected

```text
firmware/esp32-node/Cargo.toml
firmware/esp32-node/.cargo/config.toml
firmware/esp32-node/src/*
```
