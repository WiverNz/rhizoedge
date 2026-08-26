# Component Model

This document defines every component, what it owns, what it must never do, and
the interfaces it exposes. It is the contract that keeps the simulator and the
firmware interchangeable.

---

## 1. Crate and component inventory

| Component | Path | Kind | Milestone introduced |
|---|---|---|---|
| `rhizo-mqtt-contract` | `crates/mqtt-contract` | lib (`no_std` + `alloc`) | M1 |
| `rhizo-policy` | `crates/policy` | lib (`no_std` + `alloc`, pure) | M1 |
| `rhizo-domain` | `crates/domain` | lib (std, pure) | M1 |
| `rhizo-storage` | `crates/storage` | lib (sqlx/SQLite) | M3 |
| `rhizo-telemetry` | `crates/telemetry` | lib (tracing/metrics) | M0 |
| `rhizo-cloud-client` | `crates/cloud-client` | lib (HTTP) | M7 |
| `rhizo-testkit` | `crates/testkit` | lib (test support) | M0 |
| `edge-controller` | `crates/edge-controller` | bin | M3 |
| `device-simulator` | `crates/device-simulator` | bin | M2 |
| `cloud-api` | `crates/cloud-api` | bin | M7 |
| `esp32-node` | `firmware/esp32-node` | bin (separate workspace) | M9 |
| `rhizo-ui` | `ui/rhizo-ui` | Tauri app (separate workspace) | M12 |

Rationale for these boundaries is in [ADR-001](../adr/001-rust-workspace-and-crate-boundaries.md).

---

## 2. `rhizo-mqtt-contract` — the shared wire contract

**The single most important crate in the project.** It is the only code shared
between the Edge Controller, the Device Simulator, and the ESP32 firmware.

Owns:

- the message envelope type and its JSON representation
- all telemetry, status, config, command, and command-result payload types
- topic grammar: building and parsing `rhizo/v1/...` topics
- `DeviceId` newtype and its validation grammar
- protocol version constant and compatibility rules
- physical range constants used for validation (`MOISTURE_VWC_RANGE`, etc.)

Constraints:

- `#![no_std]` with `extern crate alloc`; a `std` feature enables `std::error::Error`
  impls and nothing else semantically.
- No `chrono`. Time is carried as `i64` Unix milliseconds (`UtcMillis` newtype).
  `chrono` conversion helpers live behind the `std` feature.
- No I/O, no async, no `tokio`, no `rumqttc`. It describes bytes, not transport.
- No floating point in identifiers or keys; `f32` only inside measurement values.

Must never:

- depend on `rhizo-domain` (the dependency runs the other way)
- contain business rules such as "when to water"

Public surface sketch:

```rust
pub const PROTOCOL_VERSION: u16 = 1;

pub struct DeviceId(/* validated, ≤32 bytes */);
pub struct UtcMillis(pub i64);

pub struct Envelope<T> {
    pub v: u16,
    pub kind: MessageKind,
    pub message_id: Uuid,      // UUIDv7
    pub device_id: DeviceId,
    pub boot_id: Uuid,
    pub sequence: u64,
    pub device_time: Option<UtcMillis>,
    pub clock_synced: bool,
    pub data: T,
}

pub enum Topic {
    TelemetrySoil(DeviceId),
    TelemetryWeight(DeviceId),
    TelemetryTank(DeviceId),
    TelemetryPump(DeviceId),
    Status(DeviceId),
    Config(DeviceId),
    CommandWater(DeviceId),
    CommandTare(DeviceId),
    CommandCalibrate(DeviceId),
    CommandResult(DeviceId),
}

impl Topic {
    pub fn to_string(&self) -> alloc::string::String;
    pub fn parse(topic: &str) -> Result<Topic, TopicError>;
}
```

Full specification: [docs/protocol/mqtt-v1.md](../protocol/mqtt-v1.md).

---

## 2b. `rhizo-policy` — the offline decision subset

The **second** crate shared with the firmware
([ADR-015](../adr/015-device-offline-autonomy.md)). It holds the deliberately
restricted rules an isolated device may evaluate, and nothing else.

Owns:

- `OfflinePolicy`, `OfflineState`, `OfflineInputs`, `OfflineDecision`, `RefuseReason`
- `evaluate_offline(policy, state, inputs, elapsed) -> OfflineDecision`
- the offline safety gate and rolling-budget accounting

