# Rhizo Edge — Roadmap

The execution plan for Rhizo Edge, from an empty repository to a system that can
be trusted with a real plant.

**Planning status:** complete. **Implementation status:** not started.
**Host toolchain:** Rust 1.98.0 ([ADR-001](docs/adr/001-rust-workspace-and-crate-boundaries.md)).

- Source of truth for *what* each milestone builds: [docs/prd/](docs/prd/)
- Source of truth for *why*: [docs/adr/](docs/adr/)
- Source of truth for *how, step by step*: [docs/issues/](docs/issues/)
- Execution order: [docs/architecture/dependency-graph.md](docs/architecture/dependency-graph.md)

---

## 1. Milestone table

| ID | Name | Objective | Depends on | Issues | Status |
|---|---|---|---|---|---|
| M0 | Foundation and Engineering Baseline | A clean Rust repository whose tooling, lint, test, container, and observability baseline every later milestone inherits | — | 13 | **READY** |
| M1 | Domain Model and MQTT Protocol | The shared wire contract and pure domain types that let simulator, edge, and firmware be written independently | M0 | 14 | **READY** |
| M2 | Device Simulator | A host device indistinguishable from firmware at the protocol level, with fault injection and virtual time | M1 | 15 | **READY** |
| M3 | Edge Ingestion and SQLite | Reliable MQTT consumption with durable deduplication and crash-safe persistence | M1, M2 | 16 | **READY** |
| M4 | Device Registry and Health | Device lifecycle, staleness, sensor health, config drift, and the first REST surface | M3 | 11 | **READY** |
| M5 | Plant Model and Recommendations | Plants, profiles, trends, manual-watering detection, and an explainable recommendation engine — **issuing no commands** | M4 | 13 | **READY** |
| M6 | Irrigation Control and Safety | The state machine, the safety gate, the command lifecycle, and every non-hardware SAFETY invariant | M5, M2 | 19 | **READY** |
| M7 | Cloud API and PostgreSQL | Optional idempotent history sync that cannot affect local safety | M6 | 15 | **READY** |
| M8 | End-to-End Test Environment | The whole software system reproducible and verifiable with one command, no hardware | M7 | 15 | **READY** |
| M9 | ESP32 Rust Firmware Foundation | Real firmware speaking the same protocol, with fake sensors and pump | M8 | 15 | PLANNED |
| M10 | Real Soil Sensor Integration | Real readings behind the unchanged `SoilSensor` trait | M9 | 11 | PLANNED |
| M11 | Real Pump and Safety Hardware | Real actuation with calibration and physically verified lockouts | M10 | 14 | PLANNED |
| M12 | Rust UI | A Tauri 2 + Leptos desktop client that structurally cannot bypass safety | M6 (functional), M11 (full picture) | 13 | PLANNED |
| M13 | Multi-Plant Home System | Several nodes, provisioning tooling, notifications, and a supportable deployment | M12 | 13 | PLANNED |
| M14 | Field Readiness Architecture | Architecture and honest constraints for greenhouse and field — **documentation only** | M13 | 7 | PLANNED |

**Total: 204 issues.**

### Status semantics

| Status | Meaning |
|---|---|
| `READY` | Fully specified; every issue written; no unresolved external dependency. Executable as soon as its predecessor completes. |
| `PLANNED` | Fully specified, but carries an unresolved external dependency — hardware to purchase, a toolchain to verify on a real machine, or a UI stack version to pin. |
| `BLOCKED` | A dependency is unmet or a decision is outstanding. None currently. |
| `IN PROGRESS` | Implementation started. |
| `DONE` | Every issue closed **and** the milestone's exit criteria demonstrably met. |

M0–M8 are `READY`: they are pure software, need no hardware, and every
prerequisite is a preceding milestone. M9–M11 are `PLANNED` because they depend
on physical hardware and on ADR-007's toolchain being executed on a real machine
(M9-001). M12–M13 are `PLANNED` pending pinned Tauri/Leptos versions. M14 is
`PLANNED` and produces documentation only.

