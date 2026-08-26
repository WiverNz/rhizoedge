# Rhizo Edge — Project Plan

> **Normalisation note.** This is the original product specification and remains
> the source of product *intent*. Superseded implementation alternatives have
> been replaced with the decisions actually taken, so the repository does not
> retain contradictory project-facing architecture. Specifically: the cloud
> service is **Rust** (there is no Go anywhere in this project), the ESP32
> firmware is **Rust** (Arduino/C++ is not a planned fallback), and the UI is a
> **Tauri 2 + Leptos** desktop application talking to the Edge REST API.
> Product goals, safety rules, and milestones are unchanged.
>
> This document lives in `docs/`, alongside the planning briefs. Nothing
> requires it at the repository root.
>
> Authoritative for decisions: [../ROADMAP.md](../ROADMAP.md),
> [adr/](adr/), [prd/](prd/). Where this file and an ADR disagree, the ADR wins.

## 1. Project Overview

**Rhizo Edge** is an edge-first monitoring and automated irrigation platform for plants.

The first version targets **indoor houseplants**, using:

- ESP32 devices
- soil moisture / temperature / EC sensors
- optional pot weight sensors
- water reservoir level sensors
- leak sensors
- peristaltic pumps
- MQTT
- a Rust edge controller
- local persistent storage
- optional cloud synchronization
- a desktop UI (Tauri 2 + Leptos)

The architecture should be designed from the beginning so the same platform can later scale to:

- multiple rooms
- greenhouses
- gardens
- irrigation zones
- farms
- multi-depth soil probes
- LoRaWAN / NB-IoT connectivity
- weather data
- agricultural recommendations

The core principle is:

> **The system must remain safe and useful even when the cloud or Internet is unavailable.**

---

# 2. Goals

## 2.1 Primary goals

1. Measure soil conditions continuously.
2. Store and visualize historical plant data.
3. Detect watering events automatically.
4. Recommend when and how much to water.
5. Support manual remote watering.
6. Support safe automatic watering.
7. Continue functioning while offline.
8. Detect hardware failures and unsafe states.
9. Use MQTT as the device communication protocol.
10. Use Rust for the edge/backend logic.
11. Design the system so it can later support agricultural deployments.

---

## 2.2 Secondary goals

- Learn practical Rust async development.
- Learn MQTT semantics and failure modes.
- Learn ESP32 firmware development.
- Learn Modbus/RS485 sensor integration.
- Implement idempotent distributed messaging.
- Implement local-first edge processing.
- Build a useful real-world system instead of a demo-only IoT project.

---

# 3. Non-Goals for V1

The first version should **not** attempt to implement:

- machine learning
- automatic fertilizer dosing
- direct N/P/K mineral measurement
- LoRaWAN
- solar power
- agricultural weather forecasting
- multi-region cloud infrastructure
- mobile applications
- complex user authentication
- production-grade multi-tenancy
- autonomous watering without safety limits

These can be added later.

---

# 4. High-Level Architecture

```text
                        HOUSE / LOCAL NETWORK

 ┌────────────────────────────────────────────────────────────┐
 │                                                            │
 │  Plant Node #1                                             │
 │  ┌──────────────────────────────┐                          │
 │  │ ESP32                        │                          │
 │  │                              │                          │
 │  │ Soil probe                   │                          │
 │  │ - moisture                   │                          │
 │  │ - temperature                │                          │
 │  │ - EC                         │                          │
 │  │                              │                          │
 │  │ Optional sensors             │                          │
 │  │ - pot weight                 │                          │
 │  │ - tank level                 │                          │
 │  │ - leak detector              │                          │
 │  │                              │                          │
 │  │ Actuator                     │                          │
 │  │ - peristaltic pump           │                          │
 │  └──────────────┬───────────────┘                          │
 │                 │                                          │
 │                 │ Wi-Fi / MQTT                             │
 │                 ▼                                          │
 │         ┌───────────────┐                                  │
 │         │ MQTT Broker   │                                  │
 │         │ Mosquitto     │                                  │
 │         └──────┬────────┘                                  │
 │                │                                           │
 │                ▼                                           │
 │   ┌───────────────────────────────────────┐                │
 │   │ Rust Edge Controller                  │                │
 │   │                                       │                │
 │   │ - MQTT consumer                       │                │
 │   │ - device registry                     │                │
 │   │ - measurement processing              │                │
 │   │ - irrigation state machines           │                │
 │   │ - local rules                         │                │
 │   │ - safety logic                        │                │
 │   │ - event detection                     │                │
 │   │ - offline queue                       │                │
 │   │ - local REST API                      │                │
 │   │ - cloud sync                          │                │
 │   └───────────────┬───────────────────────┘                │
 │                   │                                        │
 │                   ▼                                        │
 │            ┌──────────────┐                                │
 │            │ SQLite       │                                │
 │            │ local DB     │                                │
 │            └──────────────┘                                │
 │                                                            │
 └────────────────────────────────────────────────────────────┘

                         OPTIONAL INTERNET

                              │
                              ▼
                    ┌──────────────────┐
                    │ Cloud API        │
                    │ Rust             │
                    └────────┬─────────┘
                             │
                             ▼
                    ┌──────────────────┐
                    │ PostgreSQL       │
                    └────────┬─────────┘
                             │
                             ▼
                    ┌──────────────────┐
                    │ Rhizo UI         │
                    │ Tauri 2 + Leptos │
                    │ desktop app      │
                    │ → Edge REST API  │
                    └──────────────────┘
```

