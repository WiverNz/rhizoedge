# ADR-007 — ESP32 chip, Rust framework, and toolchain

## Status

Accepted — 2026-08-25. Planned for M9. **Toolchain commands verified against
upstream documentation on 2026-08-25; re-verify at the start of M9 (issue
M9-001).** The firmware Rust version is an explicit exception to the host
workspace's 1.98.0 pin — see "Rust version: the embedded toolchain exception".

## Context

The firmware must do Wi-Fi, MQTT, NVS persistence, GPIO for a pump, ADC or
UART/RS485 for sensors, and a hardware watchdog. (Wall time arrives from the Edge
over MQTT — [ADR-013](013-clock-and-time-semantics.md) — so no SNTP client is
required, though ESP-IDF provides one.) It must be written in Rust
(hard project constraint) and must not block M0–M8.

Two axes to decide: **which chip** and **which Rust stack**.

The Rust-on-ESP ecosystem offers two distinct paths:

- **`std` via ESP-IDF** — `esp-idf-svc` / `esp-idf-hal` / `esp-idf-sys` wrap
  Espressif's C SDK. Rust gets a real `std` (threads, `Mutex`, sockets), and the
  battle-tested C implementations of Wi-Fi, LwIP, mbedTLS, NVS, and MQTT.
- **`no_std` bare-metal** — `esp-hal` plus `esp-wifi`, pure Rust, no C SDK.

## Decision

### Chip: ESP32-C3 (RISC-V), primary target

| | ESP32-C3 | ESP32 / S3 (Xtensa) |
|---|---|---|
| Architecture | RISC-V | Xtensa |
| Rust compiler | **upstream Rust** — RISC-V targets are supported by the standard toolchain | requires the Espressif Rust fork, installed via `espup` |
| CI cost | low | higher (fork install per job) |
| SRAM | 400 KB | 520 KB / 512 KB + PSRAM |
| Cores | 1 | 2 |
| Wi-Fi | yes | yes |

**ESP32-C3 is chosen primarily because it is a RISC-V target supported by the
upstream Rust compiler.** Not needing a forked compiler makes CI simpler, makes
onboarding a new developer a `rustup target add` rather than a toolchain
adventure, and removes a dependency on the fork tracking upstream releases.

400 KB of SRAM and one core are ample: this firmware samples a few sensors,
publishes small JSON messages, and toggles a GPIO.

ESP32-S3 remains a documented fallback if a future requirement needs more RAM,
PSRAM, or a second core; that switch is an `espup install` and a target change,
not a rewrite, because `esp-idf-svc` covers both.

### Stack: `std` via ESP-IDF

```toml
esp-idf-svc = "0.52"     # Wi-Fi, MQTT, SNTP, NVS, event loop
esp-idf-hal = "0.45"     # GPIO, ADC, UART, I2C
esp-idf-sys = "0.36"     # raw bindings, build script
embedded-svc = "0.28"    # transport-agnostic traits
```

(Versions are indicative as of 2026-08; M9-001 pins the exact set that builds.
`esp-idf-svc` 0.52.1 was the current release at the time of writing.)

Reasons for `std` over `no_std`:

1. **The C SDK's Wi-Fi and TLS stacks are the ones Espressif actually supports.**
   Wi-Fi reconnection under adverse conditions — a device behind a flaky home
   router, all day, for months — is the single most important non-safety
   behaviour in this firmware, and it is where the mature implementation earns
   its keep.
2. **A working MQTT client with TLS is available immediately** rather than being
   a project of its own.
3. **`std` means the shared contract crate can be used directly**, and much
   firmware logic can be unit-tested on the host with `cargo test` (see
   [ADR-008](008-shared-code-simulator-and-firmware.md)).
4. **NVS is first-class and load-bearing**: command dedup across reboot
   (SAFETY-001, SAFETY-011), the persisted offline policy and budget
   (SAFETY-013, SAFETY-015). ESP-IDF also provides SNTP, which this design does
   not use — wall time comes from the Edge over MQTT — but its availability is
   worth noting should a future requirement need a second time source.

The cost is a C toolchain in the build and a larger binary. Both are acceptable
for a mains-powered indoor device. A battery-powered field node with a
multi-year life is a different problem and may well want `no_std` — that is an
M14 question, and the trait-based HAL abstraction keeps it open.

### Target triple