Constraints:

- `#![no_std]` + `alloc`; depends only on `rhizo-mqtt-contract`
- **Pure.** No I/O, no allocation beyond `alloc`, and **no clock**: elapsed time
  arrives as a `MonotonicMillis` parameter, which is what makes SAFETY-015
  structural rather than disciplined
- Every absent-able input is `Option` or an explicit tri-state; the gate matches
  exhaustively with no catch-all arm

Must never:

- fit a trend, generate a recommendation, score confidence, detect manual
  watering, reason across plants, author a policy, or compute a dose size
- be called from more than one place per consumer

The Edge links it too, so it can validate a policy before publishing and predict
what an isolated device will do. Full model:
[offline-autonomy.md](offline-autonomy.md).

---

## 3. `rhizo-domain` — pure decision logic

Owns every rule that decides plant state, recommendations, and irrigation
transitions. It is the crate the safety invariants are proved against.

Owns:

- `PlantState`, `IrrigationState`, `LockoutReason` enums
- the irrigation state machine as a pure transition function
- the safety gate (`SafetyEvaluation`) — the single place lockouts are decided
- the rule-based recommendation engine and its explanation structures
- moisture trend and manual-watering-detection algorithms
- plant profile types and their validation
- the `Clock` trait

Hard constraints:

- **No I/O whatsoever.** No database, no network, no filesystem, no `tokio`.
- **No wall-clock access.** All time arrives through `Clock` or as parameters.
  This is what makes the safety tests deterministic.
- Transition functions are `fn(current_state, inputs) -> Decision`, not methods
  that mutate hidden state.

Shape of the central abstraction:

```rust
pub trait Clock: Send + Sync {
    fn now(&self) -> DateTime<Utc>;
}

pub struct IrrigationInputs<'a> {
    pub now: DateTime<Utc>,
    pub state: &'a IrrigationState,
    pub latest_soil: Option<&'a SoilSample>,
    pub tank: Option<TankState>,
    pub leak: LeakState,
    pub profile: &'a PlantProfile,
    pub delivered_today_ml: f32,
    pub last_cycle_completed_at: Option<DateTime<Utc>>,
    pub auto_watering_enabled: bool,
}

pub enum IrrigationDecision {
    Idle,
    Recommend { ml: f32, reasons: Vec<Reason> },
    IssueDose { ml: f32, reasons: Vec<Reason> },
    Wait { until: DateTime<Utc> },
    Lock { reason: LockoutReason },
}

pub fn evaluate(inputs: IrrigationInputs<'_>) -> IrrigationDecision;
```

Because `evaluate` is pure and total, property tests can hammer it with
randomised inputs to prove SAFETY-003 … SAFETY-006 and SAFETY-012.

Must never:

- publish MQTT, write SQLite, or call the cloud
- read the system clock directly
- be bypassed by the API layer when issuing a command

---

## 4. `rhizo-storage` — persistence and transactional boundaries

Owns:

- SQLite schema and embedded migrations for the edge
- repository types (`MeasurementRepo`, `DeviceRepo`, `PlantRepo`,
  `CommandRepo`, `OutboxRepo`, `IrrigationStateRepo`)
- the **deduplicate-and-persist transaction**, the mechanism behind SAFETY-001
  and SAFETY-010
- connection pool setup, WAL mode, busy timeout

Key guarantee it provides:

> Recording that a message was processed and recording that message's effects
> happen in the same SQLite transaction. Either both are durable or neither is.

Must never:

- decide whether to water
- interpret telemetry beyond mapping it to columns

---

## 5. `rhizo-telemetry` — observability wiring

Owns `tracing` subscriber construction, log format selection (JSON or pretty),
the metric registry, and metric name constants. Shared by all three host
binaries so field names stay consistent.

Deliberately small. See [ADR-010](../adr/010-observability-strategy.md).

---

## 6. `device-simulator` — the reference device

A host binary that behaves like an ESP32 plant node and speaks the identical
protocol.

Owns:

- a physical model: soil drying curve, watering absorption, temperature drift,
  EC response, tank depletion, pot weight
- pump execution with calibrated `ml_per_second`
- **the same hard-limit refusal logic the firmware has** — the simulator must
  reject an over-large or expired command exactly as hardware would, otherwise
  safety tests would be testing a more permissive device than reality
