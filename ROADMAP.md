# Rhizo Edge — Roadmap

The execution plan for Rhizo Edge, from an empty repository to a system that can
be trusted with a real plant.

**Planning status:** complete. **Implementation status:** M0, M1, M2, and M3 complete; M4 next.
**Host Rust:** MSRV **1.98.0**; `rust-toolchain.toml` currently pins 1.98.0; the
pin may move forward deliberately
([ADR-001](docs/adr/001-rust-workspace-and-crate-boundaries.md) §Rust version policy).

> **Architecture pass, 2026-08-26 — after M0, before M1.** Requirements expanded
> in ways that had to land before M1 froze the MQTT and domain contracts:
> **true device offline autonomy** ([ADR-015](docs/adr/015-device-offline-autonomy.md)),
> a **per-plant binding and policy model**
> ([ADR-016](docs/adr/016-plant-binding-and-policy-model.md)), and an
> **extensible typed measurement model**
> ([ADR-017](docs/adr/017-extensible-measurement-model.md)). MQTT v1 was revised
> in place because nothing had ever spoken it
> ([versioning-policy.md](docs/protocol/versioning-policy.md) §0). Eight
> invariants were appended as SAFETY-013…020; the original twelve are unchanged
> and were never renumbered. **M0 was not reopened.**

- Source of truth for *what* each milestone builds: [docs/prd/](docs/prd/)
- Source of truth for *why*: [docs/adr/](docs/adr/)
- Source of truth for *how, step by step*: [docs/issues/](docs/issues/)
- Execution order: [docs/architecture/dependency-graph.md](docs/architecture/dependency-graph.md)

---

## 1. Milestone table

| ID | Name | Objective | Depends on | Issues | Status |
|---|---|---|---|---|---|
| M0 | Foundation and Engineering Baseline | A clean Rust repository whose tooling, lint, test, container, and observability baseline every later milestone inherits | — | 13 | **DONE** |
| M1 | Domain Model and MQTT Protocol | The shared wire contract, typed measurement kinds, capability and offline-policy payloads, and the pure domain and policy crates | M0 | 19 | **DONE** |
| M2 | Device Simulator | A host device indistinguishable from firmware at the protocol/mechanics level, including offline-policy persistence, isolation/replay mechanics, fault injection, and virtual time; policy evaluation activates in M6 | M1 | 19 | **DONE** |
| M3 | Edge Ingestion and SQLite | Reliable MQTT consumption with durable deduplication and crash-safe persistence | M1, M2 | 18 | **DONE** |
| M4 | Device Registry and Health | Device lifecycle, staleness, sensor health, config drift, and the first REST surface | M3 | 13 | **READY** |
| M5 | Plant Model and Recommendations | Plants, **bindings, per-measurement thresholds**, offline-policy authoring, trends, and an explainable recommendation engine — **issuing no commands** | M4 | 17 | **READY** |
| M6 | Irrigation Control and Safety | The state machine, the safety gate, the command lifecycle, the **offline evaluator and reconciliation**, and every non-hardware SAFETY invariant | M5, M2 | 22 | **READY** |
| M7 | Cloud API and PostgreSQL | Optional idempotent history sync that cannot affect local safety | M6 | 15 | **READY** |
| M8 | End-to-End Test Environment | The whole software system reproducible and verifiable with one command, no hardware | M7 | 17 | **READY** |
| M9 | ESP32 Rust Firmware Foundation | Real firmware speaking the same protocol, with fake sensors and pump, **plus the persisted offline policy, evaluator, event buffer, and monotonic budget** | M8 | 19 | PLANNED |
| M10 | Real Soil Sensor Integration | Real readings behind the unchanged `SoilSensor` trait | M9 | 11 | PLANNED |
| M11 | Real Pump and Safety Hardware | Real actuation with calibration and physically verified lockouts | M10 | 14 | PLANNED |
| M12 | Rust UI | A Tauri 2 + Leptos desktop client that structurally cannot bypass safety | M6 (functional), M11 (full picture) | 17 | PLANNED |
| M13 | Multi-Plant Home System | Several nodes, provisioning tooling, notifications, a supportable deployment, **release binary CI, the MSRV matrix, and the optional Grafana profile** | M12 | 16 | PLANNED |
| M14 | Field Readiness Architecture | Architecture and honest constraints for greenhouse and field, **plus optional Helm packaging and the future actuator model** — **documentation only** | M13 | 9 | PLANNED |

