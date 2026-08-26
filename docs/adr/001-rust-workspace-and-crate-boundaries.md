# ADR-001 — Rust workspace and crate boundaries

## Status

Accepted — 2026-08-25. Implemented in M0/M1.

## Context

Rhizo Edge has five deliverables that share meaning but not runtime: an ESP32
firmware, a host simulator, an edge service, a cloud service, and a desktop UI.
Two constraints shape the layout:

1. **The firmware and the host services cannot live in the same Cargo
   workspace.** They need different target triples, different `std`
   implementations, different toolchains, and — for ESP-IDF — a build script
   that pulls in a C SDK. A single workspace would force `cargo test
   --workspace` to attempt an embedded build.
2. **The safety logic must be testable without any I/O.** If watering decisions
   are entangled with `sqlx` and `rumqttc`, then property-testing SAFETY-006
   requires a database and a broker, which in practice means it will not be
   property-tested.

Additionally, the project mandates Rust everywhere and forbids Go, Node.js, and
TypeScript.

## Decision

### Three workspaces

```text
/                        root workspace: crates/*  (host, x86_64/aarch64)
/firmware/esp32-node     own workspace: riscv32imc-esp-espidf
/ui/rhizo-ui             own workspace: host + wasm32-unknown-unknown
```

The root `Cargo.toml` uses `exclude = ["firmware", "ui"]` so
`cargo test --workspace` at the root is exactly "all host code" and never
attempts a cross build. Each nested workspace has its own lockfile and its own
CI job.

### Root workspace crates

| Crate | Kind | Depends on | Purpose |
|---|---|---|---|
| `rhizo-mqtt-contract` | lib, `no_std`+`alloc` | — | wire format, topic grammar, measurement kinds, shared validators |
| `rhizo-policy` | lib, `no_std`+`alloc`, pure | mqtt-contract | offline policy types and `evaluate_offline` ([ADR-015](015-device-offline-autonomy.md)) |
| `rhizo-domain` | lib, std, pure | mqtt-contract, policy | plant/irrigation logic, safety gate |
| `rhizo-storage` | lib | domain, mqtt-contract | SQLite schema, repos, transactions |
| `rhizo-telemetry` | lib | — | tracing + metrics wiring |
| `rhizo-cloud-client` | lib | domain | HTTP client for the cloud |
| `rhizo-testkit` | lib | domain, mqtt-contract, storage | fixtures, TestClock, assertions |
| `edge-controller` | bin | all of the above | the control plane |
| `device-simulator` | bin | mqtt-contract, telemetry, testkit | the reference device |
| `cloud-api` | bin | mqtt-contract, telemetry | ingest + read APIs |

### The five boundary rules

1. `rhizo-mqtt-contract` depends on **nothing** in this workspace and performs
   no I/O. It is the crate the firmware imports.
2. `rhizo-policy` is `no_std`, pure, and depends only on `mqtt-contract`. It is
   the **second** crate the firmware imports, and the only place the offline
   decision rules exist. It never reads a clock: elapsed time arrives as a
   parameter (`MonotonicMillis`).
3. `rhizo-domain` performs **no I/O and never reads a clock directly**. Every
   decision function is pure: `fn(inputs) -> decision`. It links `rhizo-policy`
   so the Edge can validate a policy before publishing it and predict what an
   isolated device will do.
4. `rhizo-storage` holds transactions but **no decisions**.
5. Binaries depend on libraries. Libraries never depend on binaries. Integration
   tests are the only thing allowed to depend on `edge-controller`.

### Shared dependency versions

The root workspace uses `[workspace.dependencies]` and every member writes
`tokio = { workspace = true }`. This prevents the common failure where two
crates disagree on a `chrono` or `uuid` version and types stop unifying.

### Rust version policy: MSRV 1.98.0, pin may move forward

Two distinct things, previously conflated:

```text
MSRV                  1.98.0   the minimum host Rust the project supports
current tested pin    1.98.0   what rust-toolchain.toml selects today
future pin            may move to any newer stable, deliberately
```

- **MSRV is 1.98.0.** The host workspace and the UI must keep compiling on it.
- **`rust-toolchain.toml` currently pins exactly `1.98.0`**, and that file is the
  truth about what CI and developers run today. An exact pin — never `stable` —
  is required so that `-D warnings` is reproducible: a new clippy lint in a later
  release must not turn a green main branch red with no code change.
