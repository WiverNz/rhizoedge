# Rhizo Edge — Documentation Index

Everything needed to implement Rhizo Edge without rediscovering the
architecture. **M0–M3 are complete. M4 is READY; M4-001 is next.**

> An architecture pass on 2026-08-26 added device offline autonomy
> ([ADR-015](adr/015-device-offline-autonomy.md)), the per-plant binding and
> policy model ([ADR-016](adr/016-plant-binding-and-policy-model.md)), and the
> extensible measurement model
> ([ADR-017](adr/017-extensible-measurement-model.md)). MQTT v1 was revised in
> place before M1 froze it. Documents written before that date may describe the
> device as having no irrigation intelligence — ADR-015 corrects that.

**If you are here to write code, read these four, in this order:**

1. [ROADMAP.md](../ROADMAP.md) — milestones, exit criteria, conventions
2. [architecture/dependency-graph.md](architecture/dependency-graph.md) — which issue is safe to execute next
3. The PRD for your milestone — what to build and why
4. Your issue file — the step-by-step scope and acceptance criteria

Then start at **[M1-001](issues/M1/001-add-mqtt-contract-crate-skeleton.md)** —
M0 is complete.

---

## Start here

| Document | What it answers |
|---|---|
| [../README.md](../README.md) | What is this project? |
| [../ROADMAP.md](../ROADMAP.md) | What is built when, and what "done" means |
| [architecture/system-overview.md](architecture/system-overview.md) | How does it fit together? |
| [architecture/safety-invariants.md](architecture/safety-invariants.md) | What must never happen? |
| [architecture/dependency-graph.md](architecture/dependency-graph.md) | What do I implement next? |

## Source material

Historical inputs, kept for provenance. Where they conflict with the decisions
below, **the decisions win** — superseded implementation alternatives have been
normalised out of the project plan.

| Document | Role |
|---|---|
| [Rhizo_Edge_PROJECT_PLAN.md](Rhizo_Edge_PROJECT_PLAN.md) | Original product specification and intent |
| [Rhizo_Edge_Claude_Code_Planning_Prompt.md](Rhizo_Edge_Claude_Code_Planning_Prompt.md) | Brief for the planning phase |
| [Rhizo_Edge_Claude_Code_Implementation_Prompt.md](Rhizo_Edge_Claude_Code_Implementation_Prompt.md) | Brief for the implementation phase |

## Architecture

| Document | Contents |
|---|---|
| [system-overview.md](architecture/system-overview.md) | Components, trust and authority, why edge-first |
| [component-model.md](architecture/component-model.md) | Per-component responsibilities, interfaces, and prohibitions |
| [data-flow.md](architecture/data-flow.md) | Ingestion, control, and cloud pipelines with transaction boundaries |
| [deployment-model.md](architecture/deployment-model.md) | Dev, home, and future topologies; sizing and retention |
| [safety-invariants.md](architecture/safety-invariants.md) | **SAFETY-001…024** — rationale, enforcement, tests, milestone |
| [failure-model.md](architecture/failure-model.md) | Every failure: detection, expected state, recovery, safety behaviour |
| [time-model.md](architecture/time-model.md) | Clock authority, staleness, TTL, the rolling 24-hour window |
| [configuration-model.md](architecture/configuration-model.md) | Five configuration layers and who may change what |
| [dependency-graph.md](architecture/dependency-graph.md) | Milestone and issue execution order |
| [connectivity-modes.md](architecture/connectivity-modes.md) | Cloud offline, site offline, device isolated — and what each degrades |
| [offline-autonomy.md](architecture/offline-autonomy.md) | The offline policy model, evaluation, buffering, reconciliation |

## Architecture Decision Records

