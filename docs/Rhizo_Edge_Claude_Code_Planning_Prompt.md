# Claude Code Planning Prompt — Rhizo Edge

You are working on a new project called **Rhizo Edge**.

This is **PLANNING PHASE ONLY**.

Do **not** implement production code.
Do **not** start milestone implementation.
Do **not** create placeholder service crates just to make the repository look started.
Do **not** write ESP32 firmware.
Do **not** build the UI.

Your task is to turn the existing project concept/specification into a complete, implementation-ready engineering plan that another Claude Code session can execute milestone by milestone without needing to rediscover architecture.

The repository should contain the project specification:

```text
Rhizo_Edge_PROJECT_PLAN.md
```

If it is still named:

```text
OpenSoil_Edge_PROJECT_PLAN.md
```

or still contains the old project name `OpenSoil Edge`, treat it as the same specification and update project-facing references to:

```text
Rhizo Edge
```

Do not materially change product intent unless required to resolve contradictions.

---

# 1. Project Summary

Rhizo Edge is an **offline-first soil monitoring and fail-safe automated irrigation platform**.

Initial target:

```text
indoor houseplants
```

Long-term direction:

```text
houseplants
→ multi-plant home system
→ greenhouse
→ agricultural field deployments
```

Core stack:

```text
Rust everywhere
ESP32 in Rust
MQTT
Mosquitto
Rust Edge Controller
SQLite at edge
Rust Cloud API
PostgreSQL in Docker
Rust UI in a later milestone
Docker-based simulator/test environment
```

Primary architectural principle:

> A plant must remain safely monitored and controllable locally even when Internet/cloud connectivity is unavailable.

---

# 2. Required System Boundaries

The planning must treat these as separate subprojects/components.

## A. ESP32 Node

Real physical device.

Rust firmware.

Responsibilities eventually include:

- soil sensor acquisition
- soil temperature
- EC
- optional pot weight
- reservoir level
- leak detection
- pump control
- Wi-Fi
- MQTT
- retained status
- MQTT Last Will
- configuration
- commands
- hard local safety limits
- command TTL validation
- command deduplication
- watchdog/recovery

The firmware must ultimately be compilable and flashable onto a real ESP32 board.

---

## B. Device Simulator

Host-side Rust application.

It must behave like the future ESP32 device using the **same protocol**.

It should make it possible to build and test almost the entire platform before physical hardware exists.

The simulator should eventually support:

- soil drying
- watering response
- moisture
- temperature
- EC
- weight
- tank level
- leak state
- command execution
- device restart
- MQTT reconnect
- duplicate messages
- invalid values
- network failures
- accelerated virtual time

---

## C. Edge Controller

Rust local control-plane service.

Responsibilities:

- consume MQTT
- validate messages
- deduplicate QoS 1 deliveries
- local SQLite persistence
- device registry
- plant state
- recommendations
- watering state machine
- pump command publication
- command-result handling
- fail-safe logic
- offline operation
- local REST API
- metrics/logging
- cloud event queue
- cloud synchronization

The Edge Controller must own local safety-critical behavior.

---

## D. Cloud Service

Rust service.

Initially fully local/containerized:

```text
Rust Cloud API
+
PostgreSQL
+
Docker
```

Responsibilities later:

- receive idempotent synced events
- store historical data
- expose read APIs
- represent multiple edge instances
- accept replay after outages
- remain non-critical to immediate irrigation operation

---

## E. UI

Later milestone.

Separate Dockerized Rust application.

It must eventually allow:

- system overview
- device health
- plants
- telemetry
- charts
- watering history
- recommendation
- manual watering
- automatic watering enable/disable
- safety lockouts
- edge/cloud health
- pending sync visibility

Prefer:

```text
Leptos
```

or another justified Rust-first web approach.

Do not introduce Node.js/TypeScript by default.

---

# 3. Expected Repository Documentation Structure

Create/refine a documentation structure approximately like this:

```text
README.md
ROADMAP.md
Rhizo_Edge_PROJECT_PLAN.md

docs/
├── README.md
│
├── architecture/
│   ├── system-overview.md
│   ├── component-model.md
│   ├── data-flow.md
│   ├── deployment-model.md
│   ├── safety-invariants.md
│   ├── failure-model.md
│   └── dependency-graph.md
│
├── adr/
│   ├── 001-*.md
│   ├── 002-*.md
│   └── ...
│
├── prd/
│   ├── 000-platform-foundation.md
│   ├── 010-domain-and-protocol.md
│   └── ...
│
├── protocol/
│   ├── mqtt-v1.md
│   ├── http-api-boundaries.md
│   └── versioning-policy.md
│
├── testing/
│   ├── strategy.md
│   ├── simulator-strategy.md
│   ├── hardware-in-the-loop.md
│   └── failure-scenarios.md
│
└── issues/
    ├── M0/
    ├── M1/
    ├── M2/
    └── ...
```

You may improve the exact structure, but keep it simple and implementation-oriented.

---

# 4. Planning Deliverables

You must produce ALL of the following.

## 4.1 README.md

Create or update the root README.

It should explain:

- what Rhizo Edge is
- why edge-first
- project status
- major components
- high-level architecture
- development strategy
- simulator-first approach
- roadmap link
- docs link

Do not write a marketing-heavy README.

Keep it engineering-focused.

---

## 4.2 docs/README.md

Create a documentation index.

It must link to:

- architecture
- ADRs
- PRDs
- protocols
- testing
- issue plans
- roadmap

---

## 4.3 ROADMAP.md

This is a central artifact.

It must include:

### Milestone table

For each milestone:

- ID
- Name
- Objective
- Major deliverables
- Depends on
- Exit criteria
- Status

Use statuses like:

```text
PLANNED
READY
BLOCKED
IN PROGRESS
DONE
```

During this planning pass, milestones should normally remain:

```text
PLANNED
```

or:

```text
READY
```

only when all dependencies and issues are completely specified.

### Dependency graph

Include a readable dependency graph.

Example:

```text
M0
↓
M1
↓
M2
↓
M3
├──→ M4
├──→ M5
└──→ ...
```

Use the real planned dependencies rather than this example.

### Invariant registry summary

Reference safety invariant IDs.

### Planning conventions

Document:

- issue sizing
- dependency notation
- acceptance criteria style
- verification expectations
- definition of milestone completion

---

# 5. Milestone Model

Use the following baseline milestone structure.

You may split or slightly reorganize it if there is a concrete architectural reason, but do not collapse major responsibilities together.

---

## M0 — Foundation and Engineering Baseline

Plan:

- Rust workspace
- toolchain pin
- CI
- formatting/linting
- Docker Compose skeleton
- docs baseline
- testkit baseline
- configuration conventions
- error-handling conventions

No application behavior.

---

## M1 — Domain Model and MQTT Protocol

Plan:

- identifiers
- telemetry types
- configuration
- device state
- command types
- command results
- topic hierarchy
- QoS
- retained messages
- Last Will
- message IDs
- sequence numbers
- TTL
- versioning
- compatibility rules

---

## M2 — Device Simulator

Plan:

- Rust simulator
- Mosquitto integration
- simulated sensor state
- drying/watering model
- pump behavior
- config handling
- commands/results
- fault injection
- accelerated time

---

## M3 — Edge MQTT Ingestion and SQLite

Plan:

- MQTT subscription
- decoding
- validation
- deduplication
- SQLite schema
- migrations
- ingestion persistence
- graceful restart
- metrics/logging

---

## M4 — Device Registry and Health

Plan:

- online/offline
- retained device status
- Last Will
- last seen
- firmware/version metadata
- stale-device detection
- sensor health
- local read API

---

## M5 — Plant Model and Recommendations

Plan:

- plant entity
- plant profiles
- moisture targets
- EC thresholds
- trends
- last watering
- recommendation state
- explainable reasons
- manual watering detection

---

## M6 — Irrigation State Machine and Safety

Plan:

