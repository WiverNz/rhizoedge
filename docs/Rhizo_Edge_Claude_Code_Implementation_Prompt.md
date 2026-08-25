# Claude Code Implementation Prompt — Rhizo Edge

You are implementing **Rhizo Edge**, an edge-first plant monitoring and automated irrigation platform.

The repository contains (or will contain) the project specification:

```text
Rhizo_Edge_PROJECT_PLAN.md
```

If the existing file still has the old project name `OpenSoil Edge`, treat it as the same project and rename all project-facing references to:

```text
Rhizo Edge
```

The goal of this prompt is **not** to produce a prototype in one giant change.

You must first turn the project plan into a proper engineering roadmap with:

- architecture documentation
- ADRs
- PRDs
- milestone definitions
- issue files
- acceptance criteria
- test plans
- implementation order
- explicit dependency graph

Then implement the project **milestone by milestone**.

Do not skip planning artifacts.

Do not create placeholder planning documents that merely repeat this prompt. They must contain concrete technical decisions, interfaces, acceptance criteria, dependencies, and verification steps.

---

# 1. Product Goal

Rhizo Edge is an **offline-first Rust platform for soil monitoring and fail-safe automated irrigation**.

The first useful target is indoor plants, but the architecture must support later evolution toward:

- multiple plants
- greenhouses
- farms
- multi-depth soil probes
- RS485 / Modbus
- LoRaWAN / LTE-M / NB-IoT
- weather inputs
- irrigation zones
- agricultural edge gateways

The first complete version should work entirely on a developer machine using Docker and simulators.

Later, the same ESP32 Rust application must be buildable for a real ESP32 board and replace the simulated device without redesigning the system.

---

# 2. Hard Technical Constraints

These constraints are mandatory unless an ADR explicitly proves that a change is necessary.

## 2.1 Language

**Rust everywhere.**

Use Rust for:

- ESP32 firmware
- ESP32 simulator
- MQTT contracts/types
- edge controller
- cloud API
- cloud workers if needed
- UI backend
- UI application where practical

For the UI, prefer a Rust-native stack.

Preferred option:

```text
Leptos
```

with SSR/hydration if useful.

Acceptable fallback:

```text
Axum + Askama/HTMX
```

if it materially reduces complexity.

Do not introduce Node.js/TypeScript merely for convenience.

## 2.2 Async runtime

For non-embedded services:

```text
Tokio
```

## 2.3 MQTT

Use:

```text
Eclipse Mosquitto
```

for the local broker.

Rust MQTT clients should use an actively maintained crate such as:

```text
rumqttc
```

unless repository investigation shows a better-supported choice.

## 2.4 Storage

### Edge/local storage

Use:

```text
SQLite
```

via Rust.

Preferred:

```text
sqlx
```

### Cloud storage

Use:

```text
PostgreSQL
```

running in Docker.

Cloud persistence must remain optional for local plant operation.

## 2.5 HTTP

Use:

```text
Axum
```

for Rust HTTP APIs unless there is a strong reason not to.

## 2.6 Serialization

Use:

```text
serde
serde_json
```

MQTT payload contracts must be versioned.

## 2.7 Observability

Use:

```text
tracing
tracing-subscriber
```

Expose Prometheus-compatible metrics where useful.

## 2.8 Testing

The repository must support:

```bash
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```

Where embedded targets make workspace-wide commands impossible, structure the workspace so host tests and firmware build verification are clearly separated and documented.

No milestone is complete while its acceptance tests are red.

---

# 3. Required Repository Architecture

Design the repository as a Rust workspace with explicit subprojects.

Target shape:

```text
rhizo-edge/
├── Cargo.toml
├── Cargo.lock
├── rust-toolchain.toml
├── README.md
├── Rhizo_Edge_PROJECT_PLAN.md
│
├── crates/
│   ├── domain/
│   ├── mqtt-contract/
│   ├── edge-controller/
│   ├── device-simulator/
│   ├── cloud-api/
│   ├── cloud-client/
│   ├── storage/
│   ├── telemetry/
│   └── testkit/
│
├── firmware/
│   └── esp32-node/
│
├── ui/
│   └── rhizo-ui/
│
├── migrations/
│   ├── edge/
│   └── cloud/
│
├── deploy/
│   ├── docker-compose.yml
│   ├── docker-compose.test.yml
│   ├── mosquitto/
│   ├── cloud/
│   ├── edge/
│   └── ui/
│
├── docs/
│   ├── README.md
│   ├── architecture/
│   ├── adr/
│   ├── prd/
│   ├── protocol/
│   ├── testing/
│   └── issues/
│
├── scripts/
│
└── test/
    ├── fixtures/
    └── scenarios/
```

You may refine this structure, but changes must be documented.

---

# 4. Mandatory Subsystems

The architecture must explicitly separate these subsystems.

## A. ESP32 Node

Rust embedded application responsible for:

- sensor acquisition
- MQTT connectivity
- device identity
- telemetry publication
- retained status
- Last Will configuration
- retained configuration consumption
- command consumption
- pump control
- hard local safety limits
- command deduplication
- command expiration checks
- watchdog/recovery behavior
- local hardware safety state

Initial hardware abstraction must support running the same domain/device logic against:

1. a host-side simulator
2. real ESP32 peripherals later

The embedded implementation must not block initial development.

## B. Device Simulator

Host-side Rust process that behaves like an ESP32 plant node.

It must simulate:

- soil moisture
- soil temperature
- EC
- tank level
- leak sensor
- pot weight
- pump activation
- soil drying
- watering response
- connectivity failures
- restarts
- duplicate messages
- out-of-order telemetry where appropriate
- invalid sensor values

It must implement the **same MQTT protocol** as the real ESP32 node.

This simulator is critical.

Most end-to-end functionality must be testable before hardware exists.

## C. Edge Controller

Rust service running near the devices.

Responsibilities:

- consume MQTT telemetry
- validate schemas
- deduplicate messages
- persist local measurements in SQLite
- maintain device registry
- maintain plant state
- detect device health
- detect manual watering
- compute watering recommendations
- execute irrigation state machines
- publish watering commands
- process command results
- enforce edge-level safety
- expose local REST API
- expose metrics
- queue cloud events while offline
- synchronize to cloud when available

This is the **main control plane for local plant operation**.

Cloud loss must not disable safe local operation.

## D. Cloud API + Cloud Storage

Containerized Rust service plus PostgreSQL.

Initial cloud functionality should be deliberately limited.

Responsibilities:

- accept idempotent event synchronization from edge controllers
- persist device/plant/measurement/event history
- expose read APIs
- optionally expose config desired state later
- support multiple edge-controller IDs in the data model
- maintain event idempotency
- tolerate retry/replay

Cloud is **not** the source of truth required for immediate watering safety.

## E. UI

A separate Dockerized Rust UI service added in a later milestone.

It should support:

- device list
- device online/offline state
- plant list
- latest measurements
- measurement history
- current recommendation
- watering history
- current plant state
- manual watering command
- automatic watering enable/disable
- safety lockout visibility
- pending cloud sync count
- system health

The UI should initially talk to the **edge API** for local operation.

Later it may support the cloud API as an alternate source.

---

# 5. Local Development Topology

The repository must provide a one-command test/development deployment.

Target:

```bash
docker compose up --build
```

or an equivalent documented command.

Initial topology:

```text
┌───────────────────────────────────────────────┐
│ Docker Compose                               │
│                                               │
│ device-simulator                             │
│       │                                       │
│       │ MQTT                                  │
│       ▼                                       │
│ mosquitto                                     │
│       │                                       │
│       ▼                                       │
│ edge-controller ──► SQLite                    │
│       │                                       │
│       │ HTTP sync                             │
│       ▼                                       │
│ cloud-api ─────────► PostgreSQL               │
│                                               │
│ later:                                        │
│ rhizo-ui ──────────► edge-controller          │
└───────────────────────────────────────────────┘
```