**Total: 239 issues.**

### Status semantics

| Status | Meaning |
|---|---|
| `READY` | Fully specified; every issue written; no unresolved external dependency. Executable as soon as its predecessor completes. |
| `PLANNED` | Fully specified, but carries an unresolved external dependency — hardware to purchase, a toolchain to verify on a real machine, or a UI stack version to pin. |
| `BLOCKED` | A dependency is unmet or a decision is outstanding. None currently. |
| `IN PROGRESS` | Implementation started. |
| `DONE` | Every issue closed **and** the milestone's exit criteria demonstrably met. |

**M0 is `DONE`** — implemented, verified, and committed. It was not reopened by
the 2026-08-26 architecture pass; the new requirements land in M1 and later.

M2–M8 are `READY`: they are pure software, need no hardware, and every
prerequisite is a preceding milestone. M9–M11 are `PLANNED` because they depend
on physical hardware and on ADR-007's toolchain being executed on a real machine
(M9-001). M12–M13 are `PLANNED` pending pinned Tauri/Leptos versions. M14 is
`PLANNED` and produces documentation only.

---

## 2. Milestone detail

### M0 — Foundation and Engineering Baseline · DONE

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

### M1 — Domain Model and MQTT Protocol · DONE

**Objective.** Define the contract that cannot be changed cheaply once devices
exist in pots.

**Deliverables.** `rhizo-mqtt-contract` (`no_std` + `alloc`) implementing
[mqtt-v1.md](docs/protocol/mqtt-v1.md) · `DeviceId` grammar · `UtcMillis` ·
envelope with identity checking · topic grammar · **all MQTT v1 message and
payload types defined by the normative protocol** (§5.2–§5.12: batched typed
telemetry, actuator state, device events, status with declared capabilities, LWT,
config, the three command kinds, command results, the offline policy, and
`edge.time`) · the `MeasurementKind` enum with its `const fn spec()` · the
`TimeSyncState` strict-acceptance helper · **`validate_water_command`, the single
shared actuation gate** · protocol fixture corpus · `rhizo-domain` skeleton with
the `Clock` trait · `no_std` CI verification · clippy ban on direct clock access
in the domain.

**Exit criteria.** Every clause of mqtt-v1.md §2–§10 implemented or explicitly
noted, checked against the §11 conformance checklist. `validate_water_command`'s
ordering matches §5.8 exactly. `TimeSyncState` accepts only a **strictly** newer
`edge_time_ms` (§5.12). The contract crate builds for a bare-metal target with
default features off. `Utc::now()` in `rhizo-domain` fails clippy. Every fixture
behaves as documented: each valid file decodes into its **concrete payload type**
and survives a re-encode without losing a stated field, and each invalid file
fails with its **named** typed variant.

**Invariants.** Delivers the mechanism for SAFETY-002, SAFETY-007, SAFETY-012;
enforcement tested in M6.

**PRD.** [010](docs/prd/010-domain-and-mqtt-protocol.md)

---

### M2 — Device Simulator · DONE

**Objective.** A reference device that makes M3–M8 achievable without hardware —
and that is **never more permissive than firmware**.

