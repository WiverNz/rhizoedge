![Rhizo Edge — a seedling with soil sensor, drip emitter and wireless link, beside the Rhizo Edge wordmark](preview.png)

# Rhizo Edge

An **offline-first Rust platform for plant monitoring and fail-safe automated
irrigation**, using MQTT, local edge processing, ESP32 devices, and optional
cloud synchronisation.

> **Status: M0 through M7 complete. M8 ready, and not started.**
> **Unless explicitly marked as implemented, the sections below describe the
> planned target architecture.**
>
> Implemented and green: the engineering baseline, the shared wire contract and
> pure domain, the device simulator, Edge ingestion and SQLite, the device
> registry and health model, the whole plant model — plants, bindings,
> per-measurement thresholds, trends, manual-watering detection, species
> presets, offline-policy authoring, and an explainable recommendation engine —
> as of M6 **irrigation itself**, and as of M7 the **optional cloud sink**.
>
> **M6 is the milestone where software moves water.** The safety gate, the
> irrigation state machine, the command lifecycle, reconciliation of offline
> history, and the one shared offline evaluator a device runs while isolated.
> Fourteen safety invariants moved to `ENFORCED`, and `cargo test safety_` is
> the executable gate over all of them. A focused **post-M6 correction**
> (2026-08-31) then closed two durability defects the milestone had claimed but
> not held — see [docs/reports/M6.md](docs/reports/M6.md).
>
> **M7 is the milestone that adds the cloud without letting it matter.** An
> append-only PostgreSQL history behind an idempotent batch-ingest endpoint; a
> durable edge outbox whose event is written in the same transaction as the
> change it describes; and a drain that survives an outage of any length,
> quarantines a poison event rather than wedging behind it, and prunes only
> measurements when capped. It stays **disabled by default**, the cloud has no
> route that can originate a command, and `rhizo-domain` cannot depend on the
> cloud client — proven by test, not by review. A **post-M7 correction**
> (2026-08-31) then aligned the emitted event catalogue with ADR-005 and gave
> destructive changes the history they were not writing — see
> [docs/reports/M7.md](docs/reports/M7.md).
>
> **M8 is next**, and adds no features: it makes the whole software system
> reproducible and verifiable with one command, on any machine with Docker and
> no hardware at all. Start at [ROADMAP.md](ROADMAP.md); the next issue is
> [M8-001](docs/issues/M8/001-add-dockerfiles.md).
>
> To run it now, see [Running it locally](#running-it-locally).

---

## What it is

Rhizo Edge measures the conditions a plant actually lives in, decides whether it
needs water, and delivers a bounded dose through a pump. It is designed so that
loss of Internet, cloud, broker, Wi-Fi, or power does not turn a connectivity
failure into unsafe watering.

The first target is indoor houseplants. The architecture is deliberately shaped
so greenhouse and field deployments are an extension of the same system rather
than a rewrite.

## Not one box per plant

Rhizo is **not** a one-device-per-plant system. Sensing and actuation are
independent capabilities, declared by the device and bound per plant, so one
node can serve several plants and one plant can draw on several nodes.

- A plant may have a **dedicated soil probe**, or share a multi-channel node
  with the pots beside it.
- **Ambient sensors are shared.** One temperature, humidity, and light node is
  bound to every plant it describes — each with its own thresholds, because the
  same reading is "fine" for a succulent and "critical" for a fern.
- A plant may have **no actuator at all**: a first-class configuration with
  alerts, trends, and recommendations, not a degraded one (SAFETY-018).
- Watering may come from a **separate node** holding the pump, wherever the
  tubing actually runs.
- A single node may **do both** — sense its pot and water it.

Bindings, not wiring, decide which measurement serves which plant, so adding a
pump node or moving a probe is an API call rather than a rebuild.

### Device profiles

Three names for how the same components are deployed. They are **deployment
profiles, not separate products**: one firmware, one Edge Controller, one wire
protocol, and a device's behaviour follows the capabilities it declares.

| Profile | What it is | Built from |
|---|---|---|
| **Rhizo Sense** | Sensor-oriented node — soil, ambient, light, weight; no actuator | `esp32-node` declaring sensors only |
| **Rhizo Water** | Actuation node — a pump, plus the tank and leak sensing its own safety gate requires | `esp32-node` declaring an actuator |
| **Rhizo Hub** | The Edge Controller — bindings, thresholds, the safety gate, SQLite, the local API | `edge-controller` beside Mosquitto |

A Rhizo Water node is never *only* an actuator. The device-side gate reads leak
and tank state from **its own** sensors and refuses the dose when either is
unknown ([MQTT protocol v1](docs/protocol/mqtt-v1.md) §5.8 steps 6–8), so a pump
and the sensing that can veto it stay on one board.

### Deployment shapes

```text
── monitoring only ───────────────────────────────────────────────
   Plant A ◄─ soil_moisture ──── Sense node
           ◄─ ambient ────────── Ambient node
           no actuator bound: alerts, trends, recommendations

── separate Sense + Water ────────────────────────────────────────
   Plant B ◄─ soil_moisture ──── Sense node
           ◄─ ambient ────────── Ambient node
           ─► dose ───────────── Water node   pump · tank · leak

── combined Sense + Water ────────────────────────────────────────
   Plant C ◄─ soil_moisture ──┬─ Sense+Water node
           ─► dose ───────────┘
           ◄─ ambient ────────── Ambient node

── shared ambient sensor ─────────────────────────────────────────
   Ambient node ─► Plant A · Plant B · Plant C
                   one node, three bindings, three sets of thresholds
```

Every arrow is a **binding** held by the Hub, which decides every dose while
connected. Devices are never wired to each other, and no device is tied to a
plant by construction.

Details: [ADR-016](docs/adr/016-plant-binding-and-policy-model.md) ·
[ADR-017](docs/adr/017-extensible-measurement-model.md) ·
[deployment model](docs/architecture/deployment-model.md)

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
Twenty-four numbered invariants
([SAFETY-001…024](docs/architecture/safety-invariants.md)) state this precisely,
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

**And the record of what happened is treated as safety-critical, not as
telemetry.** The rolling 24-hour cap is derived from stored watering rows rather
than from a counter, so anything that loses or misfiles a row weakens it
silently — and always in the over-watering direction. Two mechanisms exist for
that reason alone:

- **Every dose report is acknowledged end to end.** A device holds each
  `command.result` until the Edge says it has *committed* it, and keeps
  retrying until then. MQTT's own QoS 1 acknowledgement is not enough and is
  never treated as enough: it is written by the broker on receipt, so it says
  nothing about whether any Edge read the message, stored it, or was even
  running. The same rule already governed replayed offline history.
- **A dose delivered while isolated names the plant it watered.** The Edge
  charges the plant the device names, not whichever plant holds the pump binding
  when the history finally arrives — because an isolated device is exactly when
  someone has time to move a pump, and a dose filed against the wrong plant
  leaves the plant that *was* watered free to be watered again.

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

## A sleeping device is not an offline device

A node on a balcony has no socket, so it runs from a battery: it sleeps, wakes
every ten to fifteen minutes, powers its sensors, samples, connects, exchanges
data, and sleeps again. It is unreachable almost all the time **by design**.

Reporting that as "offline" would be a lie a hundred times a day, and it would
train its owner to ignore the one indicator that says a device has actually
died. So sleep is **announced** by the device and **bounded** by the Edge:

```text
Connected                        awake and reachable
Sleeping { expected_wake_at }    announced, bounded, expected back
Isolated                         absent without an announcement, or overdue
Reconciling                      replaying buffered history after an absence
```

The window is computed from the Edge's own clock, so a device cannot make itself
look punctual, and a device that stops waking becomes `Isolated` — the new state
can only ever *defer* the offline indication, never suppress it.

Manual watering on such a device has honest latency. The Edge holds the request
as a durable **intent**, re-runs the full safety gate when the device wakes, and
only then issues the command — so a leak that appeared while the device slept
still refuses the dose. The UI shows `Pending until device wakes`, not a spinner.

Nothing on the wire changed to make this work: no protocol bump, no retained
commands, and the same 120-second command TTL, because the command is minted at
the wake rather than at the request.

**Battery life figures are targets, not specifications**, until they are measured
on assembled hardware — and the number that matters is complete-system sleep
current, which is not the chip's datasheet figure.

Details: [ADR-018](docs/adr/018-battery-and-deep-sleep-device-mode.md)

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

**Implemented in M5.** The catalogue ships twenty-two curated entries compiled
into the binary; applying one writes ordinary per-plant threshold rows resolved
against the bindings the plant already has. Tests assert over the whole
catalogue that no entry contains an interval or a schedule, and that no field
names a device, sensor, point, or capability — a preset describes a plant, not
an installation. The picker and the review step are M12.

## The pump is optional

Most plants in a real home will never have one. A plant with no actuator is a
**first-class, fully supported** configuration — telemetry, history, trends,
thresholds, warnings, critical alerts, recommendations, and UI visibility — that
simply has no actuation path. It is not a plant with a missing part. And when a
plant does have one, the pump need not sit on the plant's own node — see
[Not one box per plant](#not-one-box-per-plant).

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
  ESP32 Device Node (Rust)             Device Simulator (Rust)
  ┌──────────────────────────┐         ┌──────────────────────────┐
  │ declares capabilities:   │         │ same MQTT protocol       │
  │  sensors[] · actuators[] │         │ shared evaluator (M6+)   │
  │ Sense · Water · or both  │         │ virtual time · faults    │
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
| `rhizo-policy` | `no_std` offline-policy state and decision contract, and the one offline evaluator (M6-019) shared by simulator, firmware, and any required edge use |
| `rhizo-domain` | Pure plants, bindings, per-measurement policies, trends, detection, thresholds, the species preset catalogue, the recommendation engine, and — since M6 — the irrigation state machine and the safety gate |
| `rhizo-storage` | SQLite schema, repositories, and the deduplicate-and-persist transaction |
| `edge-controller` | Deployed as the **Rhizo Hub**. The control plane — the only component that decides while connected. Since M6 it owns the command lifecycle, reconciliation of replayed offline history, and the durable acknowledgement of every dose result |
| `device-simulator` | Implemented reference device with protocol mechanics, persistence, isolation/replay, virtual time, battery mode with real deep-sleep cycles, and faults; since M6 it waters autonomously while isolated, through the shared evaluator |
| `rhizo-cloud-client` | The edge's typed HTTP client for the cloud: bounded requests, exhaustive retry classification, and `Retry-After` handling. Nothing else in the edge knows the cloud exists |
| `cloud-api` | Implemented in M7. Idempotent event ingestion keyed by `(edge_id, event_id)` into an append-only PostgreSQL ledger, the projections rebuilt from it, and read-only history APIs. It can issue nothing |
| `esp32-node` | ESP32-C3 firmware; the final hardware safety boundary and the offline fallback controller. One codebase, deployed as **Rhizo Sense**, **Rhizo Water**, or both, according to the capabilities it declares |
| `rhizo-ui` | Tauri 2 + Leptos desktop client; talks HTTP to the edge only |

## Development strategy: simulator before hardware

The Device Simulator implements the *same* MQTT protocol as the firmware and
calls the *same* command validator. M2 supplied connectivity, policy/runtime
state persistence, isolation, buffering/replay, virtual time, and fault
mechanics, with an enabled offline policy still inert. M6-019 added the single
evaluator to `rhizo-policy` and its sole simulator call site, and a test counts
those call sites so a second implementation cannot appear quietly. Firmware will
call that same implementation; there is never a simulator-specific copy. This
keeps the full control plane — offline autonomy included — buildable and
testable before electronics exist.

**Milestones M0–M8 require no hardware at all.** When hardware does arrive at M9,
the [home node hardware guide](docs/hardware/home-node-hardware-guide.md) is the
bill of materials, wiring, power, and assembly order for building one — bench
bring-up on an official Espressif ESP32-C3-DEVKITM-1-N4X through a battery and
optional solar deployment. It is
practical guidance, not a specification: its parts and values are starting points
to be measured.

M8 delivers the complete software system, verified end to end by one command:

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

## Running it locally

No hardware is needed. One broker, one edge, one simulated plant node.

**First run only** — the broker needs accounts, and they are generated rather
than committed:

```bash
cp .env.example .env                    # then replace every placeholder
./scripts/gen-mosquitto-passwd.sh
docker compose -f deploy/docker-compose.yml up -d --wait mosquitto
```

Then, in two terminals:

```bash
# the control plane
RHIZO_EDGE__MQTT__BROKER_URL=mqtt://localhost:1883 RHIZO_EDGE__LOG__FORMAT=compact cargo run -p edge-controller

# a plant node, drying ten simulated minutes per real second
cargo run -p device-simulator --   --device-id plant-node-01   --broker mqtt://localhost:1883   --initial-moisture 42   --time-scale 600
```

`--time-scale` is what makes any of this watchable: a six-hour drying curve
takes about half a minute. Nothing in the system reads a wall clock to decide
anything, which is why accelerating time is honest rather than a shortcut
([time-model.md](docs/architecture/time-model.md) §8).

Nothing above starts the cloud, and nothing above needs it. To watch history
sync as well, bring up PostgreSQL and the cloud API and restart the edge with
the sink switched on:

```bash
docker compose -f deploy/docker-compose.yml up -d --wait postgres cloud-api
RHIZO_EDGE__CLOUD__ENABLED=true RHIZO_EDGE__CLOUD__BASE_URL=http://localhost:8081 \
  cargo run -p edge-controller
```

Stopping `cloud-api` again is the interesting half: events queue in the edge's
outbox, every watering decision carries on unchanged, `/health/ready` stays
**200**, and the backlog drains when the cloud returns.

With both running, the edge is on `http://localhost:8080`:

```bash
curl -s localhost:8080/api/v1/devices | jq              # the node, its sensors, its health
curl -s localhost:8080/api/v1/presets | jq '.presets[].display_name'
curl -s localhost:8080/api/v1/plants | jq
```

Creating a plant takes three calls — a plant, a binding that says which probe
supplies which measurement, and a threshold policy — after which
`GET /api/v1/plants/{id}/recommendation` explains itself:

```json
{
  "recommendation": "water",
  "recommended_ml": 40.0,
  "reasons": [
    { "code": "moisture_below_target", "vwc": 24.1, "target_min": 28.0,
      "message": "moisture 24.1% is below the target minimum of 28.0%" },
    { "code": "dry_for", "minutes": 42, "required": 30 }
  ]
}
```

Since M6 that recommendation can become water. `POST /plants/{id}/water`
answers **422** `no_actuator_bound` for a plant with no pump — distinguishable
from a safety refusal, which is a **409** carrying `{ reason, since, clearable,
message }` — and **202** with a `command_id` when the gate allows the dose. For
a device that is asleep it answers **202** with an `intent_id` and no
`command_id` at all: the command is minted at the next wake, with the whole gate
re-run against inputs that are current then.

There is no override, force, or bypass parameter on any of these paths, and
there is no control to wake, expedite, or cancel for a sleeping device.

### Debugging in VS Code

`.vscode/launch.json` is checked in, so `F5` works on a fresh clone once `.env`
exists. It carries the configurations worth having rather than one per binary:

| Configuration | What it is for |
|---|---|
| **Edge controller** | The control plane, loopback API, cloud off. A second entry raises the log level to `debug` |
| **Simulator: plant-node-01** | The standard node — soil, weight, tank, leak, one pump — at 600× virtual time |
| **Simulator: plant-node-02** | A second node, on its own control port, for shared-sensor and two-plant setups |
| **Simulator: monitoring only** | No actuators at all: the common shape in a real home, and the one SAFETY-018 is about |
| **Simulator: battery node** | Sleeps between samples and announces each sleep, so the `sleeping` connectivity state has a producer |
| **Simulator: with a fault…** | Prompts for any fault in the catalogue — leak, tank-empty, clock-unsync, stuck-sensor, miss-wake, and the rest |
| **Simulator: choose device and time scale…** | Prompts for both |

Three compounds start the usual pairings in one keystroke: *Edge + one plant
node*, *Edge + two plant nodes*, and *Edge + battery node*.

`.vscode/tasks.json` carries the broker (`Mosquitto: up`, `logs`, `down`), the
gate commands, and two `mosquitto_sub` watchers — including one on the command
topics alone, which is the fastest way to confirm the edge is still not acting.

The launch configurations use the MSVC debugger, matching the
`x86_64-pc-windows-msvc` host. On Linux and macOS use the two `(lldb)` entries
as the pattern; they need the CodeLLDB extension and build through cargo
themselves.

More: [local development](docs/testing/local-development.md) — watching MQTT
directly, injecting faults through the simulator's control API, and what each
symptom usually means.

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
RHIZO_REQUIRE_BROKER=1 RHIZO_REQUIRE_POSTGRES=1 RHIZO_REQUIRE_CLOUD=1 \
  cargo test --workspace --all-features
cargo build -p rhizo-mqtt-contract --no-default-features --target thumbv7em-none-eabi
cargo build -p rhizo-policy --no-default-features --target thumbv7em-none-eabi
docker compose -f deploy/docker-compose.yml config
cargo sqlx prepare --workspace --check
cargo run -p rhizo-docscheck
```

The three `RHIZO_REQUIRE_*` flags are how CI runs the suite. Without them the
tests that need a broker, PostgreSQL, or a live cloud print a loud skip and
pass, so a fresh clone is green; with them, a missing service is a failure — a
suite that can silently skip its own subject eventually proves nothing.

They also mean **a bare test total is not evidence**. Dozens of tests skip and
still count as passed, so the workspace total is identical with the services
stopped and with them running. Quote the environment and the per-suite counts,
or the number says nothing.

Each flag needs the address of the thing it requires: `RHIZO_TEST_BROKER`,
`RHIZO_TEST_POSTGRES_URL`, `RHIZO_TEST_CLOUD_URL`.

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
| [Safety invariants](docs/architecture/safety-invariants.md) | SAFETY-001…021 |
| [Connectivity modes](docs/architecture/connectivity-modes.md) | Cloud offline vs site offline vs device isolated vs sleeping |
| [Offline autonomy](docs/architecture/offline-autonomy.md) | The offline policy model and reconciliation |
| [MQTT protocol v1](docs/protocol/mqtt-v1.md) | Normative wire specification |
| [Dependency graph](docs/architecture/dependency-graph.md) | What to implement next |
| [Failure model](docs/architecture/failure-model.md) | Every failure and its expected behaviour |
| [Testing strategy](docs/testing/strategy.md) | How the safety claims are proven |
| [Local development](docs/testing/local-development.md) | Running and debugging |
| [Milestone reports](docs/reports/) | What each completed milestone actually delivered, and what it deliberately did not |
| [Home node hardware guide](docs/hardware/home-node-hardware-guide.md) | What to buy and how to build one physical node |

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
  for them. Unacknowledged dose reports are bounded the same way, and for the
  same reason.
- **Telemetry is lossy on purpose.** Samples get no acknowledgement and no
  retry: a lost sample makes data look older, and stale data blocks watering, so
  losing one fails safe. Only ledger data — dose results and offline history —
  is delivered with an end-to-end guarantee.

## License

Rhizo Edge is **source-visible, not open source**. The repository is public so
that the code and its design documents can be read, reviewed, and evaluated.

Publication grants no licence to use the software. Copyright © 2026 WiverNz;
all rights reserved. Without prior written permission you may not use,
copy, modify, distribute, sublicense, sell, create derivative works from, or
incorporate this code — or any part of it — into another product or project.
Commercial and non-commercial use alike require that permission, and enquiries
are welcome.

Rights granted by GitHub's Terms of Service for viewing and forking through
GitHub are unaffected, and third-party dependencies remain governed by their own
licences.

Full terms: [LICENSE](LICENSE).
