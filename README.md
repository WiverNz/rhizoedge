# Rhizo Edge

An **offline-first Rust platform for soil monitoring and fail-safe automated
irrigation**, using MQTT, local edge processing, ESP32 devices, and optional
cloud synchronisation.

> **Status: planning complete, implementation not started.**
> This repository currently contains the engineering plan — architecture,
> decision records, requirements, protocol specification, and 204 implementation
> issues. No production code has been written yet.
> Start at [ROADMAP.md](ROADMAP.md); the first issue to execute is
> [M0-001](docs/issues/M0/001-create-repository-skeleton.md).

---

## What it is

Rhizo Edge measures soil conditions, decides whether a plant needs water, and
delivers a bounded dose through a pump — while guaranteeing that a loss of
Internet, cloud, broker, or power never results in unsafe watering.

The first target is indoor houseplants. The architecture is deliberately shaped
so greenhouse and field deployments are an extension of the same system rather
than a rewrite.

## Two principles

**Edge-first.** The Edge Controller owns all irrigation decisions and all local
state. The cloud is an append-only history sink that is optional by default
(`cloud.enabled = false`) and can vanish for a week without affecting a single
watering decision. This is enforced structurally, not by discipline: the domain
crate cannot depend on the cloud client, and the type carrying every watering
input has no cloud-derived field.

**Safety-first.** When any input is missing, stale, invalid, or contradictory,
the answer is *do not water*, plus a visible lockout — never *water anyway*.
Twelve numbered invariants ([SAFETY-001…012](docs/architecture/safety-invariants.md))
state this precisely, each with named automated tests and the milestone where it
becomes enforced. `cargo test safety_` is the project's definition of "are the
invariants still enforced?".

Defence in depth: the edge decides *whether* and *how much*; the device decides
whether to **obey**. Limits compiled into firmware cannot be raised by any
message, API call, or configuration — so even a completely wrong Edge Controller
cannot flood the room.

## Architecture

```text
 ESP32 Plant Node (Rust)        Device Simulator (Rust)
   sensors · pump                 same MQTT protocol
   HARD SAFETY LIMITS             HARD SAFETY LIMITS
        └──────────── MQTT (QoS 1) ────────────┘
                        │
                   Mosquitto
                        │
              ┌─────────▼──────────────────────┐
              │ Edge Controller (Rust)         │
              │  ingest → validate → dedup     │
              │  plant state · recommendations │
              │  irrigation state machine      │
              │  SAFETY GATE                   │
              │  local REST API · metrics      │
              └───┬──────────────────┬─────────┘
                  │                  │ HTTP (optional, outbound only)
             ┌────▼─────┐      ┌─────▼──────────────┐
             │ SQLite   │      │ Cloud API (Rust)   │
             │ source   │      │ append-only        │
             │ of truth │      │ → PostgreSQL       │
             └──────────┘      └────────────────────┘
                  ▲
                  │ HTTP
        Rhizo UI (Tauri 2 + Leptos desktop app)
```

Details: [system overview](docs/architecture/system-overview.md) ·
[component model](docs/architecture/component-model.md) ·
[data flow](docs/architecture/data-flow.md)

## Components

| Component | Role |
|---|---|
| `rhizo-mqtt-contract` | `no_std` wire contract shared with the firmware — the **only** shared crate |
| `rhizo-domain` | Pure decision logic: plant state, irrigation machine, safety gate. No I/O, no clock access |
| `rhizo-storage` | SQLite schema, repositories, and the deduplicate-and-persist transaction |
| `edge-controller` | The control plane — the only component that decides |
| `device-simulator` | Reference device; same protocol, virtual time, fault injection |
| `cloud-api` | Idempotent event ingestion into PostgreSQL |
| `esp32-node` | ESP32-C3 firmware; the final hardware safety boundary |
| `rhizo-ui` | Tauri 2 + Leptos desktop client; talks HTTP to the edge only |