---

# 5. Deployment Model

## 5.1 Development

Everything initially runs on one PC:

```text
Docker Compose
├── Mosquitto
├── edge-controller
├── SQLite volume
├── mock-cloud
└── (UI runs on the host, not in Compose)
```

A Rust `device-simulator` acts like an ESP32.

This allows most of the software to be implemented **before buying or wiring hardware**.

---

## 5.2 Home deployment

Recommended later deployment:

```text
Raspberry Pi / mini-PC
├── Mosquitto
├── edge-controller
└── SQLite
```

ESP32 nodes communicate with it over local Wi-Fi.

Internet access is optional.

---

# 6. Main Components

## 6.1 ESP32 Plant Node

Responsibilities:

- read sensors
- validate basic sensor values
- publish telemetry
- receive configuration
- receive watering commands
- control pump
- enforce local hard safety limits
- publish health/status
- reconnect automatically to Wi-Fi/MQTT
- survive broker/network interruptions

The ESP32 should **not** contain complex irrigation intelligence in V1.

The Rust edge controller owns the main decision logic.

However, the ESP32 must enforce final safety limits even if a bad command arrives.

---

## 6.2 MQTT Broker

Use:

- Eclipse Mosquitto

Responsibilities:

- telemetry transport
- commands
- device status
- configuration delivery
- Last Will messages
- QoS handling

The MQTT broker runs locally.

Cloud communication should not be required for plant operation.

---

## 6.3 Rust Edge Controller

This is the main software component.

Responsibilities:

- subscribe to telemetry
- normalize data
- validate measurements
- persist measurements
- detect watering events
- maintain plant/device state
- execute watering state machines
- publish pump commands
- detect unsafe conditions
- buffer cloud events while offline
- expose local API
- provide metrics/logging
- support graceful shutdown and restart

Suggested libraries:

```text
tokio
rumqttc
axum
serde
serde_json
sqlx
tracing
tracing-subscriber
thiserror
uuid
chrono
```

Optional:

```text
prometheus
config
tower
tower-http
```

---

## 6.4 Device Simulator

The simulator should be implemented **before the ESP32 firmware**.

It must simulate:

- soil drying over time
- watering
- temperature drift
- EC values
- reservoir level
- leak sensor
- MQTT disconnects
- duplicate MQTT messages
- out-of-order messages
- invalid measurements
- pump operation

Example:

```text
cargo run -p device-simulator -- \
  --device plant-node-01 \
  --drying-rate 0.4 \
  --initial-moisture 42
```

---

# 7. Hardware Design

## 7.1 Minimum indoor prototype

Required:

- ESP32 development board
- capacitive soil moisture sensor OR RS485 moisture/temp/EC probe
- peristaltic pump
- pump driver / MOSFET module
- external pump power supply
- tubing
- water reservoir

Recommended:

- leak sensor
- reservoir level sensor
- load cell + HX711

---

## 7.2 Recommended sensor progression

### Phase A

Start with:

- soil moisture
- soil temperature

### Phase B

Add:

- EC

### Phase C

Add:

- pot weight
- reservoir level
- leak sensor

