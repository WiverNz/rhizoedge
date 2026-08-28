# System Overview

## 1. What Rhizo Edge is

Rhizo Edge is an **offline-first soil monitoring and fail-safe irrigation platform**.

It measures soil conditions, decides whether a plant needs water, and delivers a
bounded dose through a pump — while guaranteeing that a loss of Internet, cloud,
broker, or power never results in unsafe watering.

The first target is indoor houseplants. The architecture is deliberately shaped so
that greenhouse and field deployments are an extension of the same system rather
than a rewrite.

## 2. The governing principle

> A plant must remain safely monitored and controllable locally even when
> Internet/cloud connectivity is unavailable.

Two corollaries drive nearly every design decision in this repository:

1. **The Edge Controller owns irrigation decisions whenever it is reachable.**
   Neither the cloud nor the UI may command hardware directly.
2. **The device is the final safety boundary.** Even a correct-looking command
   from a trusted Edge Controller is re-validated on the ESP32 against limits
   compiled into firmware.
3. **An isolated device is not a useless device.** A plant-side node explicitly
   provisioned with a validated offline policy keeps that plant alive when it
   cannot reach the Edge — from that policy only, never by improvising
   ([ADR-015](../adr/015-device-offline-autonomy.md)).

"Offline" therefore means three distinct things in this system, and they degrade
different capabilities: **cloud offline**, **site offline**, and **device
isolated**. See [connectivity-modes.md](connectivity-modes.md); the bare word is
avoided in new documentation.

## 3. Component map

```text
                         HOUSE / LOCAL NETWORK
 ┌──────────────────────────────────────────────────────────────────┐
 │                                                                  │
 │   ESP32 Plant Node (Rust)          Device Simulator (Rust)       │
 │   ┌────────────────────┐           ┌────────────────────┐        │
 │   │ soil / weight /    │           │ same MQTT protocol │        │
 │   │ tank / leak        │           │ virtual time       │        │
 │   │ pump               │           │ fault injection    │        │
 │   │ HARD SAFETY LIMITS │           │ HARD SAFETY LIMITS │        │
 │   └─────────┬──────────┘           └─────────┬──────────┘        │
 │             │                                │                   │
 │             └──────────── MQTT ──────────────┘                   │
 │                            │                                     │
 │                            ▼                                     │
 │                   ┌─────────────────┐                            │
 │                   │ Mosquitto       │  QoS 1, retained status,   │
 │                   │ local broker    │  Last Will                 │
 │                   └────────┬────────┘                            │
 │                            │                                     │
 │                            ▼                                     │
 │        ┌──────────────────────────────────────────┐              │
 │        │ Edge Controller (Rust)                   │              │
 │        │                                          │              │
 │        │  ingestion → validation → dedup          │              │
 │        │  device registry / health                │              │
 │        │  plant state + recommendations           │              │
 │        │  irrigation state machine + SAFETY       │              │
 │        │  command issue / result handling         │              │
 │        │  local REST API + metrics                │              │
 │        │  cloud outbox                            │              │
 │        └───────┬──────────────────────┬───────────┘              │
 │                │                      │                          │
 │                ▼                      │ HTTP (optional)          │
 │        ┌───────────────┐              │                          │
 │        │ SQLite        │              │                          │
 │        │ source of     │              │                          │
 │        │ truth (local) │              │                          │
 │        └───────────────┘              │                          │
 │                                       │                          │
 │   Rhizo UI (Tauri 2 + Leptos) ────────┘ HTTP to Edge REST API    │
 │   native desktop app, thin client                                │
 └───────────────────────────────────────┼──────────────────────────┘
                                         │
                              OPTIONAL INTERNET
                                         ▼
                            ┌────────────────────────┐
                            │ Cloud API (Rust/Axum)  │  append-only
                            │ idempotent ingest      │  history sink
                            └───────────┬────────────┘
                                        ▼
                            ┌────────────────────────┐
                            │ PostgreSQL             │
                            └────────────────────────┘
```

## 4. Trust and authority

