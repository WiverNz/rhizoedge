# ADR-007 — ESP32 chip, board portability, Rust framework, and toolchain

## Status

Accepted — 2026-08-25. **Toolchain section executed and corrected on a real
machine on 2026-09-02 (M9-001); the commands below are what actually ran, not
what upstream documentation described.** The firmware Rust version is an
explicit exception to the host workspace's 1.98.0 pin, and M9-001 resolved it:
the exception **is** required — see "Rust version: the embedded toolchain
exception".

**Amended 2026-08-31 — actual development board.** The chip commitment
(ESP32-C3) and the *development board* commitment (official Espressif
ESP32-C3-DEVKITM-1-N4X) are separated, and
board wiring is moved behind a compile-time-selected board profile. See
"Board: development board versus product board" and the revised firmware
structure. No other part of this ADR changed.

## Context

The firmware must do Wi-Fi, MQTT, NVS persistence, GPIO for a pump, ADC or
UART/RS485 for sensors, and a hardware watchdog. (Wall time arrives from the Edge
over MQTT — [ADR-013](013-clock-and-time-semantics.md) — so no SNTP client is
required, though ESP-IDF provides one.) It must be written in Rust
(hard project constraint) and must not block M0–M8.

Three axes to decide: **which chip**, **which board**, and **which Rust stack**.
The first and third were decided in August 2026; the second was left implicit,
which is the gap the 2026-08-28 amendment closes.

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

### Board: development board versus product board

The chip is a commitment. The **board** is not, and the two were previously
conflated. Four distinct things:

```text
MCU / platform choice:              ESP32-C3               committed
development board:                  ESP32-C3-DEVKITM-1-N4X initial, reference
possible battery deployment board:  Seeed XIAO ESP32-C3    candidate, unpurchased
future:                             custom ESP32-C3 PCB    must remain possible
```

**The official Espressif ESP32-C3-DEVKITM-1-N4X is the initial development and
reference board.** It is
chosen for bring-up convenience, not for deployment: a full pin header for
breadboarding, an on-board USB-to-UART bridge for serial provisioning and
`espflash --monitor`, exposed strapping pins, and a form factor that tolerates
a multimeter probe on the pump driver input — which is exactly what HIL-1
requires.

None of those properties matter in a sealed battery enclosure, and some of them
are actively wrong there: the development board's regulator and USB circuitry
draw current
that a device sleeping fourteen minutes out of every fifteen
([ADR-018](018-battery-and-deep-sleep-device-mode.md)) cannot afford. The
**Seeed XIAO ESP32-C3** is the candidate replacement for that deployment, and a
custom ESP32-C3 PCB is the plausible end state.

**The XIAO is not mandatory and is not a commitment.** It has not been purchased
or measured. This ADR names it as a candidate so the architecture is built to
accept it; it does not select it. That selection happens when the hardware
exists and M10-012 has measured something.

Therefore: **board selection belongs at the HAL/wiring boundary, and nowhere
else.** All three of those boards are ESP32-C3. They differ in pin mapping,
available GPIOs, peripheral construction, signal polarity, and low-power
hardware — and in nothing above that line. Changing the board must therefore be
a board-profile change, never a firmware refactor.

What a board change **may** alter:

- GPIO numbers, including UART and RS485 DE/RE pins
- pump-control GPIO, sensor power-enable / load-switch GPIO, tank and leak inputs
- active-high versus active-low polarity
- board-specific peripheral construction and board-specific power-control pins
- which GPIOs exist at all, and board power characteristics

What a board change **must not** alter:

- the MQTT contract, device identity semantics, or configuration semantics
- the NVS data model
- `validate_water_command`, `rhizo_policy::evaluate_offline`, or any watering
  safety rule
- the sensor traits, the pump abstraction, or the application state machine
- the Edge Controller, in any respect whatsoever

The physical board is a hardware adapter. It is not a domain architecture
choice, and the moment a GPIO number appears in `src/app/` or `src/safety/` it
has quietly become one.