### Phase D

Optional:

- pH
- ambient light
- ambient temperature/humidity

Do **not** make cheap NPK probes a core requirement.

---

# 8. Sensor Interfaces

## 8.1 Simple analog/I2C sensors

Useful for early development.

ESP32 reads them directly.

---

## 8.2 RS485 / Modbus RTU

Preferred long-term soil sensor interface.

Example:

```text
Soil Probe
  │
  │ RS485
  ▼
RS485 Transceiver
  │
  ▼
ESP32
```

Benefits:

- industrial standard
- long cable support
- robust communication
- real sensor addressing
- good preparation for agricultural hardware

The firmware should contain a generic Modbus sensor abstraction.

---

# 9. MQTT Topic Design

Use versioned, predictable topics.

Base:

```text
rhizo/v1/
```

---

## 9.1 Telemetry

```text
rhizo/v1/devices/{device_id}/telemetry/soil
rhizo/v1/devices/{device_id}/telemetry/weight
rhizo/v1/devices/{device_id}/telemetry/tank
rhizo/v1/devices/{device_id}/telemetry/pump
```

Example:

```json
{
  "message_id": "018fd6c4-...",
  "device_id": "plant-node-01",
  "sequence": 81273,
  "timestamp": "2026-08-25T11:30:00Z",
  "soil": {
    "moisture_vwc": 31.7,
    "temperature_c": 21.4,
    "ec_us_cm": 840
  }
}
```

---

## 9.2 Device status

```text
rhizo/v1/devices/{device_id}/status
```

Retained.

Online:

```json
{
  "status": "online",
  "firmware": "0.1.0",
  "timestamp": "..."
}
```

MQTT Last Will:

```json
{
  "status": "offline",
  "reason": "connection_lost"
}
```

---

## 9.3 Configuration

```text
rhizo/v1/devices/{device_id}/config
```

Retained.

Example:

```json
{
  "telemetry_interval_seconds": 300,
  "pump": {
    "max_single_run_seconds": 15,
    "max_daily_ml": 300
  }
}
```

---

## 9.4 Commands

```text
rhizo/v1/devices/{device_id}/commands/water
rhizo/v1/devices/{device_id}/commands/tare
rhizo/v1/devices/{device_id}/commands/calibrate
```

Water command:

```json
{
  "command_id": "018fd7...",
  "requested_ml": 50,
  "expires_at": "2026-08-25T11:35:00Z"
}
```

ESP32 must reject:

- expired command
- duplicate command
- invalid amount
- amount above local safety maximum
- command while leak detected
- command while tank empty
- command while pump lockout is active

---

## 9.5 Command results

```text
rhizo/v1/devices/{device_id}/commands/result
```

Example:

```json
{
  "command_id": "018fd7...",
  "status": "completed",
  "delivered_ml": 49.2,
  "duration_ms": 6120
}
```

---

# 10. MQTT Semantics

## 10.1 QoS

Use:

```text
QoS 1
```

for:

- telemetry
- commands
- command results
- important events

Reason:

At-least-once delivery is acceptable if consumers are idempotent.

---

## 10.2 Deduplication

Every message must contain:

```text
device_id
sequence
message_id
```

The edge controller maintains the latest processed sequence or message IDs.

Duplicate messages must not create duplicate watering events or commands.

---

## 10.3 Retained messages

Use retained messages for:

- device status
- configuration

Do not retain regular telemetry.

---

## 10.4 Last Will

Every device configures MQTT Last Will.

If connection disappears unexpectedly:

```text
status = offline
```

is automatically published.

---

# 11. Local Database

Use SQLite initially.

Suggested tables:

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

---

## 11.1 devices

```text
id
name
firmware_version
last_seen_at
status
config_version
created_at
```

---

## 11.2 plants

```text
id
device_id
name
species
pot_volume_ml
soil_type
auto_watering_enabled
created_at
```

---

## 11.3 measurements

```text
id
device_id
timestamp
sequence
moisture_vwc
soil_temperature_c
ec_us_cm
pot_weight_g
tank_level_percent
```

Index:

```text
(device_id, timestamp)
```

---

## 11.4 watering_events

```text
id
plant_id
started_at
completed_at
requested_ml
delivered_ml
mode
reason
status
```

Modes:

```text
manual
recommended
automatic
```

---