- manual command
- recommended watering
- automatic watering
- multi-dose loop
- absorption wait
- cooldown
- max single dose
- max daily dose
- tank lockout
- leak lockout
- stale sensor lockout
- invalid sensor lockout
- command TTL
- deduplication
- recovery after restart

---

## M7 — Cloud API and PostgreSQL

Plan:

- Rust API
- PostgreSQL
- edge identity
- idempotent event ingestion
- event replay
- pending sync queue
- retries
- backoff/jitter
- cloud outage behavior
- read APIs

Everything remains runnable locally in Docker.

---

## M8 — Full Software-Only E2E Environment

Plan:

```text
device simulator
→ Mosquitto
→ edge
→ SQLite
→ cloud API
→ PostgreSQL
```

One-command startup.

Automated end-to-end scenarios.

No hardware required.

---

## M9 — ESP32 Rust Firmware Foundation

Plan:

- selected ESP32 chip/board target
- Rust toolchain
- build tooling
- Wi-Fi
- MQTT
- status/LWT
- config
- commands/results
- fake sensor adapter
- fake pump adapter
- host-testable shared logic
- firmware build verification
- flashing instructions

The planning must investigate the appropriate current Rust-on-ESP32 ecosystem and record the selected approach in ADR form.

Do not implement firmware during this planning pass.

---

## M10 — Real Soil Sensor Integration

Plan:

- sensor abstraction
- real adapter
- likely RS485/Modbus path
- calibration
- errors
- malformed readings
- telemetry integration
- optional simpler first sensor if justified

---

## M11 — Real Pump and Safety Hardware

Plan:

- pump interface
- hardware driver
- calibration
- reservoir level
- leak detector
- local hard limits
- fail-closed boot state
- restart while watering
- watchdog behavior

---

## M12 — Rust UI

Plan:

- separate Docker container
- Rust web stack
- edge API integration
- dashboards
- actions
- charts
- health
- watering controls
- lockouts
- pending sync state

---

## M13 — Multi-Plant Home System

Plan:

- multiple nodes
- onboarding
- credentials
- multiple pumps
- multiple plant profiles
- centralized overview

---

## M14 — Field Readiness

Architecture/planning only initially.

Plan future support for:

- multi-depth probes
- RS485 buses
- LoRaWAN
- LTE-M
- NB-IoT
- gateways
- irrigation zones
- weather data
- low power
- solar/battery
- field deployment constraints

Avoid speculative implementation.

---

# 6. ADR Requirements

Create the necessary ADRs.

At minimum, plan and write:

```text
ADR-001 Rust workspace and crate boundaries
ADR-002 MQTT protocol/QoS/topic strategy
ADR-003 Edge-first ownership model
ADR-004 SQLite local persistence
ADR-005 Cloud event model and idempotency
ADR-006 Irrigation state machine ownership
ADR-007 ESP32 Rust stack/toolchain
ADR-008 Shared simulator/firmware contracts
ADR-009 Rust UI architecture
ADR-010 Observability strategy
ADR-011 Configuration/secrets strategy
ADR-012 Device identity/provisioning
ADR-013 Clock/time semantics
ADR-014 Failure/retry policy
```

Create more only when needed.

Every ADR must have:

```text
# ADR-NNN — Title

## Status

## Context

## Decision

## Alternatives considered

## Consequences

## Risks

## Follow-up
```

Do not leave major decisions as vague TODOs when they can reasonably be decided now.

---

# 7. PRD Requirements

Create implementation-oriented PRDs.

Baseline set:

```text
000-platform-foundation.md
010-domain-and-mqtt-protocol.md
020-device-simulator.md
030-edge-ingestion-and-storage.md
040-device-registry-and-health.md
050-plant-model-and-recommendations.md
060-irrigation-control-and-safety.md
070-cloud-sync-and-storage.md
080-end-to-end-test-environment.md
090-esp32-rust-firmware.md
100-real-soil-sensor.md
110-real-pump-and-safety-hardware.md
120-rust-ui.md
130-multi-plant-home.md
140-field-readiness.md
```