| Concern | Authoritative component | Notes |
|---|---|---|
| Irrigation decision while isolated | Device, from a persisted validated policy | bounded subset only; ADR-015 |
| Device capabilities | Device declares; edge never assumes | ADR-016 |
| Raw measurement truth | Device | Edge may reject, never invent |
| Measurement history | Edge SQLite | Cloud is a replica |
| Plant / irrigation state | Edge | Persisted; survives restart |
| Watering decision | Edge | Cloud and UI have no vote |
| Final actuation veto | Device firmware | Hard limits are compiled in |
| Plant profiles / config | Edge | Cloud does not push config in V1 |
| Long-term history, cross-site view | Cloud | Never required for safety |

The chain of authority is strictly one-directional for actuation:

```text
UI → Edge REST API → Edge domain/state machine → MQTT command → device veto → pump
```

Any path that shortcuts this chain is a defect, not a feature.

## 5. Data flow in one sentence

Devices publish telemetry over MQTT QoS 1; the Edge Controller validates,
transport-deduplicates by `message_id`, durably deduplicates/orders logical
effects by stable identities, and persists them transactionally. M4–M7 extend
that delivered M3 pipeline with registry/plant projections, connected safety and
irrigation evaluation, persisted command publication, and an opportunistic cloud
outbox; none of those future stages is implied to exist in M3.

See [data-flow.md](data-flow.md) for the detailed pipeline.

## 6. Why edge-first rather than cloud-first

A cloud-first design fails the core requirement in three ways:

- **Latency of safety.** A leak must lock the pump in the next control cycle,
  not after a round trip that may not complete.
- **Availability.** Home Internet is not reliable enough to gate a pump that
  can flood a floor.
- **Failure blast radius.** A cloud bug should never be able to over-water a
  plant. Making the cloud an append-only sink means it structurally cannot.

The cost is that the edge must implement persistence, idempotency, and state
recovery itself. That cost is accepted deliberately and is the substance of
milestones M3–M6.

## 7. Development strategy: simulator before hardware

The Device Simulator implements the *same* MQTT protocol as the ESP32 firmware
and is the primary development target for M0–M8. This makes roughly the entire
control plane buildable and testable before any electronics exist.

The hard requirement that follows:

> Replacing the simulator with a real ESP32 changes the device implementation —
> never the MQTT protocol or the Edge Controller architecture.

Milestones M0–M8 must therefore pass with **no hardware attached**.

## 8. Technology summary

| Layer | Choice | ADR |
|---|---|---|
| All software and firmware | Rust **1.98.0** (host); see ADR-007 for firmware | [ADR-001](../adr/001-rust-workspace-and-crate-boundaries.md) |
| Async runtime (host) | Tokio | ADR-001 |
| Broker | Eclipse Mosquitto | [ADR-002](../adr/002-mqtt-topic-versioning-and-qos.md) |
| MQTT client (host) | `rumqttc` | ADR-002 |
| Edge storage | SQLite via `sqlx` | [ADR-004](../adr/004-sqlite-edge-persistence-model.md) |
| Cloud storage | PostgreSQL via `sqlx` | [ADR-005](../adr/005-cloud-event-model-and-idempotency.md) |
| HTTP | Axum | ADR-001 |
| Serialization | `serde` / `serde_json` | ADR-002 |
| Observability | `tracing` + Prometheus text format | [ADR-010](../adr/010-observability-strategy.md) |
| ESP32 | `esp-idf-svc` (std), ESP32-C3 | [ADR-007](../adr/007-esp32-rust-framework-and-toolchain.md) |
| UI | Tauri 2 + Leptos (CSR) + Trunk | [ADR-009](../adr/009-ui-architecture-and-rust-web-stack.md) |

There is **no Go, Node.js, or TypeScript** anywhere in this project. This is a
hard constraint, not a preference.

## 9. Related documents

- [component-model.md](component-model.md) — responsibilities and interfaces per component
- [data-flow.md](data-flow.md) — ingestion and control pipelines
- [deployment-model.md](deployment-model.md) — dev, home, and future topologies
- [safety-invariants.md](safety-invariants.md) — the SAFETY-nnn registry
- [failure-model.md](failure-model.md) — enumerated failures and expected behavior
- [dependency-graph.md](dependency-graph.md) — milestone and issue ordering
- [connectivity-modes.md](connectivity-modes.md) — cloud offline vs site offline vs device isolated
- [offline-autonomy.md](offline-autonomy.md) — the offline policy model and reconciliation
- [time-model.md](time-model.md) — clock semantics (safety-relevant)
- [configuration-model.md](configuration-model.md) — configuration ownership layers
