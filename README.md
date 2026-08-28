![Rhizo Edge — a seedling with soil sensor, drip emitter and wireless link, beside the Rhizo Edge wordmark](preview.png)

# Rhizo Edge

An **offline-first Rust platform for plant monitoring and fail-safe automated
irrigation**, using MQTT, local edge processing, ESP32 devices, and optional
cloud synchronisation.

> **Status: M0, M1, M2, and M3 complete. M4 ready.**
> **Unless explicitly marked as implemented, the sections below describe the
> planned target architecture.**
>
> The engineering baseline, shared contracts/domain, device simulator, and Edge
> ingestion/SQLite foundation are implemented and green. M3 delivered the
> supervised `edge-controller`, real-Mosquitto ingestion with reconnect and
> re-subscription, edge-stamped receipt time, bounded quarantine, crash-safe
> deduplication, typed persistence, replay acknowledgement, and retention.
> Device registry and health behavior remains M4 work.
> Start at [ROADMAP.md](ROADMAP.md); the next issue is
> [M4-001](docs/issues/M4/001-implement-device-status-ingestion.md).

---

## What it is

Rhizo Edge measures the conditions a plant actually lives in, decides whether it
needs water, and delivers a bounded dose through a pump. It is designed so that
loss of Internet, cloud, broker, Wi-Fi, or power does not turn a connectivity
failure into unsafe watering.

The first target is indoor houseplants. The architecture is deliberately shaped
so greenhouse and field deployments are an extension of the same system rather
than a rewrite.

## Three principles

**Edge-first.** While connected, the Edge Controller owns high-level irrigation
decisions and local authoritative state. When a device becomes isolated, it may
execute only a previously validated, persisted offline policy with a
deliberately restricted rule set. The cloud is an append-only history sink,
optional by default (`cloud.enabled = false`), and can vanish for a week without
affecting a single watering decision — enforced structurally, not by discipline:
the domain crate cannot depend on the cloud client, and the type carrying every
watering input has no cloud-derived field.

**Safety-first.** When any input is missing, stale, invalid, or contradictory,
the answer is *do not water*, plus a visible lockout — never *water anyway*.
Twenty numbered invariants
([SAFETY-001…020](docs/architecture/safety-invariants.md)) state this precisely,
each with named automated tests and the milestone where it becomes enforced.
From M6 onward, `cargo test safety_` is the executable safety-invariant gate.

**Offline-capable at every layer.** A device that loses Wi-Fi is not a device
that stops caring for its plant. See the next section — this is the part most
comparable projects get wrong.

Defence in depth throughout. While connected, the edge decides *whether* and
*how much*; the device independently decides whether it is **safe to obey**.
When isolated, the device may evaluate only its provisioned offline policy,
while the same firmware hard limits remain authoritative. Those limits are
compiled in and cannot be raised by any message, API call, or configuration — so
even a completely wrong Edge Controller cannot flood the room.

## Working offline — three different outages

"Offline" is not one condition, and each degrades something different:

| | What broke | What still works |
|---|---|---|
| **Cloud offline** | the cloud endpoint | everything local; history queues and syncs later |
| **Site offline** | the whole internet | everything local — devices take their clock from the Edge over the MQTT connection they already have, so watering continues with no internet at all |
| **Device isolated** | that device's link to the edge | **the device keeps monitoring and, if provisioned, keeps watering on its own** |

The third case is the one that matters when you are away for two weeks and the
router reboots. A plant-side device that has been explicitly given a validated
**offline policy** continues to sample its sensors, evaluate the policy, and
deliver bounded doses — then reports everything that happened once the link
returns.

Crucially, it does **not** improvise:

- It acts only from a policy the Edge authored, validated, versioned, and that
  the device persisted and activated atomically. No policy, an invalid policy, or
  a missing required sensor all mean **no watering**.