**Deliverables.** Full protocol conformance including LWT and retained status ·
**declared sensor and actuator capabilities**, so the edge assumes nothing ·
batched typed measurements across the kinds the simulated hardware exposes · soil
model with absorption lag, probe overshoot, and drainage · weight rising
immediately while VWC lags · tank, leak, EC models · **actuation exclusively
through `validate_water_command`** · **wall clock maintained solely from
`edge.time`**, with strict duplicate rejection · NVS-equivalent persistent state ·
**a persisted, versioned offline policy applied atomically** · isolation and
reconnect state plus persisted monotonic evaluator inputs, but **no offline
decision or autonomous dosing until M6-019 installs the single shared
`rhizo_policy::evaluate_offline` call site** · **a bounded offline event buffer with
ordered replay and explicit gap reporting** · control API · thirteen injectable
faults · accelerated virtual time.

**Exit criteria — met, with evidence in
[docs/reports/M2.md](docs/reports/M2.md).** Runs standalone against a bare broker
(`docker compose --profile devices up mosquitto device-simulator`; telemetry read
back with `mosquitto_sub`). Exactly one call site of `validate_water_command` and
**no offline evaluator or autonomous-dose call site yet**, both checked
structurally rather than asserted.
`requested_ml: 10000` published directly to the broker delivers 80 ml — the
compile-time limit — and the reservoir confirms it. No retained messages on
command, telemetry, event, or `time` topics, verified after a full command cycle
by a fresh subscriber, with both negative controls run and reverted.
A replayed `edge.time` never extends `clock_synced`. An isolated simulator keeps
sampling and buffering through two simulated days on a bone-dry plant with an
enabled policy and never actuates. ACL isolation holds, and an over-permissive
ACL pattern fails the test. A full commanded cycle completes in **1.4 s** at
scale 600.

**Invariants.** Makes SAFETY-002, SAFETY-007, SAFETY-011, policy activation, and
buffer/replay mechanics testable before hardware exists — each now with a green
test in `crates/device-simulator`. M6-019/M6-021 provide the first executable
autonomous-device evaluation for SAFETY-013…017.

**PRD.** [020](docs/prd/020-device-simulator.md)

---

### M3 — Edge Ingestion and SQLite · DONE

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

**Deliverables.** Plant CRUD with `auto_watering_enabled` defaulting false ·
**explicit `SensorBinding[]` with `control`/`required`/`advisory` roles, bound to
declared device capabilities** · **an optional `ActuatorBinding` (0..1), whose
absence is a normal monitoring plant, not a degraded one** · **per-measurement
policies: target band, warning and critical thresholds, staleness, hysteresis,
confirmation duration** · **threshold evaluation raising alerts that never cause
watering** · **authoring and validating the per-plant offline policy the device
may later act on** · profile validation that **rejects rather than clamps**, with
the profile demoted to a template that pre-populates policies · least-squares
moisture trend returning `None` on sparse data · dry-duration tracking with gap
handling · manual-watering detection with command attribution · stuck-sensor
detection · rule-based recommendation with typed reasons · plant state
derivation · EC trend and warning · evaluation tick and endpoints.

**Exit criteria.** Drying produces `WaterRecommended` with a non-empty reason
list and **zero MQTT commands published**. A plant with no `ActuatorBinding`
returns **422**, not 409, from every actuation path, and still receives telemetry,
thresholds, and alerts. A binding to a capability the device never declared is
rejected. A critical temperature raises an alert and waters nothing. A profile
with `dose_ml = 200` is rejected with 422 naming the firmware limit. A moisture
step following a command creates no second event.

**Invariants.** Enforces SAFETY-018 (no actuator binding ⇒ no actuation path).
Otherwise builds the gate's inputs without actuating.

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
lockout endpoints with **no override parameter** · no-delivery detection ·
**`rhizo_policy::evaluate_offline`, the single shared offline evaluator — pure,
`no_std`, taking elapsed time as a parameter and reading no clock** ·
**reconciliation of offline actions on reconnect, idempotent by `event_id`, with
the plant held `Uncertain` and no dose issued until replay completes** · the full
property-test suite, including the offline safety properties.