# 12. Measurement Processing Pipeline

```text
MQTT message
   ↓
decode
   ↓
schema validation
   ↓
deduplicate
   ↓
range validation
   ↓
normalize
   ↓
persist
   ↓
update plant state
   ↓
event detection
   ↓
irrigation evaluation
   ↓
optional cloud sync
```

---

# 13. Sensor Validation

Example acceptable ranges:

```text
moisture_vwc:       0..100 %
temperature:       -20..80 °C
EC:                  0..20000 µS/cm
tank_level:          0..100 %
```

Invalid values must:

- be stored as diagnostic events if useful
- not trigger automatic watering
- increment error counters
- potentially mark the sensor unhealthy

---

# 14. Plant State Model

Each plant maintains derived state.

Example:

```text
HEALTHY
DRYING
WATER_RECOMMENDED
WAITING_FOR_WATER_RESPONSE
RECOVERING_AFTER_WATER
SENSOR_FAULT
WATERING_LOCKED
```

State transitions should be explicit and testable.

---

# 15. Automatic Watering State Machine

Automatic watering must never be implemented as:

```text
if moisture < threshold:
    pump_on()
```

Use a state machine.

```text
NORMAL
  │
  │ moisture below threshold
  │ continuously for N minutes
  ▼
DRY_CONFIRMED
  │
  │ safety checks pass
  ▼
WATER_DOSE
  │
  │ 20-50 ml
  ▼
WAIT_FOR_ABSORPTION
  │
  │ 10-20 minutes
  ▼
RECHECK
  ├──── moisture recovered ───► NORMAL
  │
  └──── still dry ────────────► WATER_DOSE
                                   │
                                   │ max dose reached
                                   ▼
                              LOCK_AND_ALERT
```

---

# 16. Watering Safety Rules

These rules are mandatory.

## 16.1 Maximum single dose

Example:

```text
max_single_dose_ml = 50
```

---

## 16.2 Maximum daily water

Example:

```text
max_daily_water_ml = 300
```

---

## 16.3 Cooldown

Example:

```text
minimum_time_between_cycles = 6 hours
```

---

## 16.4 Leak lockout

If leak sensor is active:

```text
AUTOMATIC WATERING DISABLED
MANUAL WATERING DISABLED
```

Require explicit reset after leak disappears.

---

## 16.5 Empty reservoir lockout

If tank level is below minimum:

```text
pump disabled
```

---

## 16.6 Sensor fault lockout

If soil moisture sensor is missing or invalid:

```text
automatic watering disabled
```

---

## 16.7 Stale data

If latest soil measurement is older than configured threshold:

```text
automatic watering disabled
```

---

## 16.8 Command expiry

Water commands must have TTL.

Old commands must never execute after reconnection.

---

## 16.9 ESP32 hard limit

Even if edge-controller sends:

```json
{
  "requested_ml": 10000
}
```

the ESP32 must refuse it.

The device itself is the final hardware safety boundary.

---

# 17. Pump Calibration

Pump delivery must be calibrated.

Example process:

1. Place outlet into measuring container.
2. Run pump for 10 seconds.
3. Measure delivered water.
4. Repeat 3-5 times.
5. Calculate average flow.

Store:

```text
ml_per_second
```

Example:

```text
8.2 ml/sec
```

Then:

```text
duration = requested_ml / ml_per_second
```

Calibration should be stored in device configuration.

---

# 18. Pot Weight Integration

Optional but strongly recommended.

Hardware:

```text
Load Cell
   ↓
HX711
   ↓
ESP32
```

Benefits:

- estimate total water loss
- detect watering independently
- identify faulty soil sensor
- estimate evapotranspiration
- verify delivered water

Derived signal:

```text
weight loss per day
```

---

# 19. Watering Event Detection

The system should detect manual watering even when the pump was not used.

Possible signals:

```text
soil moisture sharply increases
AND/OR
pot weight sharply increases
```

Example:

```text
before:
  VWC = 28%
  weight = 5220 g

after:
  VWC = 48%
  weight = 5570 g
```

Create:

```json
{
  "event": "manual_watering_detected",
  "estimated_water_ml": 350
}
```

---

# 20. Irrigation Recommendation Engine — V1

Start rule-based.

Inputs:

- current moisture
- moisture trend
- time since last watering
- pot weight trend
- plant profile
- soil temperature
- EC
- recent watering