- It runs a deliberately restricted rule set — threshold, confirmation duration,
  hysteresis, cooldown, a fixed dose, bounded dose count, rolling volume cap —
  never the Edge's full recommendation engine.
- Every safety veto still applies: leak, empty or unknown tank, faulted pump,
  firmware hard limits.
- Offline doses count against the **same** daily budget as commanded doses, and
  replay to the edge exactly once, so nothing is watered twice across the seam.

Details: [connectivity modes](docs/architecture/connectivity-modes.md) ·
[offline autonomy](docs/architecture/offline-autonomy.md) ·
[ADR-015](docs/adr/015-device-offline-autonomy.md)

## Configuration flows down, versioned

Devices are not configured by reflashing them. The Edge Controller owns their
settings and publishes them as **retained, versioned MQTT messages**, so a device
that boots three days later still receives current desired state:

| Channel | Carries | Example |
|---|---|---|
| `config` | device runtime parameters | telemetry interval, pump calibration (`ml_per_second`), tank minimum, which sensors are enabled |
| `policy` | per-plant offline automation rules | thresholds, dose size, cooldown, required sensors, enable/disable |

Both are validated before publication, re-validated by the device, applied
atomically, and acknowledged — the device echoes the version it is actually
running, so configuration drift is visible rather than silent. A policy that
fails validation never replaces the working one.

Adjusting a plant's watering threshold, changing how often a node reports, or
recalibrating a pump is an API call, not a firmware build.

**What configuration can never do** is raise a firmware hard limit. Maximum run
duration, maximum millilitres per dose, and maximum daily volume are compile-time
constants with no representation in any message. Changing them requires
reflashing the device, deliberately.

Details: [configuration model](docs/architecture/configuration-model.md) ·
[ADR-011](docs/adr/011-configuration-and-secrets-model.md)

## Sensors: an open, typed set

Plants care about more than soil moisture, so the measurement model is
extensible without becoming untyped:

```text
soil_moisture        soil_temperature      soil_ec        soil_ph
ambient_temperature  ambient_humidity      illuminance
pot_weight           tank_level            leak_state
nitrate_concentration
```

Each kind has exactly one canonical unit and a physical plausibility range,
declared once as compile-time data the firmware itself can check against.

Adding a measurement kind — PAR/PPFD, CO₂, whatever a future probe measures —
does not require a new MQTT topic or a database-schema redesign. Devices that
actually support a new physical sensor may still require a firmware update for
the corresponding driver and capability declaration.

Three consequences worth knowing:

- **A device declares what it has.** The edge never assumes a node is a pump
  controller; a plant can be bound to a soil probe on one device and a shared
  room temperature and light sensor on another.
- **Thresholds belong to the plant, not the sensor.** The same room sensor is
  "fine" for a succulent and "critical" for a fern, so warning and critical bands
  are configured per plant, per measurement.
- **A warning is not a command.** A critical temperature or a light level below
  target raises a visible alert; it never causes watering. Alerts work on plants
  that have no pump at all.

**EC is EC.** There is deliberately no measurement kind for nitrogen, phosphorus,
or potassium: cheap "NPK" probes derive those from conductivity by an undisclosed
formula, and presenting them as nutrient measurements would be a false claim
about a real plant. A genuinely calibrated ion sensor can report a real value,
with its calibration reference attached.

Details: [ADR-017](docs/adr/017-extensible-measurement-model.md) ·
[ADR-016](docs/adr/016-plant-binding-and-policy-model.md)

## Species presets: a starting point, not an authority

Thresholds belonging to the plant is the right design, and it means configuring
a new plant begins with an operator inventing a moisture band for a species they
bought yesterday. **Plant presets** fill that in: pick "Rose", "Monstera", or
"Basil" and the per-measurement policy is prefilled — then reviewed and edited
before anything is written.

Three rules stop a convenience from becoming an authority:

- **A preset is not a schedule.** It stores what a species prefers — a moisture
  band, a light preference, temperature and humidity ranges, pH, a dose and
  cooldown class — and never "water every 2 days". Watering remains a function
  of measurements and the safety gate. A timer would be a second actuation
  authority that no sensor reading and no lockout could contradict.
- **A preset is a template, exactly as `PlantProfile` is.** Applying one writes
  ordinary per-plant threshold rows and then stops mattering: every value stays
  editable, nothing is silently re-derived later, and a preset value that would
  exceed a firmware hard limit is rejected rather than clamped. A curated
  catalogue is an input, not a trusted one.
- **A stated fact and a derived guess are labelled differently.** Where a source
  says a species likes "moderate" soil moisture, the volumetric figure Rhizo
  would target is an interpretation with a unit conversion inside it. The
  interface says which is which, in words — that sentence is what invites an
  operator to correct a number they know better than the catalogue does.

The catalogue is curated, versioned, and compiled into the binary, so creating a
plant is not the one operation that needs the Internet. External species
databases may be used to research and import entries offline, with human review
and a verified licence — never as a runtime dependency.

Configuring a plant entirely by hand remains an equal path, not a fallback.

Planned for M5 (catalogue and application) and M12 (picker and review step).

## The pump is optional

Most plants in a real home will never have one. A plant with no actuator is a
**first-class, fully supported** configuration — telemetry, history, trends,
thresholds, warnings, critical alerts, recommendations, and UI visibility — that
simply has no actuation path. It is not a plant with a missing part.

Supported shapes, all equally normal:

```text
monitoring only
monitoring + recommendation
monitoring + manual remote watering
monitoring + connected automatic watering
monitoring + offline autonomous watering
```

## Architecture

```text
  ESP32 Plant Node (Rust)              Device Simulator (Rust)
  ┌──────────────────────────┐         ┌──────────────────────────┐
  │ sensors: soil · ambient  │         │ same MQTT protocol       │
  │   light · tank · leak …  │         │ shared evaluator (M6+)   │
  │ pump (optional)          │         │ virtual time · faults    │
  │ offline policy + budget  │         │ offline policy + budget  │
  │ HARD SAFETY LIMITS       │         │ HARD SAFETY LIMITS       │
  └────────────┬─────────────┘         └────────────┬─────────────┘
               └──────────── MQTT (QoS 1) ──────────┘
                    │  ▲ telemetry · events · results
                    │  │ ▼ config · policy · commands
               Mosquitto
                    │
      ┌─────────────▼──────────────────────────┐
      │ Edge Controller (Rust)                 │
      │  ingest → validate → dedup             │
      │  device registry · capabilities        │
      │  plant bindings · thresholds · alerts  │
      │  recommendations                       │
      │  irrigation state machine              │
      │  SAFETY GATE                           │
      │  offline policy authoring              │
      │  reconciliation of offline history     │
      │  local REST API · metrics · time sync  │
      └───┬──────────────────────┬─────────────┘
          │                      │ HTTP (optional, outbound only)
     ┌────▼─────┐          ┌─────▼──────────────┐
     │ SQLite   │          │ Cloud API (Rust)   │
     │ source   │          │ append-only        │
     │ of truth │          │ → PostgreSQL       │
     └──────────┘          └────────────────────┘
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
| `rhizo-mqtt-contract` | `no_std` wire contract, measurement kinds, hard limits, the shared command validator |
| `rhizo-policy` | `no_std` offline-policy state and decision contract; M6-019 adds the one evaluator shared by simulator, firmware, and any required edge use |
| `rhizo-domain` | Pure M1 plant, binding, policy, state, validation, and clock abstractions; later milestones add recommendation and irrigation behaviour |
| `rhizo-storage` | SQLite schema, repositories, and the deduplicate-and-persist transaction |
| `edge-controller` | The control plane — the only component that decides while connected |
| `device-simulator` | Implemented reference device with protocol mechanics, persistence, isolation/replay, virtual time, and faults; M6 connects the shared evaluator |
| `cloud-api` | Idempotent event ingestion into PostgreSQL |
| `esp32-node` | ESP32-C3 firmware; the final hardware safety boundary and the offline fallback controller |
| `rhizo-ui` | Tauri 2 + Leptos desktop client; talks HTTP to the edge only |

## Development strategy: simulator before hardware

The Device Simulator implements the *same* MQTT protocol as the firmware and
calls the *same* command validator. M2 supplies connectivity, policy/runtime
state persistence, isolation, buffering/replay, virtual time, and fault
mechanics, but an enabled offline policy remains inert there. M6-019 then adds
the single evaluator to `rhizo-policy` and its sole simulator call site. Firmware
later calls that same implementation; there is never a simulator-specific copy.
This keeps the full control plane — including offline autonomy after M6 —
buildable and testable before electronics exist.

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

**MSRV is Rust 1.98.0**, and `rust-toolchain.toml` currently pins exactly that.
The pin may be raised to a newer stable as a deliberate change, but no change may
silently raise the MSRV, and nothing is downgraded below it. The firmware
workspace is separate and may pin an ESP-compatible toolchain if the Espressif
ecosystem requires it; that exception is isolated and documented in
[ADR-007](docs/adr/007-esp32-rust-framework-and-toolchain.md).

| Layer | Choice |
|---|---|
| Async runtime | Tokio |
| Broker | Eclipse Mosquitto, MQTT 3.1.1, QoS 1, `clean_session = true` |
| Edge storage | SQLite via `sqlx`, WAL |
| Cloud storage | PostgreSQL via `sqlx` |
| HTTP | Axum |
| Observability | `tracing` + Prometheus text format; Grafana optional |
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
RHIZO_REQUIRE_BROKER=1 cargo test --workspace --all-features
cargo build -p rhizo-mqtt-contract --no-default-features --target thumbv7em-none-eabi
cargo build -p rhizo-policy --no-default-features --target thumbv7em-none-eabi
docker compose -f deploy/docker-compose.yml config
cargo sqlx prepare --workspace --check
cargo run -p rhizo-docscheck
```

