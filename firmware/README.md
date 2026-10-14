# Rhizo Edge firmware

Two workspaces, deliberately.

```text
firmware/
├── node-app/       the application logic — host toolchain, no ESP-IDF
└── esp32-node/     the ESP32-C3 image — nightly + build-std, cross-compiled
```

`node-app` holds every safety-relevant decision: the actuation gate's caller,
the pending-result ledger, the monotonic budget, the policy store, the event
buffer, the wake machine. It has **no ESP-IDF dependency**, so it builds and
tests on any machine with a Rust toolchain — no board, no ESP-IDF installation,
no nightly. `esp32-node` supplies the ESP-IDF implementations of its traits and
the board pin map, and is the crate that produces a flashable image.

That split is [ADR-007](../docs/adr/007-esp32-rust-framework-and-toolchain.md)'s
requirement — "`app/` contains no `esp_idf_*` imports and is host-testable" —
made structural rather than grep-enforced.

---

## The safety suite: no setup at all

```bash
cd firmware/node-app
cargo test          # 139 tests, seconds, no board and no ESP-IDF
```

This is where SAFETY-001, -002, -007, -011, -012, -013, -015, -016, -019, -020
and -021 are covered, where the simulator/firmware conformance suite lives
(M9-014), and where the structural board-isolation checks run. If you are
changing firmware behaviour, this is the loop to work in.

---

## Building the image

The full procedure, its findings, and its measured timings are in
[ADR-007](../docs/adr/007-esp32-rust-framework-and-toolchain.md) §Toolchain
setup. The short version:

```bash
cargo install espup ldproxy
cargo install espflash            # only needed to flash

# --targets all, NOT --targets esp32c3. A RISC-V-only install provides no
# libclang and the build fails in bindgen six minutes in.
espup install --targets all --std

# Every shell that builds firmware. Sets LIBCLANG_PATH *and* PATH; both matter.
. $HOME/export-esp.sh             # Windows: . $HOME\export-esp.ps1

cd firmware/esp32-node
cargo build --release --ignore-rust-version
```

The ESP32 image retains `nightly-2026-07-01`. Its build commands use
`--ignore-rust-version` because the shared crates and `node-app` declare the
host MSRV, now 1.99.0. This bypasses Cargo metadata checking only; the image
build and Clippy still verify compiler compatibility. Do not use this flag
for the root workspace or `firmware/node-app` host checks.

### Windows: `CARGO_TARGET_DIR` must be short

`esp-idf-sys` refuses an output directory over 88 characters after
canonicalisation, and cargo's fixed suffix uses 73 of them — so the target
directory must be **15 characters or fewer**. This repository's natural one is
47.

```powershell
$env:CARGO_TARGET_DIR = "D:\rzt-fw"
```

Without it the build fails with `Too long output directory`. `subst` does not
help; the message says so.

### The Linux-container fallback

Building in a Linux environment and flashing from the host remains the
documented alternative, and CI's `firmware-image` job is the reference for what
that needs. On Windows the native build works with the short target directory,
so the fallback is a fallback rather than the default.

### Cold build

About six and a half minutes plus downloads the first time — `esp-idf-sys`
clones ESP-IDF v5.5.1 and builds its C components — and roughly 2.5 seconds for
a warm rebuild after a source edit. Repeated builds need no network.

---

## Reaching the board from WSL2 (Windows)