**Exit criteria.** `cargo test safety_` fully green. `PROPTEST_CASES=10000 cargo
test safety_006` passes. `POST /water` during a leak returns 409 with nothing
published. Restart after publish produces no second command and one watering
event. Replaying a buffered offline batch three times out of order produces one
`watering_event` per `event_id` and one budget charge. No `_ =>` arm on any
safety match.

**Invariants enforced.** SAFETY-001, -002, -003, -004, -005, -006, -007 (via the
shared validator), -010, -012, and — against the simulator as the reference
device — SAFETY-013, -014, -015, -016, -017. SAFETY-019 and SAFETY-020 are
firmware-owned and land in M9.

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
`main`** · Wi-Fi · MQTT with LWT · **wall clock synchronised from `edge.time`
over that same MQTT connection — no SNTP client — with strictly-increasing
acceptance** · telemetry · command handling through the shared validator with an
NVS dedup ring · config handling · **an NVS-persisted, versioned offline policy
activated atomically, where a bad update never replaces a good one** ·
**integration of the shared `rhizo_policy::evaluate_offline`, called from exactly
one place** · **a bounded offline event buffer with audit and telemetry tiers,
ordered replay, and explicit gap markers** · **monotonic budget and cooldown state
persisted across reboot, never replenished by a restart** · interrupted-dose
reporting · serial provisioning.

**Exit criteria.** Builds for the ESP target with no board. Conformance test
shows identical behaviour to the simulator, including the offline evaluator.
**With a board:** HIL-1 passes on a multimeter across 20 resets; a duplicate
`command_id` survives a power cycle; withholding `edge.time` refuses commands
while telemetry continues; power-cycling mid-cooldown neither shortens the
cooldown nor replenishes the budget.

**Invariants enforced.** SAFETY-011 (firmware); SAFETY-002 and SAFETY-007 on
real silicon; SAFETY-001 gains its device-side enforcement point; SAFETY-013,
-015, -019, -020 on the device that actually owns them.

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
**a binding editor offering only capabilities the device actually declared, and
presenting a plant with no actuator binding as a first-class monitoring plant** ·
**per-measurement threshold configuration with warning and critical bands** ·
**connectivity views distinguishing cloud offline, site offline, and device
isolated, showing the applied offline-policy version and whether autonomous
control is active** · **offline history showing autonomously delivered doses with
`origin: offline_autonomous` and any reported gaps** · watering actions with **no
override control** · inline SVG charts with the target band and watering markers ·
profile editor · connection-state handling that never shows a blank screen ·
packaging with WebView2 bootstrap · CI job asserting no Node artefacts.

**Exit criteria.** Builds on Windows and Linux. No JS toolchain anywhere. A leak
lockout is prominent with no clear button and manual watering shows the reason.
A monitoring-only plant shows no watering control at all rather than a disabled
one. An isolated device's autonomous doses appear in history, attributed, once
reconciled. Stopping the edge shows greyed last-known data with its age.

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
scale · **release CI publishing checksummed binaries from a `v*` tag** · **a CI
matrix building on the MSRV 1.98.0 and on current stable, so the MSRV cannot rise
by accident** · **an opt-in `observability` Compose profile adding Prometheus and
Grafana, which nothing depends on**.

**Exit criteria.** SCEN-080 shows byte-identical state for unaffected plants.
Provisioning refuses reuse without `--force`. A dead notification channel does
not delay the control loop. The system survives a Pi reboot. Backup and restore
reproduce identical watering history. 20 plants evaluate within one tick. A tag
produces downloadable archives whose `--version` matches. The MSRV job fails on
an accidental bump and names ADR-001. Everything works with the observability
profile disabled.

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
requirements stated plainly · **optional Helm packaging specified for server-side
components only, with the plant-side edge controller explicitly out of scope** ·
**the future actuator capability model — what `valve`, `grow_light`, `fan`,
`heater`, `humidifier`, and `fertiliser_dosing_pump` would each require, and which
need a different automation model rather than an extension**.

