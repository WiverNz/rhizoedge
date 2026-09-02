# Issue M9-003 — Create the firmware workspace and module layout

**Milestone:** M9 · **PRD:** [PRD 090](../../prd/090-esp32-rust-firmware.md) · **Depends on:** M9-001

## Context

ADR-001 and ADR-007. A separate workspace with a path dependency on the
contract crate — one source of truth on disk, so the two cannot drift.

ADR-007's 2026-08-28 amendment adds the second half of this issue.
**The official Espressif ESP32-C3-DEVKITM-1-N4X is the development and
reference board; it is not a
commitment to a product board.** The battery deployment
([ADR-018](../../adr/018-battery-and-deep-sleep-device-mode.md)) will likely run
on a Seeed XIAO ESP32-C3 or a custom ESP32-C3 PCB. Same chip, different wiring —
so the board must be a compile-time profile from the first commit, not something
retrofitted once a second board exists. Retrofitting it means editing safety
code to change hardware, which is the outcome this structure exists to prevent.

## Goal

Establish the firmware project structure, including the board layer that keeps
board wiring out of application and safety code.

## Scope

- `firmware/esp32-node/` as its own workspace with its own toolchain pin
- `rhizo-mqtt-contract` by **path**, `default-features = false`
- `.cargo/config.toml` with target, `ldproxy` linker, `espflash` runner
- `build.rs`, `sdkconfig.defaults`
- Module layout from ADR-007: `board/`, `net`, `sensors`, `pump`, `safety`,
  `nvs`, `app`
- **`src/board/`** as the sole home of board-specific detail: GPIO numbers, UART
  pins, RS485 DE/RE pins, pump-control GPIO, sensor power-enable / load-switch
  GPIO, tank and leak inputs, active-high/active-low polarity, board-specific
  peripheral construction, and any board-specific power-control pin
- Cargo features `board-devkitm1` (implemented) and the reserved name
  `board-xiao-esp32c3` (declared only when the board profile is written), with a
  `compile_error!` in `src/board/mod.rs` when zero or more than one is selected
- A board interface — trait or struct, whichever is cleaner — through which the
  rest of the firmware obtains already-constructed peripherals and never a pin
  number
- A **structural test** that fails when a literal GPIO number or pin polarity
  appears outside `src/board/`
- The fixture test harness running on the host

## Non-goals

- Any behaviour (M9-004 onward).
- A working `board-xiao-esp32c3` profile. The board is not purchased; this issue
  delivers the seam, not a second unverifiable pin map.
- Abstracting over anything but the board. ESP32-C3 is the chip commitment;
  ESP32-S3 remains ADR-007's separate fallback, not a profile.

## Dependencies

- M9-001

## Implementation notes

`app/` must import no `esp_idf_*` symbols — it is the host-testable
orchestration layer, and that constraint is what lets SAFETY-002, -007, and -011
be covered by `cargo test` without a board.

The path dependency, not a version dependency: the firmware workspace is not
independently publishable, which nothing requires.

The illustrative layout is `src/board/{mod.rs, devkitm1.rs}`, with
`xiao_esp32c3.rs` alongside them later. **Do not force that shape** if a cleaner
separation achieves the same isolation; the isolation is the requirement and the
file names are not.

Compile-time selection rather than a runtime pin table is deliberate. A runtime
table would make "which pin energises the pump" a configurable value, which is
the same category ADR-011 keeps out of messages, and it buys nothing: the board
is soldered in place before the firmware is flashed. Zero or two features
selected must be a `compile_error!` naming the available profiles — a silent
default here means a board that drives the wrong pin, and that is not a defect
to find with a pump attached.

The structural test is the load-bearing part. Board isolation that is only a
convention stops being true the first time somebody is in a hurry, exactly as
`grep -r esp_idf src/app` is what actually keeps `app/` host-testable. Match on
the forms the ESP-IDF HAL actually uses — `Gpio<n>`, `gpio<n>`, `pins.gpio<n>`,
and bare pin-number constants — and keep the allowed exception list to
`src/board/` alone. A test that can be satisfied by moving a constant into
`src/app/consts.rs` is not doing its job.

## Acceptance criteria

- [x] `cargo build --release` succeeds with no board attached.
- [x] The contract crate is a path dependency with default features off.
- [x] `cargo test` runs host tests in the firmware workspace.
- [x] The protocol fixture corpus runs and passes here too.
- [x] `grep -r esp_idf firmware/esp32-node/src/app` returns nothing.
- [x] `board-devkitm1` exists, is the documented default profile, and builds.
- [x] Building with **no** board feature fails with a `compile_error!` naming the
      available profiles; building with **two** fails the same way.
- [x] Nothing outside `src/board/` names a GPIO number, a pin polarity, or a
      board-specific peripheral constructor.
- [x] The structural board-isolation test exists, fails when a pin literal is
      planted in `src/app/`, and passes once it is removed.
- [x] The rest of the firmware obtains peripherals through the board interface
      and cannot observe which board profile is active.

## Verification

```bash
cd firmware/esp32-node
cargo build --release --features board-devkitm1
cargo test
cargo test board_isolation
# exactly one profile - both of these must fail, and say why:
cargo build --release --no-default-features 2>&1 | grep -q "board profile"
cargo build --release --features board-devkitm1,board-xiao-esp32c3 2>&1 | grep -q "board profile"
grep -rnE "(Gpio|gpio)[0-9]+" src/ --include=*.rs | grep -v "^src/board/"   # expect nothing
```

The two `cargo build` lines are expected to fail; the `grep` on their output is
the assertion. A silent success there is the defect.

## Tests required

- The shared fixture corpus, run in this workspace.
- The structural board-isolation test, including its negative case.

## Documentation impact

- [ADR-007](../../adr/007-esp32-rust-framework-and-toolchain.md) — the firmware
  structure section, corrected to the layout actually built.

## Files likely affected

```text
firmware/esp32-node/Cargo.toml
firmware/esp32-node/.cargo/config.toml
firmware/esp32-node/src/board/mod.rs
firmware/esp32-node/src/board/devkitm1.rs
firmware/esp32-node/src/*
firmware/esp32-node/tests/board_isolation.rs
```