---

## 2. Milestone detail

### M0 — Foundation and Engineering Baseline · READY

**Objective.** Establish workspace, toolchain, lint policy, configuration,
observability, containers, and CI so that every later milestone adds behaviour
rather than scaffolding.

**Deliverables.** Three-workspace layout with `firmware` and `ui` excluded ·
`rust-toolchain.toml` pinned to **1.98.0** · `unwrap_used`/`expect_used` denied
in libraries · `rhizo-telemetry` (tracing + Prometheus registry) · layered
edge configuration with fail-fast validation and secret redaction · full-jitter
backoff utility · `rhizo-testkit` with `TestClock` · Mosquitto with
authentication and `%u` ACLs · Compose skeleton · CI gate · documentation
validator.

**Exit criteria.** On a **fresh clone**: `cargo fmt --check`, `cargo clippy -D
warnings`, `cargo test --workspace`, `docker compose config`, and the docs
validator all pass. Mosquitto refuses anonymous connections and cross-device
publishes. Invalid configuration exits non-zero naming the key. `Debug` on the
config prints `[redacted]`.

**Invariants.** None enforced. Enables SAFETY-007 (workspace layout for the
shared contract crate) and SAFETY-012 (fail-fast posture, deny-unwrap).

**PRD.** [000](docs/prd/000-platform-foundation.md)

---

### M1 — Domain Model and MQTT Protocol · READY

**Objective.** Define the contract that cannot be changed cheaply once devices
exist in pots.

**Deliverables.** `rhizo-mqtt-contract` (`no_std` + `alloc`) implementing
[mqtt-v1.md](docs/protocol/mqtt-v1.md) · `DeviceId` grammar · `UtcMillis` ·
envelope with identity checking · topic grammar · all ten payload types ·
**`validate_water_command`, the single shared actuation gate** · protocol
fixture corpus · `rhizo-domain` skeleton with the `Clock` trait · `no_std` CI
verification · clippy ban on direct clock access in the domain.

**Exit criteria.** Every clause of mqtt-v1.md §2–§10 implemented or explicitly
noted. `validate_water_command`'s ordering matches §5.8 exactly. The contract
crate builds for a bare-metal target with default features off. `Utc::now()` in
`rhizo-domain` fails clippy. Every fixture behaves as documented.

**Invariants.** Delivers the mechanism for SAFETY-002, SAFETY-007, SAFETY-012;
enforcement tested in M6.

**PRD.** [010](docs/prd/010-domain-and-mqtt-protocol.md)

---

### M2 — Device Simulator · READY

**Objective.** A reference device that makes M3–M8 achievable without hardware —
and that is **never more permissive than firmware**.

**Deliverables.** Full protocol conformance including LWT and retained status ·
soil model with absorption lag, probe overshoot, and drainage · weight rising
immediately while VWC lags · tank, leak, EC models · **actuation exclusively
through `validate_water_command`** · NVS-equivalent persistent state · control
API · thirteen injectable faults · accelerated virtual time.

**Exit criteria.** Runs standalone against a bare broker. Exactly one call site
of `validate_water_command`. `requested_ml: 10000` published directly to the
broker never delivers above the hard limit. No retained messages on command or
telemetry topics. ACL isolation holds. A full cycle completes in under 10 s at
scale 600.

**Invariants.** Makes SAFETY-002, SAFETY-007, SAFETY-011 testable before
hardware exists.

**PRD.** [020](docs/prd/020-device-simulator.md)

---

### M3 — Edge Ingestion and SQLite · READY

**Objective.** Consume telemetry reliably and persist it crash-safely.

**Deliverables.** MQTT ingress with reconnect **and re-subscription** · decoding
with edge-stamped `received_at` · bounded, rate-limited quarantine ·
**the deduplicate-and-persist transaction** · per-field validation that keeps
good fields · full SQLite schema and migrations · `.sqlx` offline cache ·
ingestion metrics with a cardinality guard · failure classification · graceful
shutdown and startup restoration · retention that never prunes the ledger.