```text
riscv32imc-esp-espidf          # ESP32-C3, std / ESP-IDF
```

### Rust version: the embedded toolchain exception

The host workspace pins **Rust 1.98.0**
([ADR-001](001-rust-workspace-and-crate-boundaries.md), ROADMAP.md §6). The
firmware workspace is **not** required to match it, and the host workspace is
**never** downgraded to accommodate an embedded constraint. That is the reason
the firmware lives in its own Cargo workspace with its own
`rust-toolchain.toml`.

```text
Host workspace  (crates/*, ui/rhizo-ui)   Rust 1.98.0
Firmware        (firmware/esp32-node)     Rust 1.98.0 where supported,
                                          otherwise the espup-provided
                                          ESP-compatible toolchain, pinned
                                          explicitly and recorded here
```

Why this exception is expected rather than hypothetical:

- `riscv32imc-esp-espidf` is a **tier-3** target. Tier-3 targets ship no
  precompiled `std`, so building one has historically required
  `-Z build-std=std,panic_abort`, which is a nightly-only flag.
- `espup` therefore provisions its own toolchain channel (conventionally named
  `esp`) alongside the ESP-IDF C environment, and `.cargo/config.toml` in the
  firmware workspace selects it.
- Whether stock 1.98.0 can build this target directly depends on the state of
  `esp-idf-sys`, `embuild`, and tier-3 `build-std` support at the time M9 runs.
  This ADR does not guess.

**M9-001 resolves it empirically and records the answer here.** Its acceptance
criteria require executing the commands below on a real machine and correcting
this section from what actually happened. The two acceptable outcomes are:

1. `firmware/esp32-node/rust-toolchain.toml` pins `channel = "1.98.0"` and the
   build succeeds — the exception is not needed, and this subsection is trimmed
   to say so.
2. The build requires the espup-provided channel or a nightly with `build-std`.
   The exact channel name and any required flags are recorded here, and the
   divergence is isolated to the firmware workspace.

Either way the constraint is **contained**: the contract crate is `no_std` and
dependency-light by design ([ADR-008](008-shared-code-simulator-and-firmware.md)),
so it compiles under both toolchains, and M1-011's bare-metal check runs on the
host toolchain independently of any ESP tooling.

### Toolchain setup (verified 2026-08-25)

```bash
# 1. Cargo subcommands
cargo install cargo-generate
cargo install ldproxy
cargo install espflash
cargo install cargo-espflash      # optional
cargo install espup               # provisions the ESP-IDF environment

# 2. ESP toolchain + ESP-IDF environment
espup install

# 3. Source the generated export script in EVERY shell that builds firmware
#    Linux/macOS:
. $HOME/export-esp.sh
#    Windows: espup generates a PowerShell export script; M9-001 records its
#    exact path and invocation after verifying it on this machine.

# 4. Build and flash
cd firmware/esp32-node
cargo build --release
espflash flash target/riscv32imc-esp-espidf/release/esp32-node --monitor
```

Notes:

- `espup` is strictly required only for Xtensa targets; the upstream toolchain
  can build RISC-V. We use `espup` anyway because it also provisions the
  ESP-IDF C environment that `esp-idf-sys` needs, which is the harder half.
- On Linux, `libudev-dev` is required for `espflash`.
- ESP-IDF must **not** be cloned or activated manually; `esp-idf-sys` manages it.

**Anything above that fails on first contact in M9 is a bug in this ADR, not in
the developer.** M9-001's explicit job is to run these commands on a real
machine and correct this section.

### Build without a board attached

`cargo build --release` requires no hardware. This is the CI check: the firmware
must compile on every change to the shared contract crate, so that a contract
change that breaks `no_std`/embedded compatibility is caught immediately.

CI runs the firmware build in a **separate, non-blocking-for-host-tests job**,
because the ESP-IDF toolchain download is slow. It is cached aggressively and
runs only when `firmware/**` or `crates/mqtt-contract/**` changes.

### Firmware structure

```text
firmware/esp32-node/
├── Cargo.toml            # own workspace
├── rust-toolchain.toml
├── build.rs              # embuild / esp-idf-sys
├── sdkconfig.defaults
├── .cargo/config.toml    # target, linker = ldproxy, runner = espflash
└── src/
    ├── main.rs           # pump-off FIRST, then init
    ├── board.rs          # pin assignments in one place
    ├── net/              # wifi.rs, mqtt.rs, time_sync.rs
    ├── sensors/          # trait defs + fake/ + real/
    ├── pump/             # trait def + fake/ + real/
    ├── safety/           # hard limits, dedup ring, TTL check
    ├── nvs.rs            # persisted state
    └── app/              # host-testable orchestration (no ESP-IDF imports)
```