Everything must work without ESP32 hardware.

---

# 6. ESP32 Development Strategy

Do not make real hardware a prerequisite for early milestones.

Use this order:

```text
shared contracts/domain
        ↓
device simulator
        ↓
edge controller
        ↓
cloud
        ↓
system tests
        ↓
embedded abstraction
        ↓
ESP32 Rust firmware
        ↓
real hardware
```

The firmware should share protocol/domain crates where embedded constraints allow it.

If standard `std` crates cannot be shared directly, split protocol/domain representations into embedded-compatible crates or features.

Use feature flags consciously.

Document the decision in an ADR.

---

# 7. ESP32 Rust Requirements

Investigate the currently recommended Rust ESP32 stack before implementing firmware.

Prefer official/currently supported Espressif Rust tooling.

Candidate approaches include:

```text
esp-idf-svc
esp-idf-hal
embedded-svc
```

or the appropriate current equivalents.

The firmware milestone must document:

- supported ESP32 board/chip
- toolchain installation
- target triple
- flashing command
- serial monitor command
- MQTT/Wi-Fi configuration
- environment/config provisioning
- how to build without having the board connected
- how to flash when the board is available

Do not fabricate toolchain commands. Verify them against the selected Rust ESP32 ecosystem.

---

# 8. Protocol Design Requirements

Create a dedicated protocol document:

```text
docs/protocol/mqtt-v1.md
```

The protocol must include:

- topic hierarchy
- payload schemas
- QoS
- retained-message rules
- Last Will behavior
- sequence semantics
- message IDs
- command IDs
- command TTL
- device status
- config versioning
- schema/protocol versioning
- error/result semantics
- forward/backward compatibility policy

Base namespace:

```text
rhizo/v1/
```

Suggested topics:

```text
rhizo/v1/devices/{device_id}/telemetry/soil
rhizo/v1/devices/{device_id}/telemetry/weight
rhizo/v1/devices/{device_id}/telemetry/tank
rhizo/v1/devices/{device_id}/telemetry/pump

rhizo/v1/devices/{device_id}/status
rhizo/v1/devices/{device_id}/config

rhizo/v1/devices/{device_id}/commands/water
rhizo/v1/devices/{device_id}/commands/tare
rhizo/v1/devices/{device_id}/commands/calibrate

rhizo/v1/devices/{device_id}/commands/result
```

Use QoS 1 for important messages.

Consumers must be idempotent.

---

# 9. Safety Invariants

Create a dedicated invariant registry, for example:

```text
docs/architecture/safety-invariants.md
```

and corresponding tests.

At minimum:

## SAFETY-001

A duplicate watering command must never cause duplicate physical watering.

## SAFETY-002

An expired watering command must never execute.

## SAFETY-003

Automatic watering must never run when the leak sensor indicates water.

## SAFETY-004

Automatic watering must never run when the tank is below the configured minimum.

## SAFETY-005

Automatic watering must never run when the moisture measurement is stale or invalid.

## SAFETY-006

Daily automatic watering must never exceed the configured maximum.

## SAFETY-007

A single command must never exceed the ESP32 hard maximum dose/run duration.

## SAFETY-008

Cloud unavailability must not disable local monitoring.

## SAFETY-009

Cloud unavailability must not bypass local watering safety.

## SAFETY-010

Restarting the edge controller must not cause previously completed commands to execute again.

## SAFETY-011

Restarting the ESP32 during command processing must converge to a safe state.

## SAFETY-012

When uncertain, watering defaults to disabled rather than enabled.

Every invariant must map to one or more automated tests when technically possible.

---

# 10. Architectural Decisions to Document

Create ADRs before implementing affected milestones.

At minimum:

```text
ADR-001 Rust workspace and crate boundaries
ADR-002 MQTT topic/versioning/QoS strategy
ADR-003 Edge-first ownership and cloud consistency model
ADR-004 SQLite edge persistence model
ADR-005 PostgreSQL cloud event model and idempotency
ADR-006 Irrigation state machine and safety ownership
ADR-007 ESP32 Rust framework/toolchain
ADR-008 Shared code between simulator and ESP32
ADR-009 UI architecture and Rust web stack
ADR-010 Observability strategy
ADR-011 Configuration and secrets model
ADR-012 Device identity/provisioning model
```

Each ADR should contain:

- Context
- Decision
- Alternatives considered
- Consequences
- Risks
- Follow-up work

---

# 11. PRD Structure

Create PRDs under:

```text
docs/prd/
```

Use numbered files.

Recommended set:

```text
000-platform-foundation.md
010-domain-and-protocol.md
020-device-simulator.md
030-edge-ingestion-and-storage.md
040-device-registry-and-health.md
050-irrigation-recommendations.md
060-irrigation-control-and-safety.md
070-cloud-sync-and-storage.md
080-esp32-firmware-foundation.md
090-real-sensor-integration.md
100-real-pump-control.md
110-ui-and-operations.md
120-multi-plant-home.md
130-field-readiness.md
```

Each PRD must contain:

1. Problem
2. Goals
3. Non-goals
4. User/system flows
5. Functional requirements
6. Interfaces
7. Data model
8. Failure modes
9. Safety implications
10. Observability
11. Testing strategy
12. Acceptance criteria
13. Dependencies
14. Open questions
15. Out-of-scope future work

---

# 12. Issue Files

Do not rely only on GitHub/GitLab issues.

Create repository-tracked issue files:

```text
docs/issues/M0/
docs/issues/M1/
...
```

Each issue file must be small enough for one focused implementation change.

Naming example:

```text
docs/issues/M2/001-add-soil-telemetry-schema.md
docs/issues/M2/002-add-mqtt-topic-parser.md
docs/issues/M2/003-add-qos1-deduplication.md
```

Each issue must contain:

```text
# Title

## Context

## Scope

## Non-goals

## Dependencies

## Implementation notes

## Acceptance criteria

## Verification

## Files likely affected
```

Do not make giant issue files such as "implement edge controller".

Break work down into reviewable steps.

---

# 13. Milestones

Build a roadmap with explicit dependencies.

Use these milestones unless repository investigation gives a compelling reason to refine them.

## M0 — Foundation and Engineering Baseline

Goal: create a clean Rust repository with tooling and documentation structure.

Deliver:

- workspace
- pinned Rust toolchain
- formatting/lint/test configuration
- Docker Compose skeleton
- documentation index
- architecture overview
- ADR framework
- PRD framework
- issue structure
- CI
- basic testkit

Acceptance:

```bash
cargo fmt --check
cargo clippy ...
cargo test ...
docker compose config
```

all succeed.

No business behavior required yet.

## M1 — Domain Model and MQTT Protocol

Goal: define stable shared contracts.

Deliver:

- typed device IDs
- plant IDs
- measurement types
- device state
- command types
- command result types
- MQTT topics
- MQTT payload schemas
- protocol versioning
- QoS rules
- retained rules
- Last Will contract
- command expiration
- deduplication contract

Acceptance:

- unit tests for encoding/decoding
- invalid payload tests
- topic parser tests
- compatibility fixtures

## M2 — Device Simulator

Goal: create a realistic host-side simulated ESP32 node.

Deliver:

- simulator connects to Mosquitto
- publishes status
- publishes soil telemetry
- supports moisture/temp/EC
- accepts config
- accepts water commands
- simulates pump effect
- simulates drying
- publishes command results
- configurable failures
- duplicate-message mode
- reconnect mode

Acceptance:

```text
simulator → Mosquitto
```

can run independently in Docker.

## M3 — Edge Ingestion and SQLite

Goal: consume device telemetry reliably and persist locally.

Deliver:

- MQTT consumer
- schema validation
- deduplication
- SQLite migrations
- measurements table
- device event table
- structured logs
- basic metrics
- graceful shutdown/restart