**Exit criteria.** Duplicate `message_id` produces one row. Edge restart
preserves history. **Broker restart results in re-subscription**, verified by
telemetry resuming. A message with one bad field stores the rest. SIGTERM exits
0; a task panic exits non-zero.

**Invariants.** Delivers the mechanism for SAFETY-001 and SAFETY-010; establishes
`received_at` authority that SAFETY-005 depends on.

**PRD.** [030](docs/prd/030-edge-ingestion-and-storage.md)

---

### M4 — Device Registry and Health · READY

**Objective.** Know whether a device is there, and how fresh its data is.

**Deliverables.** Status ingestion and LWT handling, order-insensitive ·
**auto-registration that creates a device and never a plant** · staleness
derived from `received_at` with a liveness **timer** · sensor health
distinguishing absent from unhealthy · config drift detection · health endpoints
excluding cloud reachability · device REST endpoints · API server with CORS off
by default.

**Exit criteria.** Killing the simulator marks it offline. Restart yields a new
`boot_id` without a spurious regression flag. Stopping telemetry while connected
produces a stale indication **from the timer**. An unknown device registers with
no plant. `/health/ready` is 200 with the cloud stopped, 503 with the broker
stopped. No endpoint changes `device_id`.

**Invariants.** Provides SAFETY-005's inputs; applies SAFETY-012 to onboarding.

**PRD.** [040](docs/prd/040-device-registry-and-health.md)

---

### M5 — Plant Model and Recommendations · READY

**Objective.** Turn telemetry into an explainable recommendation — while still
issuing no commands, so the logic can be validated against a real plant for a
week before anything can pump.

**Deliverables.** Plant and profile CRUD with `auto_watering_enabled` defaulting
false · profile validation that **rejects rather than clamps** · least-squares
moisture trend returning `None` on sparse data · dry-duration tracking with gap
handling · manual-watering detection with command attribution · stuck-sensor
detection · rule-based recommendation with typed reasons · plant state
derivation · EC trend and warning · evaluation tick and endpoints.

**Exit criteria.** Drying produces `WaterRecommended` with a non-empty reason
list and **zero MQTT commands published**. A profile with `dose_ml = 200` is
rejected with 422 naming the firmware limit. A moisture step following a command
creates no second event.

**Invariants.** None enforced (no actuation). Builds the gate's inputs.

**PRD.** [050](docs/prd/050-plant-model-and-recommendations.md)

---

### M6 — Irrigation Control and Safety · READY

**Objective.** The milestone where software can move water. The most
safety-critical in the project.

**Deliverables.** `IrrigationInputs` with `Option` for every absent-able input ·
**the safety gate, exhaustive with no catch-all arm** · leak/tank/validity/
staleness checks · the pure, total state machine · the rolling 24-hour window
derived from rows · command persistence **before** publication · publication
with `retain = false` · result handling that never invents a watering event ·
**retry with the same `command_id`, never a new one** · restart reconciliation ·
config publication · control metrics · clock-step detection · watering and
lockout endpoints with **no override parameter** · no-delivery detection · the
full property-test suite.

**Exit criteria.** `cargo test safety_` fully green. `PROPTEST_CASES=10000 cargo
test safety_006` passes. `POST /water` during a leak returns 409 with nothing
published. Restart after publish produces no second command and one watering
event. No `_ =>` arm on any safety match.

**Invariants enforced.** SAFETY-001, -002, -003, -004, -005, -006, -007 (via the
shared validator), -010, -012.

**PRD.** [060](docs/prd/060-irrigation-control-and-safety.md)

---

### M7 — Cloud API and PostgreSQL · READY

**Objective.** Optional history that cannot become load-bearing.

**Deliverables.** Cloud API with idempotent batch ingestion returning **per-event
results where `duplicate` is a success** · two-layer schema with
`UNIQUE (edge_id, event_id)` · order-insensitive projections · cloud client with
exhaustive classification · outbox drain fully decoupled from control · adaptive
batch sizing · **value-tiered cap that never prunes high-tier events** · sync
metrics · the differential independence test · time round-trip test ·
reprojection command · read endpoints with **no command or config-write route**.