On a Windows host that flashes from WSL2, the board's USB-UART bridge has to be
handed to WSL with [usbipd-win](https://github.com/dorssel/usbipd-win). Two
PowerShell helpers do that:

```powershell
.\scripts\esp-wsl-attach.ps1    # board -> WSL2, appears as /dev/ttyUSB0
.\scripts\esp-wsl-detach.ps1    # board -> Windows, appears as a COM port again
```

- **Prerequisite:** `usbipd-win` installed on Windows.
- **One-time bind, as Administrator.** A device usbipd has never shared shows
  as `Not shared`, and the attach helper stops and prints the command to run in
  an elevated PowerShell — `usbipd bind --busid <BUSID>`. It never elevates
  itself. After that, attach and detach need no Administrator rights.
- **The BUSID is discovered, not configured.** It depends on the USB port and
  changes when the cable moves, so the helpers find the bridge by VID:PID
  `10c4:ea60` (Silicon Labs CP210x, including the board's CP2102N) on every run.
- **More than one matching CP210x connected:** the helpers list the matches
  and refuse to guess. Unplug the extra one, or call `usbipd` with an explicit
  `--busid`.
- **Both are idempotent.** Attaching an attached board or detaching a detached
  one reports it and exits 0.
- **Re-run attach after a WSL restart, `wsl --shutdown`, or unplugging the
  board.** Attachment does not survive any of them.

While the board is attached to WSL, Windows serial tools cannot open it.

> **After using the board from Windows, replug it before attaching it to WSL
> again.** Once a Windows program has opened the COM port, `usbipd attach` can
> refuse with `Device busy (exported)` / "appears to be used by Windows" even
> though nothing visibly holds the port. Retrying the attach in that state was
> followed by a Windows bugcheck (BSOD) on the development machine. Unplug and
> replug the board instead. Do not reach for `usbipd bind --force`.

### Is the board ready? — `esp-status`

**Run this first** whenever you, or an agent, are unsure whether the real
ESP32-C3 is usable right now:

```powershell
.\scripts\esp-status.ps1           # native Windows (PowerShell 5.1 or 7)
.\scripts\esp-status.ps1 -Json
```

```bash
./scripts/esp-status.sh            # inside WSL2 or Linux
./scripts/esp-status.sh --json
```

It finds the bridge by VID:PID `10c4:ea60` (it never assumes `COM5` or
`/dev/ttyUSB0`), checks that the port can be opened, and reads the chip with
`espflash board-info`. It reports the chip, revision, flash size, crystal,
radios, and Secure Boot / Flash Encryption state. It never flashes, erases,
binds, attaches, or detaches, but `board-info` does pulse the reset line, so
running firmware restarts. The JSON output leaves out the MAC address.

| Exit | State | Meaning |
|---|---|---|
| 0 | `READY` | verified, usable from this environment now |
| 2 | `UNSUPPORTED` | `.ps1` run outside Windows (use the `.sh`); also the `.sh`'s usage error |
| 10 | `ESPFLASH MISSING` | `espflash` not found |
| 11 | `NOT CONNECTED` | no matching bridge visible; inside WSL this usually means "not attached" — run `.\scripts\esp-wsl-attach.ps1` from Windows |
| 12 | `PERMISSION ERROR` | the tty exists but this user cannot open it (usually not in `dialout`) |
| 13 | `ATTACHED TO WSL` | Windows only: present but owned by WSL2. **Not a fault.** `.\scripts\esp-wsl-detach.ps1` returns it |
| 14 | `WRONG DEVICE` | the bridge answers, but the chip is not an ESP32-C3 |
| 15 | `BOARD NOT RESPONDING` | the port opened but the chip did not answer within 60 s |
| 16 | `MULTIPLE DEVICES` | more than one CP210x; refuses to guess |
| 17 | `PORT BUSY` | another program holds the port (a serial monitor, usually) |

Inside an agent sandbox the serial device and supplementary groups can be
hidden, which reads as `NOT CONNECTED` or `PERMISSION ERROR`. Re-run outside the
sandbox before concluding anything about the hardware.

## Flashing and monitoring

```bash
espflash flash --monitor "$CARGO_TARGET_DIR/riscv32imc-esp-espidf/release/esp32-node"
```

On Linux, `espflash` needs `libudev-dev`.

> **Before a pump is ever wired to the board**, put a meter on the pump driver
> input and confirm it reads inactive from power-on, through the bootloader, and
> into `main`. The window before any Rust runs is not coverable from firmware —
> only the external pull-down documented in `esp32-node/src/board/devkitm1.rs`
> covers it. That is HIL-1's first case and it is not optional.

---

## Provisioning

One firmware image serves every device: no binary contains a device name and no
binary contains a secret ([ADR-012](../docs/adr/012-device-identity-and-provisioning.md)).

Connect the serial console and, before the network comes up:

```text
provision wifi <ssid> <psk>
provision mqtt <host> <user> <pass>
provision device-id <id>        # optional; otherwise derived from the MAC
provision show                  # secrets redacted, always
provision commit
```

Then reboot. The device derives `plant-node-<3-byte MAC hex>` when no identity
is set, connects, publishes its retained status, and the edge auto-registers it
with no plant attached.

The console is closed once the network is up. `provision unlock` reopens it — a
serial console reachable at runtime is a credential-disclosure path on a device
somebody might put in a shared space, so reopening it is explicit.

---

## Board profiles

Exactly one per build, selected by a Cargo feature:

```bash
cargo build --release --ignore-rust-version --no-default-features --features board-devkitm1
```

| Profile | Board | State |
|---|---|---|
| `board-devkitm1` | Espressif ESP32-C3-DEVKITM-1-N4X | the reference board; the default |
| `board-xiao-esp32c3` | Seeed XIAO ESP32-C3 | **reserved name, no pin map** |

Zero or two profiles is a `compile_error!` naming the available profiles — not a
runtime default, because a device that boots with the wrong pin map drives the
pump GPIO as something else.

Adding a board is a new file in `src/board/`, a feature entry, and a line in
CI's matrix. It is not a change to application, safety, sensor, pump, or
networking code, and `node-app/tests/board_isolation.rs` fails if it becomes
one.

---

## What is verified and what is not

`docs/reports/M9.md` labels every criterion as **VERIFIED (no hardware)**,
**IMPLEMENTED, HARDWARE VERIFICATION PENDING**, or **NOT STARTED**. As of the
2026-09-02 board-free pass, everything involving a radio, real flash, the RTC
domain, deep sleep, or a GPIO measured with a meter is in the second category.
A compile is not a board.