Every PRD must contain:

```text
# PRD NNN — Title

## Summary

## Problem

## Goals

## Non-goals

## User/system flows

## Functional requirements

## Interfaces

## Data model

## State model

## Failure modes

## Safety implications

## Observability

## Testing strategy

## Acceptance criteria

## Dependencies

## Open questions

## Future work
```

If a section does not apply, say why instead of omitting it.

---

# 8. MQTT Protocol Specification

Create:

```text
docs/protocol/mqtt-v1.md
```

It must be specific enough for both simulator and firmware implementations to be independently written and interoperate.

Define:

- base namespace
- topic grammar
- device ID grammar
- telemetry topics
- device status
- configuration
- water command
- tare/calibrate command
- command result
- message envelope
- timestamps
- sequence semantics
- message IDs
- command IDs
- TTL
- QoS
- retained semantics
- Last Will
- reconnect expectations
- deduplication expectations
- ordering assumptions
- compatibility/versioning
- malformed-message behavior

Base namespace:

```text
rhizo/v1/
```

---

# 9. Data Model Planning

Document concrete initial schemas.

## SQLite

At least:

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

For every table, define:

- purpose
- important columns
- identifiers
- uniqueness constraints
- indexes
- retention assumptions
- restart/recovery implications

---

## PostgreSQL

At least:

```text
edge_instances
devices
plants
measurements
watering_events
device_events
synced_events
```

Cloud event replay must be idempotent.

Specify how logical duplicate prevention works.

---

# 10. Safety Invariant Registry

Create:

```text
docs/architecture/safety-invariants.md
```

Use stable IDs.

At minimum:

```text
SAFETY-001 duplicate watering command cannot duplicate physical watering
SAFETY-002 expired command never executes
SAFETY-003 leak state disables watering
SAFETY-004 empty reservoir disables watering
SAFETY-005 stale/invalid moisture disables auto-watering
SAFETY-006 max daily automatic water cannot be exceeded
SAFETY-007 ESP32 hard maximum cannot be bypassed by edge/cloud
SAFETY-008 cloud outage cannot disable local monitoring
SAFETY-009 cloud outage cannot bypass safety
SAFETY-010 edge restart cannot replay completed command
SAFETY-011 ESP32 restart converges to pump-off safe state
SAFETY-012 uncertainty defaults to no watering
```

For each invariant define:

- rationale
- enforcing component(s)
- persisted state required
- failure scenarios
- planned automated test(s)
- milestone where it becomes enforced

---

# 11. Failure Model

Create:

```text
docs/architecture/failure-model.md
```

At minimum cover:

### MQTT

- broker unavailable
- broker restart
- QoS 1 duplicate
- disconnect
- reconnect
- delayed messages
- malformed payload
- unexpected ordering

### Device

- ESP32 restart
- Wi-Fi failure
- sensor failure
- stale readings
- corrupted reading
- pump command interrupted

### Edge

- process crash
- SQLite unavailable/locked
- restart during state transition
- duplicate command result

### Cloud

- unavailable
- timeout
- DNS error
- 5xx
- duplicate replay
- prolonged outage

### Hardware

- pump does not deliver water
- pump stuck on
- reservoir empty
- leak detected
- calibration drift

For each failure, identify:

```text
detection
expected state
recovery
data loss expectations
safety behavior
observability
```

---

# 12. Issue Generation

This is one of the most important outputs.

Create repository-tracked issue files under:

```text
docs/issues/M0/
docs/issues/M1/
...
docs/issues/M14/
```

Issues must be **small, ordered, and implementable**.

Bad:

```text
001-implement-edge-controller.md
```

Good:

```text
001-add-edge-service-binary.md
002-add-mqtt-connection-config.md
003-add-topic-subscriptions.md
004-add-message-envelope-decoding.md
005-add-telemetry-validation.md
006-add-sqlite-pool-and-migrations.md
007-persist-soil-measurements.md
008-add-message-deduplication.md
009-add-graceful-shutdown.md
```