Example logic:

```text
IF
  moisture < target_min
AND
  moisture has remained low for 30 minutes
AND
  last watering > 24 hours ago
AND
  sensor is healthy

THEN
  recommendation = water
```

Output:

```json
{
  "plant_id": "monstera-01",
  "recommendation": "water",
  "recommended_ml": 120,
  "confidence": 0.87,
  "reason": [
    "soil moisture below target",
    "dry condition persisted for 42 minutes",
    "last watering was 6 days ago"
  ]
}
```

Explainability should be built in from the beginning.

---

# 21. Plant Profiles

Example:

```yaml
id: monstera_default

moisture:
  target_min: 28
  target_max: 48

watering:
  default_dose_ml: 100
  max_daily_ml: 300
  minimum_interval_hours: 24

ec:
  warning_high_us_cm: 1800
```

Profiles should be editable.

Do not assume universal values are correct for every soil mixture.

---

# 22. EC Monitoring

EC is used as an indicator of dissolved salts.

V1 should:

- record EC history
- show trend
- warn about abnormal increases
- correlate EC changes with watering/fertilization

Do not claim that EC directly determines exact N/P/K values.

---

# 23. Offline Operation

This is a core project requirement.

If Internet/cloud disappears:

```text
ESP32
  ↓ MQTT
Local broker
  ↓
Rust edge controller
  ↓
SQLite
```

must continue working.

Local functions that remain available:

- telemetry ingestion
- measurements
- recommendations
- safe auto-watering
- alarms
- local API

---

# 24. Cloud Sync

Cloud synchronization is asynchronous.

```text
local event
  ↓
SQLite pending_cloud_events
  ↓
sync worker
  ↓
cloud API
```

Event states:

```text
pending
sending
synced
failed
```

Retry:

```text
exponential backoff
+
jitter
```

Cloud API must be idempotent.

Use:

```text
event_id
```

as idempotency key.

---

# 25. Failure Scenarios

The project must explicitly test these.

## MQTT

- broker restart
- duplicate message
- delayed message
- device disconnect
- reconnect
- lost connection during command

## ESP32

- power loss
- restart during watering
- sensor disconnected
- corrupted measurement
- Wi-Fi unavailable

## Edge controller

- process crash
- restart
- SQLite locked
- malformed MQTT payload
- command published twice

## Pump

- pump does not deliver water
- pump runs longer than expected
- empty reservoir
- leak detected

## Cloud

- DNS failure
- timeout
- 500 response
- long outage
- duplicate synchronization

---

# 26. Observability

## 26.1 Structured logs

Use Rust `tracing`.

Fields:

```text
device_id
plant_id
message_id
command_id
event_id
```

---

## 26.2 Metrics

Expose:

```text
mqtt_messages_received_total
mqtt_decode_errors_total
measurements_processed_total
duplicate_messages_total
watering_commands_total
watering_failures_total
devices_online
devices_offline
pending_cloud_events
sensor_errors_total
```

Latency:

```text
mqtt_processing_duration
cloud_sync_duration
```

---

## 26.3 Health endpoints

```text
GET /health/live
GET /health/ready
```

---

# 27. Local REST API

Examples:

```text
GET /api/v1/devices
GET /api/v1/devices/{id}

GET /api/v1/plants
GET /api/v1/plants/{id}

GET /api/v1/plants/{id}/measurements

GET /api/v1/plants/{id}/recommendation

POST /api/v1/plants/{id}/water

POST /api/v1/plants/{id}/auto-watering/enable
POST /api/v1/plants/{id}/auto-watering/disable
```

---

# 28. UI — V1

A **Tauri 2 + Leptos (CSR) + Trunk** desktop application. No Node.js, no
TypeScript. It is a thin client of the Edge REST API and holds no authoritative
state.

The actuation path is strictly:

```text
UI → Edge REST API → domain safety gate → MQTT command → device veto → pump
```

The UI has **no MQTT dependency**, so `UI → MQTT pump command` does not compile.
A safety refusal arrives as HTTP 409 with a structured lockout reason, and there
is no override control anywhere in the interface.

See [adr/009-ui-architecture-and-rust-web-stack.md](adr/009-ui-architecture-and-rust-web-stack.md)
and [prd/120-rust-ui.md](prd/120-rust-ui.md).