Acceptance:

1. run simulator
2. ingest telemetry
3. restart edge-controller
4. history remains
5. duplicate MQTT messages do not duplicate logical records

## M4 — Device Registry and Health

Goal: track real device lifecycle.

Deliver:

- online/offline state
- retained status
- Last Will support
- last-seen
- firmware version
- stale-device detection
- sensor-health state
- local API

Acceptance:

- simulated disconnect moves device offline
- reconnect restores online state
- stale telemetry is visible

## M5 — Plant Model and Recommendation Engine

Goal: move from raw telemetry to useful plant behavior.

Deliver:

- plant profiles
- target moisture ranges
- EC thresholds
- measurement trends
- time-since-last-watering
- recommendation state
- explainable recommendation reasons
- manual watering detection logic where possible

Acceptance:

Simulator drying must eventually generate:

```text
WATER_RECOMMENDED
```

with a reason.

## M6 — Irrigation State Machine and Safety

Goal: implement automated control safely, still using simulator only.

Deliver:

- manual watering
- recommended watering
- automatic watering
- explicit state machine
- multi-dose watering
- absorption wait
- recheck
- cooldown
- max single dose
- max daily dose
- tank lockout
- leak lockout
- sensor-fault lockout
- stale-data lockout
- command TTL
- command deduplication
- safety invariant tests

Acceptance:

All safety invariants must be green.

Run scenario:

```text
dry soil
→ recommendation
→ dose
→ wait
→ recheck
→ second dose if needed
→ recovered
```

## M7 — Cloud API and PostgreSQL

Goal: add optional cloud history without weakening edge independence.

Deliver:

- Rust cloud API
- PostgreSQL migrations
- event ingestion API
- idempotency key
- device/plant/measurement/event storage
- cloud client
- pending event table/queue on edge
- exponential backoff
- jitter
- replay after outage

Acceptance:

1. stop cloud
2. edge keeps operating
3. events become pending
4. restart cloud
5. events synchronize
6. replay does not duplicate logical events

Cloud runs in Docker.

## M8 — End-to-End Test Environment

Goal: make the whole software system reproducible without hardware.

Deliver:

```text
docker compose up --build
```

starts:

- Mosquitto
- device simulator
- edge controller
- cloud API
- PostgreSQL

Add automated scenario tests.

Required scenarios:

- normal telemetry
- dry soil
- watering cycle
- duplicate command
- duplicate telemetry
- broker restart
- edge restart
- cloud outage/recovery
- tank empty
- leak detected
- stale sensor
- invalid sensor value

Acceptance:

One command can execute the end-to-end test suite.

## M9 — ESP32 Rust Firmware Foundation

Goal: replace the simulator protocol endpoint with real Rust firmware.

Do not yet require real soil hardware.

Deliver:

- ESP32 Rust project
- Wi-Fi
- MQTT
- device identity
- Last Will
- retained status
- config consumption
- command consumption
- command result publication
- fake/in-memory sensor adapter
- fake pump adapter
- build instructions
- flash instructions

The ESP32 firmware must speak the same MQTT protocol as the simulator.

Acceptance:

- firmware compiles for selected ESP32 target
- host tests cover shared logic
- if board is available: connect to local Mosquitto and appear as a device

## M10 — Real Soil Sensor Integration

Goal: read a real soil sensor from ESP32.

Preferred long-term path:

```text
RS485 / Modbus RTU
```

Initial hardware may use a simpler sensor if needed.

Deliver:

- sensor trait/interface
- fake sensor
- real sensor adapter
- calibration/config
- invalid-read handling
- telemetry integration

Acceptance:

Real readings flow:

```text
sensor
→ ESP32 Rust
→ MQTT
→ edge
→ SQLite
→ cloud
```

## M11 — Real Pump and Safety Hardware

Goal: control real irrigation hardware safely.

Deliver:

- pump driver abstraction
- pump calibration
- real pump adapter
- tank level integration
- leak sensor integration
- ESP32 hard safety limits
- watchdog behavior
- boot-safe pump state
- restart-during-watering behavior

Acceptance:

A requested test dose approximately matches expected milliliters and all local lockouts work.

## M12 — Rust UI

Goal: provide a local operations interface.

Add a dedicated Docker container:

```text
rhizo-ui
```

Deliver:

- device page
- plant page
- online/offline
- latest telemetry
- charts
- recommendation
- irrigation state
- watering history
- manual watering
- auto-watering toggle
- safety lockouts
- edge/cloud health
- pending cloud sync

UI must be implemented in Rust.

Preferred:

```text
Leptos
```

Acceptance:

Running Docker Compose exposes a usable browser UI.

## M13 — Multi-Plant Home System

Goal: support multiple ESP32 plant nodes.

Deliver:

- onboarding
- unique device credentials
- multiple plant profiles
- per-plant state
- multiple pumps
- central dashboard
- multi-device failure tests

## M14 — Field Readiness Architecture

Goal: do not fully build the farm version yet.

Prepare architecture and interfaces for:

- multi-depth soil sensors
- RS485 buses
- LoRaWAN
- LTE-M
- NB-IoT
- field gateways
- irrigation zones
- weather inputs
- power constraints
- solar/battery devices

Deliver PRDs/ADRs and small abstractions only where justified.

Avoid premature implementation.

---

# 14. Main Architecture Principles

## 14.1 Edge-first

The edge controller owns local plant operation.

Cloud is optional.

## 14.2 Safe by default

Any uncertain state means:

```text
DO NOT WATER
```

not:

```text
TRY WATERING
```

## 14.3 Desired state and explicit transitions

Device configuration and irrigation behavior should use explicit state.

Avoid hidden side effects.

## 14.4 Idempotent messaging

MQTT QoS 1 and HTTP retries imply duplicates.

Design every command/event flow accordingly.

## 14.5 Simulation before hardware

Any behavior that can be tested with the simulator should be implemented and tested before requiring ESP32 hardware.

## 14.6 Shared behavior

Simulator and ESP32 must conform to the same contracts.

Avoid separate ad-hoc implementations.

## 14.7 Observable state

Important behavior must be visible through:

- structured logs
- metrics
- API state
- persisted events

---

# 15. Data Model

Define concrete schemas in PRDs before implementation.

At minimum:

## Edge SQLite

```text
devices
plants
measurements
watering_events
device_events
commands
command_results
pending_cloud_events
plant_profiles
```

## Cloud PostgreSQL

At minimum:

```text
edge_instances
devices
plants
measurements
watering_events
device_events
synced_events
```

Cloud schema must preserve event IDs for idempotency.

---

# 16. Testkit Requirements

Create:

```text
crates/testkit
```

or an equivalent reusable testing package.

It should support:

- deterministic clock where useful
- fake MQTT payload generation
- simulated devices
- fault injection
- scenario fixtures
- test plant profiles
- command assertions
- safety-invariant assertions

Prefer deterministic tests over sleeping for long real durations.

The simulator must support accelerated virtual time for drying/watering scenarios.

---

# 17. Scenario DSL

If useful, implement a small test scenario format.

Example:

```yaml
name: cloud outage during watering

steps:
  - device_online: plant-node-01
  - set_moisture: 20
  - wait_until_state: WATER_RECOMMENDED
  - disable_cloud: true
  - enable_auto_watering: true
  - expect_command:
      type: water
      max_ml: 50
  - expect_edge_operational: true
  - enable_cloud: true
  - expect_pending_events: 0
```

Do not overengineer the DSL early.

Only add it when repeated integration tests justify it.

---

# 18. CI Requirements

Create CI that verifies host-side code on every change.

Minimum:

```bash
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
docker compose config
```

Add:

- integration test job
- Docker build verification
- ESP32 firmware compile verification when practical

Keep embedded build in a separate job if its toolchain is expensive.