### Board selection is compile-time, not runtime

Exactly one board profile is selected per firmware build, by a Cargo feature:

```text
board-devkitm1         # ESP32-C3-DEVKITM-1-N4X — first supported profile
board-xiao-esp32c3     # Seeed XIAO ESP32-C3 — added with the battery hardware
```

Zero features selected, or two, is a **compile error** with a message naming the
available profiles — a `compile_error!` in `src/board/mod.rs`, not a runtime
panic and not a silent default. A device that boots with the wrong pin map
drives the pump GPIO as something else, which is the one failure class this
project refuses to discover on hardware.

Compile-time rather than a runtime pin table because a runtime table is
configuration, and configuration for pin mapping would be remotely reachable
state that decides which pin energises a pump. ADR-011 keeps hard limits out of
messages for the same reason. It also costs nothing here: the board is soldered
in place, so nothing legitimate ever changes it after flashing.

**M9 ships `board-devkitm1` only.** The XIAO profile is written when the board
is purchased and tested. What M9 must deliver is the *seam*: adding the second
profile is a new file under `src/board/` and a feature entry, with no change to
`app/`, `safety/`, `sensors/`, `pump/`, or `net/`. That is a structural
property, and M9-003 and M9-022 check it structurally rather than trusting it.

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

### Resolved 2026-09-02 (M9-001): outcome 2, a pinned nightly with `build-std`

**Stock Rust 1.98.0 cannot build `riscv32imc-esp-espidf`.** The evidence, from
the machine:

```text
$ rustup target add riscv32imc-esp-espidf --toolchain 1.98.0-x86_64-pc-windows-msvc
error: toolchain '1.98.0-x86_64-pc-windows-msvc' has no prebuilt artifacts
       available for target 'riscv32imc-esp-espidf'
note: this may happen to a low-tier target
```

`rustc --print target-list` *knows* the target — it is a recognised tier-3
triple — but rustup distributes no `std` for it, so `std` must be built from
source with `-Z build-std=std,panic_abort`, which is nightly-only. The
firmware workspace therefore pins:

```toml
# firmware/esp32-node/rust-toolchain.toml
[toolchain]
channel = "nightly-2026-07-01"        # rustc 1.98.0-nightly (f46ec5218 2026-06-30)
components = ["rust-src", "rustfmt", "clippy"]
```

A **dated** nightly rather than `nightly`, for the same reason the host
workspace pins an exact stable: CI runs clippy with `-D warnings`, and a
floating nightly turns a green main branch red with no code change.

`rust-src` is what `build-std` compiles from, and the flag lives in
`firmware/esp32-node/.cargo/config.toml` so a bare `cargo build --release`
works:

```toml
[unstable]
build-std = ["std", "panic_abort"]
```

Three things this does **not** change, and they are the reason the exception is
tolerable:

- **The host workspace stays on 1.98.0.** It is a different workspace with its
  own `rust-toolchain.toml`, and nothing here reaches it.
- **The firmware *application* logic also stays on 1.98.0.**
  `firmware/node-app` is its own workspace on the host pin, so the safety logic
  — the actuation gate's caller, the ledger, the budget, the policy store — is
  compiled by the same compiler as the rest of the project, and its 139 tests
  run with no ESP tooling installed at all.
- **The shared crates are unaffected.** `rhizo-mqtt-contract` and `rhizo-policy`
  are `no_std` and dependency-light by design
  ([ADR-008](008-shared-code-simulator-and-firmware.md)), and M1-011's
  bare-metal check still runs on the host toolchain.

### Toolchain setup (executed 2026-09-02, M9-001)

What follows is what actually ran on the primary Windows 11 machine, in order,
from a clean shell. Three of the steps exist because of something that failed
first; those are called out.