**Exit criteria.** With the cloud stopped, every local function works and
`/health/ready` returns 200. Replay creates no duplicate rows.
`safety_009_decisions_identical_with_cloud_down` passes. `rhizo-domain` has no
cloud dependency.

**Invariants enforced.** SAFETY-008, SAFETY-009.

**PRD.** [070](docs/prd/070-cloud-sync-and-storage.md)

---

### M8 — End-to-End Test Environment · READY

**Objective.** Make the whole software system reproducible in one command, and
prove the safety claims are detected rather than assumed.

**Deliverables.** Dockerfiles with correct signal handling · complete Compose
topology · test overlay with **restart policies disabled** · time-scale
agreement check · Rust scenario runner asserting on observable state with
failure dumps · fourteen e2e scenarios · the eighteen-step first demo ·
**six-mutation verification** · CI job.

**Exit criteria.** `docker compose up --build` works from a fresh clone. The
suite runs with one command, exits 0, under 10 minutes. `scenario_first_demo`
reproduces all eighteen steps. **Each of the six mutations turns the suite red.**

**Invariants.** Re-verifies SAFETY-001…-010 and -012 in the assembled system.

**PRD.** [080](docs/prd/080-end-to-end-test-environment.md)

> **M8 is the software-only demo.** It requires no ESP32, no pump, no plant, and
> the M12 desktop UI is deliberately **not** part of it.

---

### M9 — ESP32 Rust Firmware Foundation · PLANNED

**Objective.** Real firmware speaking the identical protocol, with fake sensors
and pump, so the simulator's fidelity claim is tested.

**Deliverables.** Verified toolchain (M9-001 executes and corrects ADR-007) ·
firmware CI job · own workspace with the contract crate by **path** · NVS and
MAC-derived identity · hardware traits with `Clock::now_ms() -> Option` · the
simulator/firmware conformance test · **pump off as the first statement in
`main`** · Wi-Fi and SNTP · MQTT with LWT · telemetry · command handling through
the shared validator with an NVS dedup ring · config handling · interrupted-dose
reporting · serial provisioning.

**Exit criteria.** Builds for the ESP target with no board. Conformance test
shows identical behaviour to the simulator. **With a board:** HIL-1 passes on a
multimeter across 20 resets; a duplicate `command_id` survives a power cycle;
blocking SNTP refuses commands while telemetry continues.

**Invariants enforced.** SAFETY-011 (firmware); SAFETY-002 and SAFETY-007 on
real silicon; SAFETY-001 gains its device-side enforcement point.

**External dependency.** One ESP32-C3 board; ADR-007's toolchain verified on the
development machine.

**PRD.** [090](docs/prd/090-esp32-rust-firmware.md)

---

### M10 — Real Soil Sensor Integration · PLANNED

**Objective.** Real readings behind the unchanged trait.

**Deliverables.** Configuration-selected adapters · generic Modbus RTU client ·
**register maps as data, not code** · Modbus and analogue soil sensors ·
calibration where **uncalibrated publishes `null`** · error handling and health ·
sensor config schema · metrics and events · gravimetric validation.

**Exit criteria.** Readings flow end to end. Switching analogue↔Modbus is a
configuration change. Unplugging the probe produces `null` and a `SensorFault`
lockout. Readings match a gravimetric reference within documented bounds.
**`git diff` shows no edge-side change for sensor support.**

**Invariants.** SAFETY-005 becomes physically real.

**External dependency.** An RS485 or capacitive probe; a scale and oven for the
gravimetric reference.

**PRD.** [100](docs/prd/100-real-soil-sensor.md)

---

### M11 — Real Pump and Safety Hardware · PLANNED

**Objective.** Real actuation, physically verified. Where a software defect
becomes a wet floor.