## Development strategy: simulator before hardware

The Device Simulator implements the *same* MQTT protocol as the firmware and
calls the *same* command validator, so it can never be more permissive than real
hardware. That makes roughly the entire control plane buildable and testable
before any electronics exist.

**Milestones M0–M8 require no hardware at all.** M8 delivers the complete
software system, verified end to end by one command:

```bash
docker compose -f deploy/docker-compose.yml -f deploy/docker-compose.test.yml \
  up --abort-on-container-exit --exit-code-from scenario-runner
```

The hard requirement that follows: replacing the simulator with a real ESP32
changes the *device implementation* — never the MQTT protocol or the Edge
Controller architecture.

## Implementation constraints

**Rust only.** Firmware, simulator, edge, cloud, and UI. There is no Go, no
Node.js, and no TypeScript anywhere in this project, and the UI's Tauri workflow
is Cargo-based specifically to keep it that way.

**Rust 1.98.0** is the pinned host toolchain (`rust-toolchain.toml`), covering
`crates/*` and the UI. The firmware workspace is separate and may pin an
ESP-compatible toolchain if the Espressif ecosystem requires it; that exception
is isolated and documented in
[ADR-007](docs/adr/007-esp32-rust-framework-and-toolchain.md). The host
workspace is never downgraded to match an embedded constraint.

| Layer | Choice |
|---|---|
| Async runtime | Tokio |
| Broker | Eclipse Mosquitto, MQTT 3.1.1, QoS 1, `clean_session = true` |
| Edge storage | SQLite via `sqlx`, WAL |
| Cloud storage | PostgreSQL via `sqlx` |
| HTTP | Axum |
| Observability | `tracing` + Prometheus text format |
| ESP32 | ESP32-C3, `esp-idf-svc` (std) |
| UI | Tauri 2 + Leptos (CSR) + Trunk |

## Development flow

```text
read the issue  →  implement  →  fmt  →  clippy -D warnings  →  test
                →  run the issue's Verification commands
                →  update docs if behaviour changed  →  tick acceptance criteria
```

Project-wide gate:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo build -p rhizo-mqtt-contract --no-default-features --target thumbv7em-none-eabi
docker compose -f deploy/docker-compose.yml config
cargo run --manifest-path tools/docscheck/Cargo.toml
```

A milestone is complete only when its acceptance tests are green and its exit
criteria are demonstrably met — never on the basis of closed issues alone.

## Documentation

Start here: **[docs/README.md](docs/README.md)** — the documentation index.

| | |
|---|---|
| [ROADMAP.md](ROADMAP.md) | Milestones, exit criteria, conventions |
| [Safety invariants](docs/architecture/safety-invariants.md) | SAFETY-001…012 |
| [MQTT protocol v1](docs/protocol/mqtt-v1.md) | Normative wire specification |
| [Dependency graph](docs/architecture/dependency-graph.md) | What to implement next |
| [Failure model](docs/architecture/failure-model.md) | Every failure and its expected behaviour |
| [Testing strategy](docs/testing/strategy.md) | How the safety claims are proven |
| [Local development](docs/testing/local-development.md) | Running and debugging |

## Known V1 limitations

Stated plainly, because they are decisions rather than oversights:

- **No authentication on the Edge REST API.** The network boundary is the
  security boundary. Anyone on the LAN who reaches port 8080 can water a plant.
  This is the first thing to change for any non-trusted network.
- **No TLS on MQTT**, no per-device certificates, no signed firmware.
- **No remote access.** The UI is a desktop app on the LAN.
- **No machine learning**, and **no N/P/K inference from EC** — cheap NPK probes
  derive their output from EC by an undisclosed formula, and presenting that as
  a nutrient measurement would be a false claim.
- **Cloud pushes no configuration.** Deliberate; it needs an authentication
  story V1 does not have.

## Licence

Not yet selected.