```powershell
# 1. Cargo subcommands. `cargo-generate` is not needed -- the firmware crate
#    exists and is not generated from a template.
cargo install espup ldproxy
cargo install espflash            # needed only to flash; not needed to build

# 2. The ESP toolchain.
#
#    `--targets all --std`, NOT `--targets esp32c3`.
espup install --targets all --std

# 3. Apply the generated export script in EVERY shell that builds firmware.
#    Windows: espup writes it to the home directory as a PowerShell script.
. $HOME\export-esp.ps1
#    Linux/macOS:  . $HOME/export-esp.sh

# 4. Build. The target, linker, build-std flag, MCU and ESP-IDF version all
#    come from firmware/esp32-node/.cargo/config.toml, so this is the whole
#    command.
cd firmware/esp32-node
cargo build --release

# 5. Flash (needs a board).
espflash flash target/riscv32imc-esp-espidf/release/esp32-node --monitor
```

#### Corrections to the previous version of this section

**`espup` does *not* provision the ESP-IDF C environment.** The previous text
said it did, and gave that as the reason for using it on a RISC-V target at
all. It does not: `esp-idf-sys`'s build script clones ESP-IDF itself and
installs its own toolchain, cmake, ninja and Python virtualenv into
`<drive>/.embuild/espressif`. ESP-IDF must still **not** be cloned or activated
manually — that part was right — but for the opposite reason: `esp-idf-sys`
already owns it.

**`espup install --targets esp32c3` is not sufficient, and fails late.** For a
RISC-V-only selection espup adds the three bare-metal `riscv32*-unknown-none-elf`
targets to the stable toolchain, writes an **empty** export script, and installs
no LLVM. The build then gets all the way through cloning ESP-IDF and compiling
its ~110 C components — about six minutes — before failing in bindgen with:

```text
Unable to find libclang: "couldn't find any valid shared libraries matching:
['clang.dll', 'libclang.dll'], set the `LIBCLANG_PATH` environment variable"
```

The `libclang` `esp-idf-sys` needs ships with the LLVM espup installs alongside
the **Xtensa** toolchain, which is why `--targets all` is required even though
nothing here builds for Xtensa. `--std` skips the GCC install, which
`esp-idf-sys` provides.

**Both halves of the export script are load-bearing on Windows.** It sets
`LIBCLANG_PATH` to the DLL *and* prepends the `esp-clang/bin` directory to
`PATH`. Setting only `LIBCLANG_PATH` fails with
`LoadLibraryExW failed` — the DLL is found and cannot be loaded, because its
dependencies (`libwinpthread-1.dll` and friends) live in that directory.
Measured both ways.

#### Windows: the path-length limit, which is a hard stop

`esp-idf-sys` refuses an output directory longer than **88 characters** after
canonicalisation:

```text
Error: Too long output directory: `\\?\D:\Projects\rhizoedge\firmware\esp32-node\target\...`.
Shorten your project path down to no more than 10 characters (or use WSL2 and
its native Linux filesystem). Note that tricks like Windows `subst` do NOT work!
```