`RHIZO_REQUIRE_BROKER=1` is how CI runs the suite. Without it the broker-backed
tests print a loud skip and pass, so a fresh clone is green; with it, a missing
Mosquitto is a failure — a suite that can silently skip its own subject
eventually proves nothing.

`cargo sqlx prepare --check` verifies the committed `.sqlx/` offline cache
against the migrated schema. The cache is checked into version control on
purpose: CI has no database, so every offline build reads it instead of
connecting, and a query changed without regenerating it fails there rather than
at runtime.

A milestone is complete only when its acceptance tests are green and its exit
criteria are demonstrably met — never on the basis of closed issues alone.

## Documentation

Start here: **[docs/README.md](docs/README.md)** — the documentation index.

| | |
|---|---|
| [ROADMAP.md](ROADMAP.md) | Milestones, exit criteria, conventions |
| [Safety invariants](docs/architecture/safety-invariants.md) | SAFETY-001…020 |
| [Connectivity modes](docs/architecture/connectivity-modes.md) | Cloud offline vs site offline vs device isolated |
| [Offline autonomy](docs/architecture/offline-autonomy.md) | The offline policy model and reconciliation |
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
- **No machine learning**, and **no N/P/K inference from EC**.
- **Cloud pushes no configuration.** Deliberate; it needs an authentication
  story V1 does not have.
- **Offline autonomy is opt-in, per plant, and deliberately limited.** A device
  runs a restricted rule set, not the Edge's engine, so the two can in principle
  reach different conclusions about the same plant. Bounded by sharing the
  offline rules in one crate; not eliminated.
- **Device history is bounded.** An isolated device buffers what it can and
  reports an explicit gap for what it could not keep — audit events (doses,
  refusals, faults) outrank telemetry samples and are never dropped to make room
  for them.

## Licence

Not yet selected.