**Deliverables.** Pump driver with a **hardware gate pull-down** · run guard on a
**task independent of MQTT** · fault latching · calibration command rejecting
σ > 5 % · tank and leak adapters where `null` means refusal · leak interruption
within 1 s · hardware config schema · HIL-1, -3, -4, -5, -6 executed and
recorded.

**Exit criteria.** HIL-1 passes on a multimeter. A 40 ml request delivers within
±10 %, measured. `requested_ml: 10000` published directly delivers no more than
the hard limit, **measured in a cup**. A leak stops an in-progress dose within
1 s. `POST /water` during a leak returns 409.

**Invariants enforced.** SAFETY-003, -004, -007, -011 physically.

**External dependency.** Peristaltic pump, MOSFET driver with gate pull-down,
external supply, tubing, reservoir, tank and leak sensors, measuring cup,
multimeter, in-line power switch.

**PRD.** [110](docs/prd/110-real-pump-and-safety-hardware.md)

---

### M12 — Rust UI · PLANNED

**Objective.** An operations interface that **structurally cannot** bypass safety.

**Deliverables.** Tauri 2 + Leptos CSR + Trunk workspace with **no `package.json`
and no MQTT or `rhizo-domain` dependency** · shared API DTOs where 409 maps to a
distinct `Refused` state · overview, plant, device, events, and sync views ·
watering actions with **no override control** · inline SVG charts with the target
band and watering markers · profile editor · connection-state handling that never
shows a blank screen · packaging with WebView2 bootstrap · CI job asserting no
Node artefacts.

**Exit criteria.** Builds on Windows and Linux. No JS toolchain anywhere. A leak
lockout is prominent with no clear button and manual watering shows the reason.
Stopping the edge shows greyed last-known data with its age.

**Invariants.** Enforces none; must be incapable of violating any. Verified by
the absent dependencies and the absent override control.

**External dependency.** Pinned Tauri 2 and Leptos versions (M12-001).

**PRD.** [120](docs/prd/120-rust-ui.md)

---

### M13 — Multi-Plant Home System · PLANNED

**Objective.** A supportable household deployment.

**Deliverables.** Multi-device operation with **cross-plant isolation** ·
`rhizo-provision` refusing credential reuse · reservoir entity with
**lowest-reading-wins** resolution · grouping and filtering · cross-device cap
validation · notifications **dispatched from a separate task** · backup with
verified restore · systemd deployment for a Raspberry Pi · measurement
downsampling that never aggregates the ledger · multi-device scenarios · UI at
scale.

**Exit criteria.** SCEN-080 shows byte-identical state for unaffected plants.
Provisioning refuses reuse without `--force`. A dead notification channel does
not delay the control loop. The system survives a Pi reboot. Backup and restore
reproduce identical watering history. 20 plants evaluate within one tick.

**Invariants.** Every existing invariant must hold **per plant and per device**;
SAFETY-004 extends to shared reservoirs.

**External dependency.** Three or more ESP32 nodes; a Raspberry Pi.

**PRD.** [130](docs/prd/130-multi-plant-home.md)

---

### M14 — Field Readiness Architecture · PLANNED

**Objective.** Map the route to greenhouse and field honestly, **without
speculative implementation**.

**Deliverables.** Reservations verified **against code** · connectivity
assumptions traced to specific code with duty-cycle arithmetic · v2 protocol
requirements for constrained radio, including the unsolved TTL-without-a-clock
problem · zone and multi-depth model with the valve-stuck-open analysis · weather
boundary (recommendation input only, never the gate) · field security
requirements stated plainly.

**Exit criteria.** Every reservation verified against code. **`git diff` shows no
speculative implementation.** Open questions recorded as genuinely unresolved.

**PRD.** [140](docs/prd/140-field-readiness.md)

---

## 3. Milestone dependency graph