- **The pin may be raised to a newer stable** as a deliberate, standalone change.
  Raising the pin does not by itself raise the MSRV.
- **No change may silently raise the MSRV.** Using a feature stabilised after
  1.98.0 is an architectural decision requiring an explicit note in the change
  and an update to this ADR, `README.md`, and `ROADMAP.md` §6.
- **Nothing is ever downgraded below 1.98.0**, including to match an embedded
  toolchain constraint ([ADR-007](007-esp32-rust-framework-and-toolchain.md)).

The UI workspace uses the same version plus the `wasm32-unknown-unknown` target.
The firmware workspace pins its own toolchain independently.

When it becomes useful, CI verifies **both** the MSRV and current stable, so an
accidental MSRV bump fails the build rather than being discovered by a user on
an older toolchain. That job is planned in M13, not required earlier.

## Alternatives considered

**One workspace containing the firmware.** Rejected: `cargo test --workspace`
would try to build for the ESP target, and `cargo build` at the root would drag
in the ESP-IDF build script for every developer including those who never touch
hardware. The `exclude` approach costs one extra CI job and buys a workspace
that behaves normally.

**A single `rhizo-core` crate instead of `mqtt-contract` + `domain`.** Rejected:
the firmware needs the wire types but must not receive the irrigation engine —
it deliberately contains no irrigation intelligence. Merging them would either
push `std`-only logic into the firmware or force the whole domain to be
`no_std`, which would make the recommendation engine painful to write for no
benefit.

**Putting the safety gate in `edge-controller` directly.** Rejected: it would
make the safety property tests depend on a Tokio runtime and a database. The
purity of `rhizo-domain` is what makes the SAFETY-nnn tests cheap enough that
they actually get written.

**A `firmware` crate sharing the workspace via target-specific dependencies.**
Rejected as fragile; `cargo` resolves features across the whole workspace, so a
`std`-enabling feature elsewhere would silently break the `no_std` build.

## Consequences

Positive:

- `cargo test --workspace --all-features` at the root is fast, hermetic, and
  requires no hardware or network.
- The dependency direction makes several classes of bug structurally impossible:
  the domain cannot accidentally consult the cloud (SAFETY-009), and the
  firmware cannot accidentally import `sqlx`.
- Safety logic is property-testable in milliseconds.

Negative, accepted:

- Three lockfiles and three CI jobs to keep in sync.
- A change to `rhizo-mqtt-contract` requires rebuilding the firmware workspace
  separately, and version skew between the two is possible. Mitigated by a path
  dependency (not a version dependency) and by protocol fixture tests shared
  across both workspaces (see [ADR-008](008-shared-code-simulator-and-firmware.md)).
- `no_std` discipline in the contract crate is a real ongoing constraint —
  contributors must resist reaching for `std::collections::HashMap`.

## Risks

- **Contract crate `no_std` regression.** A `std`-only dependency added
  carelessly breaks the firmware build, which is not exercised by the default CI
  job. *Mitigation:* a dedicated CI job runs
  `cargo build -p rhizo-mqtt-contract --no-default-features --target thumbv7em-none-eabi`
  — a cheap `no_std` target that needs no ESP toolchain and catches the
  regression in seconds. Issue M1-011.
- **Domain purity erosion.** Someone adds `Utc::now()` inside the domain for
  convenience and the safety tests quietly stop being deterministic.
  *Mitigation:* a clippy `disallowed-methods` entry in `clippy.toml` banning
  `chrono::Utc::now` and `std::time::SystemTime::now` inside the domain crate.
  Issue M1-013.

## Follow-up

- M0-002 created the workspace skeleton and `[workspace.dependencies]` (done).
- M1-015 adds the `rhizo-policy` crate.
- M13-014 adds the MSRV + current-stable CI matrix.
- M0-003 pins the toolchain.
- M1-011 adds the `no_std` verification job.
- M1-013 adds `clippy.toml` with the disallowed-methods guard.
- [ADR-007](007-esp32-rust-framework-and-toolchain.md) covers the firmware
  workspace toolchain.
- [ADR-008](008-shared-code-simulator-and-firmware.md) covers what exactly is
  shared with the firmware and how skew is prevented.