**Exit criteria.** Every reservation verified against code. **`git diff` shows no
speculative implementation** — no chart, no actuator kind, no zone table.
Kubernetes remains absent from the product architecture. Open questions recorded
as genuinely unresolved.

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
| SAFETY-012 | Uncertainty defaults to no watering | Edge domain + device | M6 | M8 |
| SAFETY-013 | Autonomous action needs a validated persisted policy | Device | M6 (sim), M9 (fw) | M8 |
| SAFETY-014 | Offline doses obey the same caps and hard limits | Device + Edge | M6 | M8, M11 |
| SAFETY-015 | Clock uncertainty never grants budget or shortens cooldown | Device | M6 (sim), M9 (fw) | M8 |
| SAFETY-016 | Offline actions reconcile exactly once | Edge + Device | M6 (edge), M9 (device) | M8 |
| SAFETY-017 | Missing/stale required measurement blocks autonomous action | Device | M6 | M8 |
| SAFETY-018 | A plant with no actuator has no actuation path | Edge | M5 | M8, M12 |
| SAFETY-019 | Policy activation is atomic | Device | M9 | M8 |
| SAFETY-020 | Lost buffered history is reported as an explicit gap | Device + Edge | M9 | M8 |

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
cargo run -p rhizo-docscheck                      # planning-artefact validator
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

```text
MSRV                 1.98.0    the minimum host Rust the project supports
current tested pin   1.98.0    what rust-toolchain.toml selects today
future pin           may move to any newer stable, deliberately
```

- **MSRV is 1.98.0.** The host workspace and the UI must keep compiling on it.
- **The pin may be raised** to a newer stable as a standalone change. Raising the
  pin does not by itself raise the MSRV.
- **No change may silently raise the MSRV.** Using a feature stabilised after
  1.98.0 requires an explicit decision and an update to ADR-001, README, and this
  section.
- **Nothing is downgraded below 1.98.0**, including to match an embedded
  constraint.
- M13-014 adds a CI matrix verifying **both** the MSRV and current stable, so an
  accidental bump fails the build rather than reaching a user.

| Component | Toolchain |
|---|---|
| Host workspace (`crates/*`) | MSRV **1.98.0**; currently pinned to 1.98.0 in `rust-toolchain.toml` |
| UI workspace (`ui/rhizo-ui`) | same version plus the `wasm32-unknown-unknown` target |
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
- **Grafana as a required component.** It is an optional M13 deployment profile
  ([ADR-010](docs/adr/010-observability-strategy.md)); nothing depends on it, and
  it is not how a normal user learns whether a plant is safe.
- **Kubernetes for the plant-side edge.** Helm packaging for server-side
  components is planned in M14-007; the edge controller is explicitly excluded,
  because the component whose purpose is working when things fail should not gain
  a scheduler's failure modes.
- **A generic automation framework.** Future actuator kinds are reserved in the
  protocol with no implementation and no automation semantics.
- **Kubernetes, Kafka, microservices, multi-region.** V1 must remain buildable
  by one developer.
- **Go, Node.js, TypeScript.** Hard constraint: this is a Rust-only project.

---

## 8. Implementation starting point

**M0, M1, M2, and M3 are `DONE`.** The next unstarted issue is:

```text
M4-001 — Implement device status ingestion
```

It depends on M3-018, which is complete, so it is executable now. See
[docs/issues/M4/001-implement-device-status-ingestion.md](docs/issues/M4/001-implement-device-status-ingestion.md)
and [docs/architecture/dependency-graph.md](docs/architecture/dependency-graph.md).

This pointer must move with the milestone table above; `rhizo-docscheck` fails
the build if it names an issue from a milestone already marked `DONE`.