| ADR | Decision |
|---|---|
| [ADR-001](adr/001-rust-workspace-and-crate-boundaries.md) | Three workspaces, crate boundaries, Rust 1.98.0 |
| [ADR-002](adr/002-mqtt-topic-versioning-and-qos.md) | Topic hierarchy, QoS 1, retention, clean sessions |
| [ADR-003](adr/003-edge-first-ownership-model.md) | Edge owns truth; cloud is an append-only replica |
| [ADR-004](adr/004-sqlite-edge-persistence-model.md) | SQLite via `sqlx`; the dedup-and-persist transaction |
| [ADR-005](adr/005-cloud-event-model-and-idempotency.md) | Event ledger, `(edge_id, event_id)` idempotency, projections |
| [ADR-006](adr/006-irrigation-state-machine-ownership.md) | Pure state machine; edge decides, device vetoes |
| [ADR-007](adr/007-esp32-rust-framework-and-toolchain.md) | ESP32-C3 + `esp-idf-svc`; the embedded toolchain exception |
| [ADR-008](adr/008-shared-code-simulator-and-firmware.md) | One shared crate, one shared validator, shared fixtures |
| [ADR-009](adr/009-ui-architecture-and-rust-web-stack.md) | Tauri 2 + Leptos desktop app; no Node.js |
| [ADR-010](adr/010-observability-strategy.md) | `tracing`, metric catalogue, health endpoints |
| [ADR-011](adr/011-configuration-and-secrets-model.md) | Five config layers; secrets; hard limits unreachable |
| [ADR-012](adr/012-device-identity-and-provisioning.md) | Device ID grammar, per-device credentials, ACLs |
| [ADR-013](adr/013-clock-and-time-semantics.md) | Edge clock authority; unsynced device refuses commands |
| [ADR-014](adr/014-failure-and-retry-policy.md) | Transient/Permanent/Fatal; full-jitter backoff |
| [ADR-015](adr/015-device-offline-autonomy.md) | A provisioned device may water while isolated |
| [ADR-016](adr/016-plant-binding-and-policy-model.md) | Per-plant bindings, roles, thresholds; optional actuator |
| [ADR-017](adr/017-extensible-measurement-model.md) | Typed measurement kinds; batched telemetry; narrow table |
| [ADR-018](adr/018-battery-and-deep-sleep-device-mode.md) | Battery devices sleep; announced bounded wake windows; commands held as Edge-side intents |
| [ADR-019](adr/019-per-plant-adaptive-water-model.md) | A per-plant learned water model that may narrow a dose and never widen one |
| [ADR-020](adr/020-verified-watering-and-delivery-evidence.md) | Measured delivery evidence, an explicit outcome taxonomy, and a reservoir scale rather than a flow meter |

## Product requirements

One PRD per milestone.

| PRD | Milestone | Subject |
|---|---|---|
| [000](prd/000-platform-foundation.md) | M0 | Platform foundation |
| [010](prd/010-domain-and-mqtt-protocol.md) | M1 | Domain model and MQTT protocol |
| [020](prd/020-device-simulator.md) | M2 | Device simulator |
| [030](prd/030-edge-ingestion-and-storage.md) | M3 | Edge ingestion and storage |
| [040](prd/040-device-registry-and-health.md) | M4 | Device registry and health |
| [050](prd/050-plant-model-and-recommendations.md) | M5 | Plant model and recommendations |
| [060](prd/060-irrigation-control-and-safety.md) | M6 | **Irrigation control and safety** |
| [070](prd/070-cloud-sync-and-storage.md) | M7 | Cloud sync and storage |
| [080](prd/080-end-to-end-test-environment.md) | M8 | End-to-end test environment |
| [090](prd/090-esp32-rust-firmware.md) | M9 | ESP32 Rust firmware |
| [100](prd/100-real-soil-sensor.md) | M10 | Real soil sensor |
| [110](prd/110-real-pump-and-safety-hardware.md) | M11 | Real pump and safety hardware |
| [120](prd/120-rust-ui.md) | M12 | Rust UI |
| [130](prd/130-multi-plant-home.md) | M13 | Multi-plant home system |
| [140](prd/140-field-readiness.md) | M14 | Field readiness architecture |
| [150](prd/150-per-plant-adaptive-water-model.md) | M15 | Per-plant adaptive water model |
| [160](prd/160-verified-watering.md) | M16 | Verified watering |

## Protocol and interfaces

| Document | Contents |
|---|---|
| [mqtt-v1.md](protocol/mqtt-v1.md) | **Normative** wire specification — topics, envelope, payloads, dedup, the ordered command validation |
| [http-api-boundaries.md](protocol/http-api-boundaries.md) | Edge and Cloud REST APIs, and what neither may do |
| [versioning-policy.md](protocol/versioning-policy.md) | What may change within v1 and what forces v2 |

## Testing