The `app/` module contains no `esp_idf_*` imports and is compiled and tested on
the host with fake adapters. That is where the safety-relevant logic lives.

### Hard limits live in the shared contract crate

`FIRMWARE_MAX_RUN_SECONDS`, `FIRMWARE_MAX_ML_PER_RUN`, `FIRMWARE_MAX_DAILY_ML`
are defined in `rhizo-mqtt-contract` so the simulator enforces byte-identical
values. `validate_water_command` is one function called by both. This is what
makes SAFETY-007 testable in M6, before any hardware exists.

## Alternatives considered

**`no_std` with `esp-hal` + `esp-wifi`.** Rejected for V1. Pure Rust is
appealing and the ecosystem has matured, but Wi-Fi robustness and TLS are where
this project cannot afford to be an early adopter, and `no_std` would forfeit
straightforward host testing of the firmware logic. Reconsider for battery field
nodes in M14.

**Arduino / C++ for the first prototype.** Explicitly rejected by project
constraint, and independently a bad idea here: it would fork the safety
validator into a second implementation in a second language, which is precisely
the divergence SAFETY-007 depends on not happening.

**ESP32 classic (Xtensa) as the primary target.** Rejected: the forked compiler
is a real, recurring cost in CI and onboarding for no benefit this firmware
needs.

**ESP32-S3 as primary.** Rejected for the same toolchain reason, but retained as
the documented fallback if RAM or a second core is ever needed.

## Consequences

Positive:

- Upstream Rust toolchain; no compiler fork for the primary target.
- Mature Wi-Fi/MQTT/TLS/NVS/SNTP from ESP-IDF.
- Firmware business logic is host-testable, so most firmware bugs are caught by
  `cargo test` rather than by a plant.
- The identical hard-limit validator runs in the simulator, making SAFETY-007
  meaningful from M6.

Negative, accepted:

- The build depends on a C SDK: first build is slow (~10 min) and the toolchain
  is heavy. Mitigated by caching and by keeping the firmware job separate.
- `esp-idf-svc` version bumps occasionally require code changes. Mitigated by
  pinning exact versions and bumping deliberately.
- Larger binary and higher idle power than `no_std`. Irrelevant for a
  mains-powered indoor node.

## Risks

- **The verified commands drift** before M9 begins. *Mitigation:* M9-001 is
  explicitly a verification issue that re-runs and corrects this section, and
  this ADR carries a re-verify note in its Status.
- **Windows toolchain friction.** The primary development machine is Windows,
  and the ESP-IDF build is better exercised on Linux. *Mitigation:* M9-001
  records the exact Windows procedure; if it proves painful, the documented
  fallback is building firmware in a Linux container while flashing from the
  host, which M9-002 covers.
- **RISC-V `esp-idf` target support regressing** in an upstream Rust release.
  *Mitigation:* the firmware workspace pins its own toolchain version, which is
  exactly why it is a separate workspace. A regression there cannot affect host
  development or CI.
- **The firmware toolchain diverging from host 1.98.0** and the two drifting
  apart over time. *Mitigation:* the only shared code is the `no_std`,
  dependency-light contract crate; M1-011 checks its bare-metal build on the
  host toolchain, and M9-002 checks the full firmware build, so divergence is
  caught by CI rather than discovered on a board.

## Follow-up

- [ADR-008](008-shared-code-simulator-and-firmware.md) — what is shared and how skew is prevented.
- [PRD 090](../prd/090-esp32-rust-firmware.md) — firmware requirements.
- M9-001 verifies and corrects the toolchain section on real hardware, and
  resolves the Rust-version exception one way or the other.
- M9-002 adds the CI firmware-build job.

## References

- Rust on ESP Book — toolchain installation:
  <https://docs.espressif.com/projects/rust/book/getting-started/toolchain.html>
- `esp-idf-template`: <https://github.com/esp-rs/esp-idf-template>
- `esp-idf-svc`: <https://github.com/esp-rs/esp-idf-svc>
- `esp-idf-hal`: <https://github.com/esp-rs/esp-idf-hal>