```text
M0  Foundation
 │
 ▼
M1  Domain + MQTT contract
 │
 ├────────────────┐
 ▼                ▼
M2  Simulator    M3  Ingestion + SQLite
 │                │        (M3 also needs M2 as a traffic source)
 └───────┬────────┘
         ▼
        M4  Registry + Health
         ▼
        M5  Plant model + Recommendations
         ▼
        M6  Irrigation + Safety  ◄── also needs M2's refusal parity
         │
         ├──────────────────────────────┐
         ▼                              │
        M7  Cloud + PostgreSQL          │
         ▼                              │
        M8  End-to-end environment      │  M12 depends on M6 functionally,
         ▼                              │  NOT the reverse — the UI is never
        M9  ESP32 firmware              │  a prerequisite of M8
         ▼                              │
        M10 Real soil sensor            │
         ▼                              │
        M11 Real pump + safety hardware │
         ▼                              ▼
        M12 Rust UI  ◄──────────────────┘
         ▼
        M13 Multi-plant home
         ▼
        M14 Field readiness (docs only)
```

Full detail, including issue-level ordering:
[docs/architecture/dependency-graph.md](docs/architecture/dependency-graph.md).

---

## 4. Safety invariant ownership

Full registry: [docs/architecture/safety-invariants.md](docs/architecture/safety-invariants.md).

| ID | Invariant | Enforcing component | Enforced at | Re-verified |
|---|---|---|---|---|
| SAFETY-001 | Duplicate command → single watering | Edge + Device | M6 (edge), M9 (device) | M8, M11 |
| SAFETY-002 | Expired command never executes | Device | M6 (sim), M9 (fw) | M8, M11 |
| SAFETY-003 | Leak disables all watering | Edge + Device | M6 | M8, M11 |
| SAFETY-004 | Tank below minimum disables watering | Edge + Device | M6 | M8, M11, M13 |
| SAFETY-005 | Stale/invalid moisture disables auto-watering | Edge | M6 | M8, M10 |
| SAFETY-006 | Rolling 24 h cap never exceeded | Edge | M6 | M8 |
| SAFETY-007 | Device hard maximum cannot be bypassed | Device firmware | M6 (validator), M9 (fw) | M8, M11 |
| SAFETY-008 | Cloud outage cannot disable monitoring | Edge | M7 | M8 |
| SAFETY-009 | Cloud outage cannot bypass safety | Edge (structural) | M7 | M8 |
| SAFETY-010 | Edge restart cannot replay a completed command | Edge | M6 | M8 |
| SAFETY-011 | Device restart converges to pump-off | Device | M9 | M11 |
| SAFETY-012 | Uncertainty defaults to no watering | Edge domain | M6 | M8 |

Every invariant names at least one automated test in the registry. M6 and M7 are
not complete until those tests exist and pass.

---

## 5. Planning conventions

### Issue sizing

One issue is one focused implementation session — typically 100–400 lines of
production code plus tests.

An issue is **too large** if it combines layers (protocol *and* persistence *and*
API), if its acceptance criteria exceed roughly ten checkboxes, or if its title
needs "and". It is **too small** if it cannot be verified independently.

Every milestone ends with a `NNN-mM-verification` issue that closes it.

### Issue file structure

Every issue carries these sections, in this order:

```text
Context · Goal · Scope · Non-goals · Dependencies · Implementation notes
Acceptance criteria · Verification · Tests required · Documentation impact
Files likely affected
```

### Identifier conventions

| Kind | Form | Example |
|---|---|---|
| Milestone | `M<n>` | `M6` |
| Issue | `M<n>-NNN` | `M6-009` |
| ADR | `ADR-NNN` | `ADR-006` |
| PRD | `PRD NNN` | `PRD 060` |
| Safety invariant | `SAFETY-NNN` | `SAFETY-006` |
| Test scenario | `SCEN-NNN` | `SCEN-040` |
| Functional requirement | `F-NNN-NN` | `F-060-20` |

Safety tests are named `safety_NNN_<description>` so that `cargo test safety_`
runs the whole safety suite. That command is the project's definition of "are
the invariants still enforced?".

### Dependency notation