| Document | Contents |
|---|---|
| [strategy.md](testing/strategy.md) | The test pyramid, naming, CI gates, what is deliberately not tested |
| [failure-scenarios.md](testing/failure-scenarios.md) | **SCEN-001…107** — the executable scenario catalogue and its invariant coverage matrix |
| [simulator-strategy.md](testing/simulator-strategy.md) | Physical model, fault catalogue, the permissiveness rule |
| [hardware-in-the-loop.md](testing/hardware-in-the-loop.md) | HIL-1…HIL-7 gated checklists for real hardware |
| [local-development.md](testing/local-development.md) | Running, debugging, inspecting, common problems |

## Hardware

| Document | Contents |
|---|---|
| [home-node-hardware-guide.md](hardware/home-node-hardware-guide.md) | Milestone-organized BOM, wiring, measurement, and installation guide — bench bring-up on the official Espressif ESP32-C3-DEVKITM-1-N4X through battery and optional solar deployment |

Practical procurement and assembly guidance, and the only document here that
quotes prices. **It is not normative**: parts, ratings, and values are starting
points to be measured, while required behaviour stays in the ADRs, PRDs,
[safety invariants](architecture/safety-invariants.md), and
[MQTT v1](protocol/mqtt-v1.md). Where it names a board, the binding rule is
[ADR-007](adr/007-esp32-rust-framework-and-toolchain.md): ESP32-C3 is committed,
the board is a compile-time profile.

## Implementation issues

286 issues across 17 milestones, each with context, scope, dependencies,
acceptance criteria, and verification commands.

| Milestone | Issues | Subject |
|---|---|---|
| [M0](issues/M0/) | 13 | Foundation and engineering baseline |
| [M1](issues/M1/) | 19 | Domain model and MQTT protocol |
| [M2](issues/M2/) | 19 | Device simulator |
| [M3](issues/M3/) | 18 | Edge ingestion and SQLite |
| [M4](issues/M4/) | 13 | Device registry and health |
| [M5](issues/M5/) | 22 | Plant model and recommendations |
| [M6](issues/M6/) | 24 | Irrigation control and safety |
| [M7](issues/M7/) | 15 | Cloud API and PostgreSQL |
| [M8](issues/M8/) | 18 | End-to-end test environment |
| [M9](issues/M9/) | 22 | ESP32 Rust firmware |
| [M10](issues/M10/) | 13 | Real soil sensor |
| [M11](issues/M11/) | 14 | Real pump and safety hardware |
| [M12](issues/M12/) | 19 | Rust UI |
| [M13](issues/M13/) | 17 | Multi-plant home system |
| [M14](issues/M14/) | 10 | Field readiness architecture |
| [M15](issues/M15/) | 14 | Per-plant adaptive water model |
| [M16](issues/M16/) | 16 | Verified watering |

Issue numbering within a milestone is a valid execution order: every issue's
dependencies have lower numbers in the same milestone, or belong to an earlier
one. The [dependency graph](architecture/dependency-graph.md) shows where that
order can safely be widened into parallel work.

## Validation

```bash
cargo run -p rhizo-docscheck
```

`rhizo-docscheck` is a dependency-free Rust tool that verifies the planning
artefacts are internally consistent: required files exist, identifiers are
unique, every `M*-*` / `ADR-*` / `PRD *` / `SAFETY-*` / `SCEN-*` reference
resolves, relative links resolve, the issue dependency graph is acyclic, and
issue numbering is a valid execution order.

It is planning tooling, not a product crate — never shipped, and never a
dependency of a runtime crate. M0-011 adopted it into the root workspace, which
is why the command above is `-p rhizo-docscheck` rather than a `--manifest-path`.

## Conventions

| Kind | Form | Example |
|---|---|---|
| Milestone | `M<n>` | `M6` |
| Issue | `M<n>-NNN` | `M6-009` |
| ADR | `ADR-NNN` | `ADR-006` |
| PRD | `PRD NNN` | `PRD 060` |
| Safety invariant | `SAFETY-NNN` | `SAFETY-006` |
| Test scenario | `SCEN-NNN` | `SCEN-040` |
| Functional requirement | `F-NNN-NN` | `F-060-20` |

Safety tests are named `safety_NNN_<description>`, so `cargo test safety_` runs
the entire safety suite.

Full conventions, including issue sizing and the definition of milestone
completion: [ROADMAP.md §5](../ROADMAP.md#5-planning-conventions).