Each issue must contain:

```text
# Issue Mx-NNN — Title

## Context

## Goal

## Scope

## Non-goals

## Dependencies

## Implementation notes

## Acceptance criteria

## Verification

## Tests required

## Documentation impact

## Files likely affected
```

---

# 13. Issue Quality Rules

Each issue should generally be implementable in one focused coding session.

Avoid issues that combine:

```text
protocol + persistence + UI + hardware
```

Prefer dependency chains of small changes.

Every issue must have explicit acceptance criteria.

Every issue must identify its milestone.

Every milestone must have a final verification/acceptance issue.

Create enough issues that implementation can proceed mechanically.

Do not artificially limit issue count.

A milestone with substantial behavior will often need 8–20 issues.

---

# 14. Dependency Mapping

Create:

```text
docs/architecture/dependency-graph.md
```

Include:

## Milestone dependencies

and

## Important issue-level dependencies

Example format:

```text
M1-003 → M1-004 → M2-002
M3-006 → M3-007
M6-002 + M6-004 → M6-008
```

Do not map every trivial issue if it adds noise.

Map dependencies where execution order matters.

---

# 15. Test Strategy

Create:

```text
docs/testing/strategy.md
```

The test model must include:

## Unit tests

- protocol parsing
- domain rules
- state transitions
- watering limits
- command TTL
- deduplication
- calibration math

## Property/invariant tests

Especially watering safety.

## Integration tests

```text
simulator
→ Mosquitto
→ edge
```

and later:

```text
edge
→ cloud
→ PostgreSQL
```

## End-to-end tests

Full Docker environment.

## Failure tests

- broker restart
- edge restart
- cloud outage
- duplicates
- stale sensor
- leak
- empty tank

## Hardware-in-the-loop

Planned only for later milestones.

---

# 16. Test Mode Requirement

The entire project architecture must support a **hardware-free test mode**.

Before any real ESP32 exists, it must eventually be possible to run:

```text
device-simulator
Mosquitto
edge-controller
SQLite
cloud-api
PostgreSQL
```

in Docker/local processes.

Later:

```text
rhizo-ui
```

is added as another Docker service.

Plan all interfaces so that:

> replacing the simulator with a real ESP32 changes the device implementation, not the MQTT protocol or Edge Controller architecture.

This is a hard requirement.

---

# 17. ESP32 Planning Requirement

During planning, research and decide the expected Rust ESP32 approach.

Document:

- selected initial board/chip family
- why
- Rust framework/toolchain
- `std` vs `no_std` direction
- Wi-Fi support
- MQTT support
- shared crate strategy
- host test strategy
- firmware compile verification strategy
- real flashing workflow
- development prerequisites
- CI feasibility

Do not write firmware now.

Do not invent commands.

Record current authoritative tooling references in the ADR if web/documentation access is available.

---

# 18. UI Planning Requirement

The UI is later, but architecture must be planned now.

Decide:

- Rust web framework
- edge API boundary
- whether SSR is needed
- charting approach
- real-time update strategy
- Docker deployment
- local-only initial security model

UI must not bypass safety logic.

Manual watering from UI must flow:

```text
UI
→ Edge API
→ Edge validation/state machine
→ MQTT command
→ ESP32
```

Never:

```text
UI
→ MQTT pump command directly
```

---

# 19. Configuration Model

Plan configuration layers.

Potential levels:

```text
system
edge instance
device
plant profile
plant instance
safety hard limits
```

Clearly identify which configuration:

- can be changed from UI
- is edge-owned
- is device-retained
- is compiled/hard safety
- may later come from cloud

Prevent cloud/UI configuration from overriding hard device safety.

---

# 20. Time Model

Document how time is handled.

Decide:

- UTC storage
- device timestamps
- edge receive time
- sequence fallback
- stale-data calculation
- command expiry
- simulator virtual time
- deterministic test clock

Time semantics are safety-relevant.

---

# 21. Observability Plan

Create or include in ADR:

- structured logging fields
- metrics
- health endpoints
- diagnostic events
- device online state
- sync backlog
- watering state transitions
- failed command counters
- sensor errors
- MQTT reconnect metrics

Define metric names only where useful; avoid premature huge metric catalogs.

---

# 22. Security Scope

Plan V1 security realistically.

Local initial deployment can use:

- MQTT credentials
- no anonymous broker
- local network API
- Docker secrets/env for dev

Plan future:

- TLS
- per-device credentials
- certificates
- signed firmware
- secure provisioning
- cloud authentication

Do not let security planning block M0–M8.

---

# 23. Architectural Quality Bar

The planning should make the following possible:

### Software-only demo

```text
simulated plant dries
→ edge receives telemetry
→ recommendation appears
→ auto-water state machine sends bounded command
→ simulator applies water
→ edge rechecks
→ system recovers
```

Then:

```text
cloud is stopped
→ edge keeps operating
→ events queue
→ cloud returns
→ events replay idempotently
```

### Hardware demo later

```text
real soil probe
→ ESP32 Rust
→ MQTT
→ Edge
→ SQLite
→ Cloud
```

and:

```text
Edge
→ bounded water command
→ ESP32 local validation
→ pump
→ result
```

The planning artifacts must clearly lead to these demos.

---

# 24. Avoid Overengineering

Do not plan these as early requirements:

- Kubernetes
- Kafka
- microservice explosion
- distributed database
- multi-region cloud
- ML
- NPK inference
- LoRaWAN implementation
- complex IAM
- service mesh

The architecture may leave expansion points, but V1 must remain buildable by one developer.

---

# 25. Planning Validation

After generating all docs, perform a consistency review.

Check:

1. Every PRD maps to milestone(s).
2. Every milestone has issues.
3. Every issue maps back to a PRD or architecture need.
4. No milestone depends on an unspecified behavior.
5. Simulator and ESP32 use the same protocol.
6. Cloud is never required for local watering safety.
7. UI never directly controls hardware.
8. Safety invariants have enforcement milestones.
9. Every safety invariant has planned tests.
10. M0–M8 require no physical hardware.
11. ESP32 work begins only after the software contract is stable.
12. UI is planned as a later Dockerized subsystem.
13. Rust is used across all planned software/firmware components.
14. Failure behavior is explicit.
15. No roadmap milestone is impossibly large.
16. Issues are sufficiently granular.
17. Dependency order is implementable.
18. Documentation contains no contradictory terminology.

Fix inconsistencies before finishing.

---

# 26. Optional Documentation Validation Tool

If useful, add a lightweight documentation validation script such as:

```text
tools/check_docs.py
```

or a Rust equivalent.

It may verify:

- required PRDs exist
- ADR numbering is unique
- issue folders exist
- issue IDs are unique
- referenced milestone IDs exist
- ROADMAP links resolve

This is allowed during the planning phase because it validates planning artifacts.

Do not begin product implementation.

---

# 27. Final Planning Report

At the end, output a concise report containing:

## Files added

Grouped by:

- architecture
- ADR
- PRD
- protocol
- testing
- issues
- roadmap

## Files changed

## Milestones created

For each:

```text
M0 — ...
M1 — ...
...
```

## Issue count per milestone

Example:

```text
M0: 8
M1: 11
...
```

## Key architectural decisions

## Safety model summary

## Dependency summary

## Open risks

Only list genuinely unresolved risks.

## Implementation starting point

State exactly which issue should be implemented first in the next Claude Code session.

---

# 28. Stop Condition

This prompt is **planning only**.

Once:

- architecture docs
- ADRs
- PRDs
- roadmap
- protocols
- safety invariants
- test strategy
- milestone structure
- issue files
- dependency graph

are complete and internally consistent:

**STOP.**

Do not implement M0.

Do not create service binaries.

Do not create production Rust crates unless a tiny tooling crate is strictly needed to validate documentation structure.

The next session will receive a separate implementation prompt and execute the generated issues.

Begin by inspecting the repository and reading the full project specification.