---

# 19. Documentation Requirements

Create:

```text
docs/README.md
```

as the documentation index.

Maintain:

```text
README.md
ROADMAP.md
docs/architecture/system-overview.md
docs/architecture/safety-invariants.md
docs/protocol/mqtt-v1.md
docs/testing/strategy.md
docs/testing/local-development.md
docs/adr/
docs/prd/
docs/issues/
```

The roadmap must show:

- milestones
- dependencies
- status
- acceptance criteria summary

---

# 20. Verification Discipline

For every issue:

1. implement
2. format
3. lint
4. test affected crates
5. run relevant integration tests
6. update docs if behavior changed
7. mark issue acceptance criteria

Do not declare a milestone complete without evidence.

At the end of every milestone, report:

```text
Files added
Files changed
Tests added
Commands run
Results
Known limitations
Next milestone
```

---

# 21. First Task — Planning Pass

Before writing production code:

1. Inspect the repository.
2. Read `Rhizo_Edge_PROJECT_PLAN.md`.
3. If the file still says `OpenSoil Edge`, rename project-facing references to `Rhizo Edge`.
4. Create the proposed architecture.
5. Identify uncertainties.
6. Resolve reasonable defaults yourself.
7. Create ADRs required for M0–M3.
8. Create PRDs for at least M0–M3.
9. Create `ROADMAP.md`.
10. Create issue files for M0–M3.
11. Create a dependency graph.
12. Create the invariant registry.
13. Validate that the plan permits full simulator-based development before hardware.
14. Only then begin M0 implementation.

Do **not** stop after planning unless blocked by a genuinely external requirement.

Continue into implementation.

---

# 22. Implementation Behavior

Work autonomously.

Do not ask questions for choices that can be safely decided from this specification.

Prefer small, reviewable changes.

Do not implement future milestones prematurely.

Do not add hardware-specific complexity to the edge controller.

Do not make cloud availability a dependency of local plant safety.

Do not introduce unnecessary frameworks.

Do not hide errors.

Do not use `unwrap()` / `expect()` in long-running production paths unless the invariant is genuinely impossible to violate and is documented.

Use typed errors.

Use structured concurrency and graceful cancellation.

---

# 23. Definition of the First Major Demo

The first major software-only demo is reached after M8.

It must run without real ESP32 hardware.

A user should be able to execute the documented local command and observe:

```text
1. device simulator connects
2. device becomes online
3. soil telemetry appears
4. moisture decreases over accelerated time
5. edge detects dry soil
6. recommendation is generated
7. automatic mode sends a bounded watering command
8. simulator applies watering
9. edge waits for absorption
10. edge rechecks
11. another dose is sent only if needed
12. plant returns to healthy state
13. cloud can be stopped
14. edge continues operating
15. events queue locally
16. cloud restarts
17. events synchronize idempotently
```

This must be backed by automated tests, not just manual output.

---

# 24. Definition of the First Hardware Demo

Reached after M11.

Required flow:

```text
real soil sensor
     ↓
ESP32 Rust firmware
     ↓ MQTT
Mosquitto
     ↓
Rust edge controller
     ↓
SQLite
     ↓
optional cloud PostgreSQL
```

Then:

```text
edge publishes water command
     ↓
ESP32 validates safety/TTL/dedup
     ↓
pump executes bounded dose
     ↓
command result
     ↓
edge records watering event
```

A physical safety failure must fail closed.

---

# 25. Final Goal

Rhizo Edge should end up as a clean reference architecture for:

```text
Rust
+
ESP32
+
MQTT
+
edge computing
+
offline-first systems
+
IoT device control
+
fail-safe automation
+
local persistence
+
cloud synchronization
+
Rust web UI
```

The indoor plant version must be genuinely usable.

The farm version must be a natural evolution of the same architecture rather than a separate rewrite.

Start now with the planning pass, then implement M0 and continue milestone by milestone while keeping all documentation and issue state synchronized with the code.