- fault injection: disconnects, duplicate publishes, out-of-order sequences,
  invalid values, restarts, clock desync
- virtual time with a configurable acceleration factor

Must never:

- be more permissive than the firmware
- implement irrigation intelligence (that is the Edge Controller's job)

The shared refusal logic lives in `rhizo-mqtt-contract` (as
`validate_water_command`) so both the simulator and the firmware call the same
function. This is the mechanism that makes SAFETY-007 testable before hardware.

---

## 7. `edge-controller` — the control plane

The only component that decides. Structured as cooperating Tokio tasks:

| Task | Responsibility |
|---|---|
| `mqtt_ingress` | rumqttc event loop, decode, hand off to pipeline |
| `pipeline` | validate → dedup → persist → update state → emit events |
| `control_loop` | periodic tick: evaluate irrigation for every plant |
| `command_dispatch` | publish water commands, track in-flight, apply TTL |
| `api` | Axum REST server |
| `outbox_drain` | ship cloud events with backoff; never blocks control |
| `retention` | prune `processed_messages` and old measurements |

Owns:

- MQTT subscription and connection lifecycle
- the ingestion pipeline
- device registry and health
- plant state
- irrigation execution and command lifecycle
- local REST API and metrics endpoint
- the cloud outbox

Must never:

- require the cloud for any decision
- allow the API layer to publish an MQTT command without passing the domain
  safety gate
- lose safety state across restart (all of it is in SQLite)

Interfaces exposed: see [docs/protocol/http-api-boundaries.md](../protocol/http-api-boundaries.md).

---

## 8. `cloud-api` + PostgreSQL — append-only history

Owns:

- idempotent event ingestion keyed by `(edge_id, event_id)`
- historical storage of measurements, watering events, device events
- read APIs for multi-site history
- a data model that supports many edge instances from day one

Must never:

- issue commands to devices
- be a dependency of any safety decision
- reject an event in a way that requires human intervention to drain the outbox
  (poison events are quarantined, not retried forever at full rate)

---

## 9. `rhizo-ui` — thin desktop client

Tauri 2 shell + Leptos CSR frontend. Talks HTTP to the Edge Controller.

Owns: presentation, charts, and user intent capture.

Must never:

- connect to MQTT
- re-implement irrigation, recommendation, or safety logic
- hold authoritative state

The Tauri Rust side stays thin: window management, the edge base URL, and
optionally a local secret store. Business logic that leaks into the UI is a
defect against [ADR-009](../adr/009-ui-architecture-and-rust-web-stack.md).

---

## 10. `esp32-node` — firmware and final safety boundary

Lives in its own Cargo workspace because it uses a different target, toolchain,
and `std` implementation ([ADR-007](../adr/007-esp32-rust-framework-and-toolchain.md)).

Owns:

- sensor acquisition through a `SoilSensor` / `TankSensor` / `LeakSensor` /
  `Scale` trait set
- pump actuation through a `Pump` trait
- Wi-Fi, MQTT, NVS; wall time synchronised from the Edge over MQTT (no SNTP client)
- **hard safety limits compiled into the binary and not remotely configurable**
- command TTL validation, command deduplication across reboot
- boot-safe state: pump off before anything else initialises

Shares `rhizo-mqtt-contract` with `default-features = false`.

Must never:

- accept a dose larger than `FIRMWARE_MAX_ML_PER_RUN`
- accept any water command while its wall clock is unsynced (it cannot evaluate
  TTL, so it must refuse — see [time-model.md](time-model.md))
- keep the pump energised across a watchdog reset

---

## 11. Dependency direction

```text
mqtt-contract ◄── policy ◄── domain ◄── storage ◄── edge-controller
      ▲                          ▲              │
      │                          └──────────────┤
      ├── device-simulator                      ├── telemetry
      ├── esp32-node (no_std)                   ├── cloud-client ──► cloud-api
      └── testkit ─────────────────────────────►┘
```

Rules enforced by review and by `rhizo-docscheck` where mechanical:

1. `mqtt-contract` depends on nothing in this workspace.
2. `policy` depends only on `mqtt-contract`, is `no_std`, and reads no clock.
3. `domain` depends on `mqtt-contract` and `policy`.
3. `storage` depends on `domain` and `mqtt-contract`.
4. Binaries depend on libraries; libraries never depend on binaries.
5. Nothing depends on `edge-controller` except integration tests.