Keep UI simple.

Plant page:

```text
Monstera

Status             Healthy
Soil moisture      34%
Soil temperature   21.8°C
EC                  920 µS/cm
Pot weight          5.31 kg
Tank                72%

Last watering       4 days ago
Next estimate       ~2 days

Recommendation:
No watering required
```

Charts:

- moisture over time
- EC over time
- weight over time
- watering events

Actions:

```text
[ Water 30 ml ]
[ Water 50 ml ]
[ Enable Auto Watering ]
```

---

# 29. Security

V1 local network:

- MQTT username/password
- unique credentials per device if practical
- no anonymous broker access
- REST API bind to local network only

Later:

- MQTT TLS
- per-device certificates
- signed firmware
- secure provisioning
- cloud auth
- command authorization

---

# 30. ESP32 Firmware Structure

Suggested structure:

```text
firmware/esp32-node/
├── Cargo.toml            # own workspace, own toolchain pin
├── build.rs
├── sdkconfig.defaults
├── .cargo/config.toml
└── src/
    ├── main.rs           # pump-off FIRST, then init
    ├── board.rs          # pin assignments in one place
    ├── net/              # wifi.rs, mqtt.rs, time_sync.rs
    ├── sensors/          # trait defs + fake/ + real/
    ├── pump/             # trait def + fake/ + real/
    ├── safety/           # hard limits, dedup ring, TTL check
    ├── nvs.rs            # persisted state
    └── app/              # host-testable orchestration, no ESP-IDF imports
```

The firmware is **Rust**, targeting ESP32-C3 via `esp-idf-svc` (std). Arduino
and C++ are not planned fallbacks: a second implementation of the command
validator in a second language is precisely the divergence SAFETY-007 depends on
not happening.

Rust is used everywhere — firmware, simulator, edge controller, cloud API, and
UI — and the edge controller is still where it delivers the most value first,
which is why milestones M0–M8 build it before any hardware exists.

See [adr/007-esp32-rust-framework-and-toolchain.md](adr/007-esp32-rust-framework-and-toolchain.md)
and [adr/008-shared-code-simulator-and-firmware.md](adr/008-shared-code-simulator-and-firmware.md).

---

# 31. Rust Workspace Structure

```text
rhizo/
├── Cargo.toml
│
├── crates/
│   ├── mqtt-contract/     # no_std, shared with firmware
│   ├── domain/            # pure logic, no I/O, no clock
│   ├── storage/
│   ├── telemetry/
│   ├── cloud-client/
│   ├── testkit/
│   ├── edge-controller/
│   ├── device-simulator/
│   └── cloud-api/
│
├── firmware/
│   └── esp32-node/        # own workspace (Rust)
│
├── ui/
│   └── rhizo-ui/          # own workspace (Tauri 2 + Leptos)
│
├── migrations/
│
├── deploy/
│   ├── docker-compose.yml
│   └── mosquitto/
│
├── docs/
│   ├── architecture.md
│   ├── mqtt.md
│   ├── watering-safety.md
│   └── hardware.md
│
└── README.md
```

---

# 32. Domain Types

Example Rust types:

```rust
struct SoilMeasurement {
    device_id: DeviceId,
    sequence: u64,
    timestamp: DateTime<Utc>,
    moisture_vwc: Option<f32>,
    temperature_c: Option<f32>,
    ec_us_cm: Option<u32>,
}
```

```rust
enum WateringMode {
    Manual,
    Recommended,
    Automatic,
}
```

```rust
enum PlantState {
    Healthy,
    Drying,
    WaterRecommended,
    WaitingForResponse,
    Recovering,
    SensorFault,
    WateringLocked,
}
```

---

# 33. Testing Strategy

## 33.1 Unit tests

Test:

- moisture rules
- watering limits
- state machine transitions
- deduplication
- command expiry
- pump duration calculation
- plant profile evaluation

---

## 33.2 Property tests

Useful for safety logic.

Examples:

> Total automatic watering within any 24-hour interval must never exceed configured maximum.

> Duplicate commands must never cause duplicate pump execution.

---

## 33.3 Integration tests

Run:

```text
Mosquitto
+
edge-controller
+
device-simulator
```

Test complete MQTT workflows.

---

## 33.4 Failure tests

Automatically simulate:

- MQTT restart
- duplicate QoS 1 messages
- cloud outage
- controller restart
- stale sensor
- empty tank
- leak signal

---

## 33.5 Hardware-in-the-loop tests

Later:

```text
real ESP32
+
real pump
+
fake/small water container
+
test plant or measuring cup
```

Do not test first automatic watering against an expensive plant.

---

# 34. Development Milestones

## M0 — Repository and local infrastructure

Deliverables:

- Rust workspace
- Docker Compose
- Mosquitto
- edge-controller skeleton
- device-simulator skeleton
- logging

Acceptance:

```text
device-simulator publishes MQTT message
edge-controller receives it
```

---

## M1 — MQTT contract

Implement:

- telemetry schema
- device status
- configuration
- commands
- command results
- QoS 1
- retained status/config
- Last Will
- deduplication

Acceptance:

- simulated device can disconnect/reconnect
- duplicates do not create duplicate events

---

## M2 — Local persistence

Implement SQLite.

Store:

- devices
- measurements
- events
- commands

Acceptance:

- restart edge-controller
- historical data remains
- device state restores correctly

---

## M3 — Plant monitoring

Implement:

- plant profiles
- measurement validation
- moisture history
- moisture trends
- recommendation engine V1

Acceptance:

- simulator dries soil
- system eventually generates watering recommendation

---

## M4 — Watering state machine

Software-only first.

Implement:

- manual watering command
- recommended watering
- automatic watering state machine
- cooldown
- daily max
- stale-data lockout
- sensor-fault lockout
- command TTL

Acceptance:

- all safety rules covered by tests
- simulator demonstrates multi-dose watering cycle

---

## M5 — Offline/cloud synchronization

Implement:

- mock cloud API
- pending event queue
- idempotent sync
- exponential backoff + jitter

Acceptance:

1. stop cloud
2. generate events
3. verify local operation
4. restart cloud
5. verify all events synchronize exactly once logically

---

## M6 — ESP32 hardware prototype

Replace simulator with one real ESP32.

Start with:

- moisture sensor
- temperature sensor
- MQTT telemetry

Acceptance:

- real plant telemetry visible in edge-controller

---

## M7 — Real pump

Add:

- peristaltic pump
- driver
- calibration
- water command
- ESP32 hard safety maximum

Acceptance:

```text
POST water 50ml
```

delivers approximately configured amount.

---

## M8 — Safety hardware

Add:

- reservoir level
- leak sensor

Acceptance:

- empty reservoir prevents watering
- leak immediately locks pump

---

## M9 — Pot weight

Add:

- load cell
- HX711
- tare/calibration
- weight telemetry

Implement:

- manual watering detection
- water-loss trend

---

## M10 — UI

Implement simple UI.

Acceptance:

- current state
- charts
- history
- recommendation
- manual watering
- automatic watering toggle

---

# 35. V1 Definition of Done

V1 is complete when:

1. One real houseplant is monitored.
2. Soil moisture and temperature are collected automatically.
3. Measurements are stored locally.
4. System works without Internet.
5. MQTT uses QoS 1 and deduplication.
6. Device Last Will works.
7. Configuration is retained.
8. UI shows plant history.
9. Manual watering works.
10. Automatic watering uses a state machine.
11. Leak/tank/sensor failures disable watering.
12. Daily water limits cannot be exceeded.
13. Controller survives restarts.
14. Pump commands are idempotent.
15. Cloud outage does not prevent local operation.

---

# 36. V2 — Multi-Plant Home System

Add:

- multiple ESP32 nodes
- multiple pumps
- plant onboarding
- device provisioning
- reusable plant profiles
- centralized reservoir
- notifications

Possible architecture:

```text
              Rust Edge Hub
                    │
        ┌───────────┼────────────┐
        ▼           ▼            ▼
     ESP32 #1    ESP32 #2     ESP32 #3
       │            │             │
    Monstera      Ficus         Basil
```

---

# 37. V3 — Greenhouse

Add:

- irrigation zones
- solenoid valves
- several soil probes
- ambient humidity
- ambient temperature
- light
- water flow meter
- stronger edge computer
- alerting

---

# 38. V4 — Agricultural / Field Version

Replace or extend connectivity.