The check is on `OUT_DIR`, and the fixed suffix cargo appends
(`\riscv32imc-esp-espidf\release\build\esp-idf-sys-<16 hex>\out`, plus the
`\\?\` prefix) is 73 characters on its own — so **`CARGO_TARGET_DIR` must be
15 characters or fewer**. This repository's natural target directory is 47.

The measure that works, and the one this project uses:

```powershell
$env:CARGO_TARGET_DIR = "D:\rzt"      # any short path on any drive
```

`ESP_IDF_PATH_ISSUES=warn` downgrades the check to a warning, and is recorded
here only so nobody has to find it: it does not make the underlying Windows
`MAX_PATH` problem go away, it just moves where the build fails.

#### The Linux-container fallback

Still the documented alternative, and M9-002's CI job is the reference for it:
build in a Linux environment and flash from the host. On this machine the
native Windows path works with the short `CARGO_TARGET_DIR`, so the fallback is
not required — but it remains the answer if a future ESP-IDF version tightens
the path limit further.

#### Versions that actually build

| Component | Version |
|---|---|
| Firmware toolchain | `nightly-2026-07-01` (rustc 1.98.0-nightly, f46ec5218) |
| Host toolchain (unchanged) | 1.98.0 |
| Target | `riscv32imc-esp-espidf` |
| `esp-idf-svc` | **0.52.1** |
| `esp-idf-hal` | **0.46.2** (the ADR previously guessed 0.45) |
| `esp-idf-sys` | **0.37.2** (previously guessed 0.36) |
| `embedded-svc` | **0.29.0** (previously guessed 0.28) |
| `embuild` (build-dependency) | 0.33.4 |
| ESP-IDF | **v5.5.1**, pinned in `.cargo/config.toml` |
| `bindgen` (transitive) | 0.71.1 |
| `espup` | 0.17.1 |
| `ldproxy` | 0.3.5 |
| Python (host, accepted by IDF 5.5.1) | 3.14.2 |
| git | 2.55.0 |

Every one is pinned with `=` in `firmware/esp32-node/Cargo.toml`.

#### Environment

```text
CARGO_TARGET_DIR   a path of 15 characters or fewer (Windows only, see above)
LIBCLANG_PATH      set by export-esp.ps1 / export-esp.sh
PATH               must include the esp-clang/bin directory (same script)
MCU                esp32c3          } both set in .cargo/config.toml, so they
ESP_IDF_VERSION    v5.5.1           } are not a per-shell concern
```

#### Build times, measured

| | |
|---|---|
| espup + ldproxy install | ~2 min |
| ESP-IDF clone, tool install, and C build (first time) | **341 s**, ~5.4 GB in `.embuild` |
| Rust compile after that, first time | ~30 s |
| Warm rebuild after a source edit | **~2.5 s** |
| Rebuild after `cargo clean -p esp-idf-sys` | ~30 s |

So a genuinely cold build is about **six and a half minutes** plus downloads,
and every build after that is seconds. The "~10 min" this ADR estimated was the
right order of magnitude.

#### Offline and repeated builds

Verified. Once `.embuild` and the cargo registry are populated, repeated builds
need no network: ESP-IDF is cloned once to a pinned tag and reused, and the
`=`-pinned crate versions resolve from the local registry.

### Build without a board attached

`cargo build --release` requires no hardware. This is the CI check: the firmware
must compile on every change to the shared contract crate, so that a contract
change that breaks `no_std`/embedded compatibility is caught immediately.

CI runs the firmware build in a **separate, non-blocking-for-host-tests job**,
because the ESP-IDF toolchain download is slow. It is cached aggressively and
runs only when `firmware/**` or `crates/mqtt-contract/**` changes.

### Firmware structure (as built, M9-003)

```text
firmware/
├── node-app/                 # WORKSPACE A -- host toolchain 1.98.0
│   ├── Cargo.toml            #   no ESP-IDF dependency, by construction
│   ├── rust-toolchain.toml   #   1.98.0, the project MSRV
│   ├── src/                  #   ports, fakes, boot, persist, identity,
│   │                         #   command, ledger, dedup, recovery, config,
│   │                         #   policy, offline, budget, buffer, power,
│   │                         #   awake_hold, sampling, telemetry, provision
│   └── tests/                #   conformance, board_isolation,
│                             #   single_actuation_path
└── esp32-node/               # WORKSPACE B -- nightly + build-std, ESP target
    ├── Cargo.toml            #   board-* features declared here
    ├── rust-toolchain.toml   #   nightly-2026-07-01 (see the exception above)
    ├── build.rs              #   embuild / esp-idf-sys
    ├── sdkconfig.defaults
    ├── .cargo/config.toml    #   target, ldproxy, espflash, build-std, MCU
    └── src/
        ├── main.rs           #   pump-off FIRST, then init, then the loop
        ├── run.rs            #   the session loop: moves bytes and time only
        ├── board/            #   THE ONLY place a GPIO number may appear
        │   ├── mod.rs        #     profile selection, the trait, compile_error!
        │   └── devkitm1.rs   #     ESP32-C3-DEVKITM-1-N4X pin map
        ├── hal/              #   ESP-IDF adapters for node-app's traits
        └── net/              #   wifi.rs, mqtt.rs, session.rs
```

**This differs from the layout sketched above, and the difference is an
improvement.** The sketch put the host-testable layer in
`firmware/esp32-node/src/app/` and enforced "no `esp_idf_*` import" with a
grep. Splitting it into its own crate makes the same property structural — the
crate has no ESP-IDF dependency, so an ESP-IDF symbol there does not compile —
and this ADR already permitted it: *"a cleaner separation that achieves the
same isolation is acceptable, and the isolation is the requirement."*

Two consequences, both wanted:

- the safety logic is compiled and tested by the **host** toolchain pin
  (1.98.0), not by the nightly `riscv32imc-esp-espidf` requires;
- `cargo test` in `firmware/node-app` needs no target flag, no `build-std`, and
  no ESP-IDF installation, so a contributor with none of that can still run the
  safety suite.

`src/board/` is the board layer, and it is the **only** place a literal GPIO
number, a pin polarity, or a board-specific peripheral construction may appear.
Everything above it receives already-constructed trait objects and cannot
observe which board it is running on.

Time synchronisation has no `time_sync.rs`: the `edge.time` rules live in
`rhizo_mqtt_contract::payload::TimeSyncState`, which the simulator uses too, and
`hal/clock.rs` holds one rather than reimplementing it. A second copy on the
device is how a device comes to claim synchronisation it does not have, and
there is no way to detect that from the edge.

`src/board/` is the board layer, and it is the **only** place a literal GPIO
number, a pin polarity, or a board-specific peripheral construction may appear.
Everything above it receives already-constructed trait objects — `Pump`,
`SoilSensor`, `PowerRail`, and the rest of M9-005's set — and cannot observe
which board it is running on. The exact file names above are illustrative: a
cleaner separation that achieves the same isolation is acceptable, and the
isolation is the requirement.

This is one board layer with two profiles, **not two firmwares**. Duplicating
the firmware per board would duplicate the safety path, which is the same
mistake as a second `validate_water_command`.

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

**Committing to one board and hardcoding its pins.** Rejected. It is the cheapest
thing to do in M9 and the most expensive thing to undo in M10–M14: the pin
constants would be read by the pump driver, the rail control, the RS485 setup,
and the boot-safe sequence, so moving to a battery board would edit safety code
to change hardware. The board layer costs one indirection now and makes that
move a new file later.

**Starting on the XIAO ESP32-C3 directly.** Rejected for now. It is the likely
deployment board, but it is not purchased, and bringing up unfamiliar firmware
on a board with few exposed pins, no header, and no comfortable place to attach
a multimeter would make HIL-1 — the one gate that genuinely matters in M9 —
harder for no gain. Develop on the official DEVKITM-1-N4X, then deploy on the
compact ESP32-C3 board that measures best.

**A runtime pin table in NVS or in device config.** Rejected. It makes the
mapping between a GPIO and the pump a remotely reachable value, which is
precisely the category ADR-011 keeps out of messages. The board does not change
after it is soldered, so the flexibility buys nothing and the risk is real.

**Building a separate firmware image per board.** Rejected for the same reason
there is one `validate_water_command` and one image across power modes
([ADR-018](018-battery-and-deep-sleep-device-mode.md)): two images are two safety
paths, and M9-014's conformance test would cover only one of them. One image,
one board profile chosen at compile time.

## Consequences

Positive:

- Upstream Rust toolchain; no compiler fork for the primary target.
- Mature Wi-Fi/MQTT/TLS/NVS/SNTP from ESP-IDF.
- Firmware business logic is host-testable, so most firmware bugs are caught by
  `cargo test` rather than by a plant.
- The identical hard-limit validator runs in the simulator, making SAFETY-007
  meaningful from M6.
- Development happens on convenient hardware while the deployment board stays
  an open question that can be answered with measurements rather than guessed
  now.
- Adding a board is a new file under `src/board/` and a feature entry; no
  application, safety, or protocol code is touched, and the host tests for
  `app/` are board-independent by construction.

Negative, accepted:

- The build depends on a C SDK: first build is slow (~10 min) and the toolchain
  is heavy. Mitigated by caching and by keeping the firmware job separate.
- `esp-idf-svc` version bumps occasionally require code changes. Mitigated by
  pinning exact versions and bumping deliberately.
- Larger binary and higher idle power than `no_std`. Irrelevant for a
  mains-powered indoor node.
- One indirection between the application and its pins, and a small amount of
  duplicated shape per board profile. Accepted: it is the price of not editing
  safety code to change hardware.
- The XIAO profile is unverified until the board exists, so the second-profile
  claim is architectural until then. M9-022 checks the seam structurally; only
  buying the board proves it.

## Risks

- **The verified commands drift.** *Mitigation:* M9-001 executed and corrected
  this section on 2026-09-02, and M9-002's CI job runs the same install and
  build on every change to `firmware/**` or either shared crate — so drift
  fails a job rather than a developer.
- **Windows toolchain friction.** *Realised, and resolved.* Three distinct
  problems, all recorded above with their measures: espup installs no
  `libclang` for a RISC-V-only selection, the export script's `PATH` entry is
  required and not just `LIBCLANG_PATH`, and `esp-idf-sys` refuses an output
  directory over 88 characters. With a short `CARGO_TARGET_DIR` the native
  Windows build works; the Linux-container fallback remains documented and is
  what CI exercises.
- **RISC-V `esp-idf` target support regressing** in an upstream Rust release.
  *Mitigation:* the firmware workspace pins its own toolchain version, which is
  exactly why it is a separate workspace. A regression there cannot affect host
  development or CI.
- **The board layer leaking.** A GPIO number reaches `app/` or `safety/` in a
  hurry, and the abstraction is decorative from then on. *Mitigation:* M9-003
  adds a structural check that fails the firmware test suite when a literal pin
  assignment appears outside `src/board/`, and M9-022 makes it an exit
  criterion. A convention nobody checks is not a boundary.
- **The second board profile never being exercised** and rotting until the
  battery hardware arrives. *Mitigation:* the requirement is the seam, not the
  second profile; once `board-xiao-esp32c3` exists, M9-002's CI job builds both
  profiles from the same application code, and a profile that stops compiling
  fails the build rather than surfacing on a board.
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
- M9-002 adds the CI firmware-build job, and builds every board profile that
  exists.
- M9-003 creates `src/board/`, the `board-devkitm1` profile, the
  exactly-one-profile compile error, and the pin-leak check.
- M9-005, M9-007, and M9-020 take their pins, polarities, and rail control from
  the board profile rather than defining them.
- [ADR-018](018-battery-and-deep-sleep-device-mode.md) — the battery deployment
  this board seam exists to serve.

## References

- Rust on ESP Book — toolchain installation:
  <https://docs.espressif.com/projects/rust/book/getting-started/toolchain.html>
- `esp-idf-template`: <https://github.com/esp-rs/esp-idf-template>
- `esp-idf-svc`: <https://github.com/esp-rs/esp-idf-svc>
- `esp-idf-hal`: <https://github.com/esp-rs/esp-idf-hal>
- Official ESP32-C3-DevKitM-1 user guide:
  <https://docs.espressif.com/projects/esp-dev-kits/en/latest/esp32c3/esp32-c3-devkitm-1/user_guide.html>
- Seeed XIAO ESP32-C3 (candidate battery board, unpurchased):
  <https://wiki.seeedstudio.com/XIAO_ESP32C3_Getting_Started/>