`M6-008 → M6-009` means M6-008 must be complete before M6-009 begins.
`M6-002 + M6-004 → M6-006` means both are prerequisites.

An issue's `Dependencies` section is normative; the dependency graph is the
readable summary.

### Acceptance criteria style

Acceptance criteria are **observable and checkable**, not aspirational. They
assert on API responses, database rows, captured MQTT traffic, metric values, or
exit codes — never on log strings, and never "the code is correct".

Each issue's `Verification` section carries the literal commands to run.

### Verification expectations

Every issue is verified by running its `Verification` commands and confirming
its `Tests required`. The project-wide gate is:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo build -p rhizo-mqtt-contract --no-default-features --target thumbv7em-none-eabi
docker compose -f deploy/docker-compose.yml config
cargo run --manifest-path tools/docscheck/Cargo.toml   # planning-artefact validator
# (becomes `cargo run -p rhizo-docscheck` once M0-011 adopts it into the workspace)
```

### Definition of milestone completion

A milestone is `DONE` when **all** of the following hold:

1. Every issue in its directory is implemented and its acceptance criteria met.
2. The milestone's exit criteria in this document are demonstrably met, with the
   evidence recorded — not asserted.
3. The full project gate passes.
4. Every SAFETY invariant the milestone claims to enforce has a **green** test.
5. Documentation touched by the milestone is updated in the same change as the
   code, including this file's status column and the invariant registry.
6. A milestone report is recorded: files added, files changed, tests added,
   commands run, results, known limitations, next milestone.

**No milestone is complete while its acceptance tests are red.** A milestone is
never marked `DONE` on the basis of closed issues alone.

### Working discipline per issue

```text
1. read the issue and its dependencies
2. implement
3. cargo fmt
4. cargo clippy -D warnings
5. cargo test (affected crates, then workspace)
6. run the issue's Verification commands
7. update docs if behaviour changed
8. tick the acceptance criteria
```

---

## 6. Toolchain

| Component | Toolchain |
|---|---|
| Host workspace (`crates/*`) | **Rust 1.98.0**, pinned in `rust-toolchain.toml` |
| UI workspace (`ui/rhizo-ui`) | **Rust 1.98.0** plus the `wasm32-unknown-unknown` target |
| Firmware (`firmware/esp32-node`) | 1.98.0 **where the Espressif ecosystem supports it**; otherwise a separately pinned ESP-compatible toolchain, documented as an embedded exception in [ADR-007](docs/adr/007-esp32-rust-framework-and-toolchain.md) and verified by M9-001 |

The firmware workspace is excluded from the root workspace precisely so it can
pin a different toolchain without affecting host development. The host
workspace is **never** downgraded to match an embedded constraint.

---

## 7. What is deliberately not on this roadmap

Recorded so their absence is a decision rather than an oversight:

- **Machine learning.** The recommendation engine is rule-based and explainable.
- **N/P/K inference from EC.** Permanently out of scope — see
  [PRD 100](docs/prd/100-real-soil-sensor.md) and [PRD 140](docs/prd/140-field-readiness.md).
- **Authentication on the Edge API.** V1's boundary is the network. The first
  thing to change for any non-trusted deployment.
- **TLS on MQTT, per-device certificates, signed firmware.** M13/M14 topics.
- **Cloud-pushed configuration.** Deferred in [ADR-003](docs/adr/003-edge-first-ownership-model.md); it needs an authentication story V1 lacks.
- **A browser-hosted frontend.** The API stays CORS-capable so one remains
  possible; building it is out of V1 scope.
- **Kubernetes, Kafka, microservices, multi-region.** V1 must remain buildable
  by one developer.
- **Go, Node.js, TypeScript.** Hard constraint: this is a Rust-only project.

---

## 8. Implementation starting point

The next session begins at:

```text
M0-001 — Create repository skeleton and directory layout
```

Its `Dependencies` section is empty and every planning artefact it references
exists. See [docs/issues/M0/001-create-repository-skeleton.md](docs/issues/M0/001-create-repository-skeleton.md).