```text
Soil probe
   │ RS485
   ▼
Field controller
   │
   ├── LoRaWAN
   ├── LTE-M
   └── NB-IoT
        │
        ▼
   Edge gateway
        │ MQTT
        ▼
   Rust platform
```

---

# 39. Multi-Depth Soil Monitoring

Agricultural deployments should support probes at depths such as:

```text
10 cm
20 cm
30 cm
40 cm
60 cm
```

This allows the system to determine:

- root-zone water availability
- whether irrigation penetrated deeply enough
- water movement through soil
- over-irrigation
- drainage

Data model should therefore avoid assuming one soil measurement per plant.

Future model:

```text
measurement_point:
  depth_cm: 30
```

---

# 40. Weather Integration

Future inputs:

- rainfall
- temperature
- humidity
- wind
- solar radiation
- weather forecast
- evapotranspiration

Recommendation engine can evolve from:

```text
current moisture
```

to:

```text
soil moisture profile
+
crop
+
growth stage
+
weather
+
rain forecast
+
evapotranspiration
+
irrigation history
```

---

# 41. Farm Irrigation Recommendations

Example future output:

```text
Field 17 — Corn

Root zone water:
31 mm available

Expected rainfall:
8 mm within 18 hours

Recommendation:
Do not irrigate today

Confidence:
0.89
```

The decision engine should remain explainable.

---

# 42. Future Fertility Monitoring

Possible progression:

1. EC monitoring
2. pH monitoring
3. fertilizer event tracking
4. calibrated nitrate sensor
5. laboratory correlation
6. crop-specific nutrient models

Do not derive exact N/P/K values from generic EC alone.

---

# 43. Future Edge Features

Potential advanced features:

- OTA firmware updates
- device certificates
- local rules engine
- Wasm-based rules
- remote diagnostics
- local anomaly detection
- adaptive sampling
- compressed telemetry
- multi-gateway failover
- edge clustering
- firmware rollout groups

---

# 44. Engineering Principles

## Local first

Cloud loss must not stop safe plant operation.

## Safety over automation

If uncertain:

```text
do not water
```

and alert the user.

## Idempotency

All commands and cloud events must tolerate retries.

## Explicit state machines

Avoid hidden implicit watering behavior.

## Observable failures

Every failure should produce:

- structured log
- state transition
- metric/event

## Hardware independence

Core domain logic must not depend directly on a specific sensor model.

## Progressive complexity

Do not add agricultural complexity until the indoor system works reliably.

---

# 45. Recommended Implementation Order

The most efficient path is:

```text
1. Rust simulator
2. MQTT
3. Rust edge controller
4. SQLite
5. watering state machine
6. offline sync
7. ESP32 telemetry
8. real soil sensor
9. pump
10. leak/tank safety
11. pot weight
12. UI
13. multi-plant
14. greenhouse
15. field connectivity
```

Do **not** begin with hardware.

The simulator allows approximately 70–80% of the interesting backend/edge logic to be implemented and tested before any electronics are assembled.

---

# 46. First Practical Demo

The first demo should show:

1. Start Mosquitto.
2. Start edge-controller.
3. Start refrigerator/plant simulator.
4. Soil moisture gradually decreases.
5. Edge-controller detects dry soil.
6. Recommendation appears.
7. Automatic mode issues a 30 ml watering command.
8. Simulator increases moisture.
9. Edge waits for absorption.
10. Moisture remains too low.
11. Second dose is issued.
12. Moisture recovers.
13. State returns to healthy.
14. Cloud is stopped.
15. Local system continues working.
16. Events queue locally.
17. Cloud restarts.
18. Pending events synchronize.

That single demo demonstrates:

- Rust
- async programming
- MQTT
- edge computing
- offline-first architecture
- idempotency
- state machines
- persistence
- safety logic
- distributed-system failure handling

---

# 47. Suggested README One-Liner

> Rhizo Edge is an offline-first Rust/ESP32 platform for soil monitoring and fail-safe automated irrigation, using MQTT, local edge processing, Modbus sensors and event-driven device control.

---

# 48. Project Success Criteria

The project is successful if it becomes something that can actually be trusted with a real plant.

Technical sophistication is secondary to:

- reliable measurements
- deterministic control
- safe failure behavior
- observable state
- recoverability
- clear architecture

The long-term agricultural architecture should grow naturally from the working indoor system rather than being designed as an oversized farm platform from day one.
