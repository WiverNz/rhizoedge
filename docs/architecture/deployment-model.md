# Deployment Model

Three topologies matter: **development** (M0–M8, no hardware), **home**
(V1 target), and **future** (greenhouse/field, M13–M14 planning only).

---

## 1. Development topology — the M8 target

Everything on one developer machine. No ESP32, no pump, no plant.

```text
$ docker compose up --build

┌──────────────────────────── Docker network: rhizo ───────────────────────────┐
│                                                                              │
│  device-simulator ──MQTT 1883──► mosquitto ──MQTT 1883──► edge-controller    │
│  (1..N instances)                                              │             │
│                                                                │ volume      │
│                                                                ▼             │
│                                                        edge-data (SQLite)    │
│                                                                              │
│  edge-controller ──HTTP 8081──► cloud-api ──5432──► postgres                 │
│                                                       │                      │
│                                                       ▼                      │
│                                                  cloud-data (volume)         │
│                                                                              │
│  exposed to host:  edge-controller 8080 (REST + /metrics)                    │
│                    cloud-api       8081                                      │
│                    mosquitto       1883 (dev only)                           │
└──────────────────────────────────────────────────────────────────────────────┘

Host (not containerised):
  rhizo-ui  (Tauri desktop app, M12) ──HTTP──► localhost:8080
```

### Compose services

| Service | Image / build | Depends on | Notes |
|---|---|---|---|
| `mosquitto` | `eclipse-mosquitto:2` | — | config + passwd from `deploy/mosquitto/` |
| `edge-controller` | built from workspace | mosquitto (healthy) | named volume for SQLite |
| `device-simulator` | built from workspace | mosquitto (healthy) | scalable; one device id per instance |
| `cloud-api` | built from workspace | postgres (healthy) | |
| `postgres` | `postgres:16` | — | named volume |

Health checks gate startup order so the edge does not race the broker. The edge
tolerates a missing broker anyway (failure-model §1.1) — the health check just
makes logs readable.

### Test topology

`deploy/docker-compose.test.yml` overlays the base file to:

- set `RHIZO_SIM__TIME_SCALE=600` (10 simulated minutes per real second)
- shorten the control tick and TTLs to match accelerated time
- disable restart policies so a crash fails the test rather than being papered over
- add a `scenario-runner` service that drives and asserts M8 scenarios

```bash
docker compose -f deploy/docker-compose.yml -f deploy/docker-compose.test.yml \
  up --abort-on-container-exit --exit-code-from scenario-runner
```

### Why the UI is not in Compose

The UI is a Tauri **desktop application**, not a web service
([ADR-009](../adr/009-ui-architecture-and-rust-web-stack.md)). It runs on the
host and talks to `localhost:8080`. It is a M12 deliverable and is deliberately
**not** part of the M8 software-only acceptance environment, so that
`docker compose up` remains the complete, headless, CI-runnable definition of
the system.

The Edge REST API is kept transport-agnostic and CORS-capable so that a
browser-hosted frontend could be added later without touching the Edge
Controller. Building that second frontend is out of scope for V1.

---

## 2. Home deployment — the V1 target

```text
┌─────────────────────────── Home LAN (192.168.x.x) ───────────────────────────┐
│                                                                              │
│   ESP32 plant node #1 ─┐                                                     │
│   ESP32 plant node #2 ─┼── Wi-Fi ──► Raspberry Pi 4 / mini-PC                │
│   ESP32 plant node #N ─┘             ┌──────────────────────────────┐        │
│                                      │ mosquitto      (systemd)     │        │
│                                      │ edge-controller(systemd)     │        │
│                                      │ SQLite on internal storage   │        │
│                                      └──────────────┬───────────────┘        │
│                                                     │                        │
│   Operator laptop ── rhizo-ui (Tauri) ──HTTP────────┘                        │
│                                                                              │
└───────────────────────────────────┬──────────────────────────────────────────┘
                                    │ optional, outbound only
                                    ▼
                          Cloud API + PostgreSQL
```

Deployment choices for the home target:

- **systemd units, not Docker**, on a Raspberry Pi. Docker on a Pi adds an I/O
  layer over an SD card for no benefit at this scale, and systemd gives better
  restart semantics and journal integration. Compose remains the *development*
  and *CI* environment. Unit files are an M13 deliverable.
- **SQLite on internal storage, not the SD card** where possible. SD card wear
  from WAL writes is the most likely long-term hardware failure of this
  deployment. Retention (§4) keeps the database small.
- **API bound to the LAN address**, not `0.0.0.0`, and not exposed through the
  router. There is no authentication in V1; the network boundary is the
  security boundary ([ADR-011](../adr/011-configuration-and-secrets-model.md) §5).
- **Cloud is opt-in and outbound-only.** No inbound connection to the home
  network is ever required.

### What must keep working when the Internet drops

Everything in the box above the dashed line: telemetry, storage,
recommendations, automatic watering, safety lockouts, the local API, and the UI.
This is SAFETY-008 and it is the whole point.

---

## 3. Device-to-broker topology

```text
ESP32 ──Wi-Fi/TCP──► Mosquitto :1883
        client_id = device_id
        username  = device_id            (per-device credentials)
        LWT       = rhizo/v1/devices/{id}/status, retained, QoS 1
        clean_session = true             (see below)
```

**`clean_session = true` is deliberate.** A persistent session would have the
broker queue water commands for an offline device — precisely the scenario
SAFETY-002 exists to defeat. Since commands are short-lived by design and
telemetry is a stream of samples rather than a ledger, there is nothing worth
queuing. The one exception, command *results*, is handled by device-side retry
rather than by broker persistence.

Retained messages (status, config, policy) are a broker-level feature independent of
session persistence, so they still work.

---

## 4. Storage sizing and retention

One device at a 300-second telemetry interval produces 288 sampling cycles per
day. Since [ADR-017](../adr/017-extensible-measurement-model.md) the
`measurements` table is narrow — **one row per sample, not per cycle** — so a
device publishing six measurement kinds writes about 1 700 rows per day. Budget
~150 bytes/row with indexes:

| Devices | Rows/year | SQLite size/year |
|---|---|---|
| 1 | ~630 k | ~100 MB |
| 5 | ~3.2 M | ~500 MB |
| 20 | ~12.6 M | ~2 GB |

The roughly sixfold increase over the previous wide-column design is the accepted
cost of an extensible measurement set. The 90-day raw retention below caps the
working set well under these annual figures, and M13 hourly downsampling bounds
it further.

Retention defaults (edge):

- `measurements`: M3 deletes eligible raw rows older than 90 days. M13 first
  adds hourly downsampling so long-range aggregate history survives that deletion.
- `processed_messages`: transport markers are kept 7 days. This bounded fast
  path is safe because durable effects have independent stable uniqueness/order
  protection; it is not justified by assumptions about broker redelivery time.
- `pending_cloud_events` with `status='synced'`: 24 hours.
- `watering_events`, `device_events`, `commands`: **never auto-pruned.** This is
  the ledger of what the machine did to a living plant.

The cloud has no retention limit in V1.

---

## 5. Resource expectations

| Component | RAM | Notes |
|---|---|---|
| `edge-controller` | < 64 MB | dominated by the SQLite page cache |
| `device-simulator` | < 16 MB each | |
| `mosquitto` | < 32 MB | |
| `cloud-api` + postgres | < 512 MB | dev only; not on the Pi |
| `esp32-node` | ~120 KB free heap target | ESP32-C3 has 400 KB SRAM |

A Raspberry Pi 4 with 2 GB comfortably runs the home deployment including the
broker.

---

## 5b. Optional observability profile (M13)

An opt-in Compose profile adds Prometheus and Grafana:

```bash
docker compose --profile observability up -d
```

Without the flag nothing changes, and the M8 acceptance suite never references
it. Operational metrics are scraped from `/metrics`; **plant history is not
exported to Prometheus** and is read from SQLite or PostgreSQL through a SQL
datasource. Grafana is an engineering dashboard, read-only, and never the way a
normal user learns whether a plant is safe
([ADR-010](../adr/010-observability-strategy.md)).

## 5c. Site time source

Devices take their wall clock from the edge **over the MQTT connection they
already have** — there is no NTP daemon to install, supervise, or firewall, and
no time-server field to configure
([ADR-013](../adr/013-clock-and-time-semantics.md),
[mqtt-v1.md](../protocol/mqtt-v1.md) §5.12).

The edge host itself should keep its own clock disciplined by the usual means
(systemd-timesyncd, chrony, or whatever the OS provides). That is ordinary host
administration, not a Rhizo Edge component.

Without this arrangement, losing the internet would leave every device with an
unsynchronised clock and SAFETY-002 would refuse every water command site-wide —
an internet outage would become an irrigation outage
([connectivity-modes.md](connectivity-modes.md) §3).

## 6. Future topologies (M13–M14, planning only)

### Greenhouse

```text
several ESP32 zone controllers ──Wi-Fi──► edge hub (mini-PC)
solenoid valves per zone                  irrigation zones
flow meter per zone                       ambient sensors
```

The change is quantitative (more devices, zones as a first-class entity), not
architectural. The data model already avoids assuming one measurement point per
plant ([ADR-004](../adr/004-sqlite-edge-persistence-model.md) §6).

### Server-side Kubernetes (optional, M14 planning only)

Helm packaging is a **possible later option for server-side components only**:
`cloud-api`, and optionally Prometheus and Grafana. PostgreSQL only when it is
not operator-managed or external.

**The plant-side edge controller is explicitly excluded.** It sits metres from a
pump and must work when the network is down; putting it behind a scheduler adds
failure modes to the one component whose purpose is surviving failure. Home
deployment stays Compose or systemd, and **Kubernetes is never required for an
indoor plant deployment** (M14-007).

### Field

```text
multi-depth probe ──RS485──► field controller ──LoRaWAN/LTE-M/NB-IoT──►
   gateway ──MQTT──► edge platform
```

The MQTT contract is the seam: a LoRaWAN gateway translates into the same
`rhizo/v1/...` topics, so the Edge Controller does not learn about radios. This
is the payoff of keeping transport concerns out of `rhizo-mqtt-contract`.

Constraints that will force real design work later, recorded now so they are not
a surprise: duty-cycle limits make 5-minute telemetry impossible on LoRaWAN;
payloads must become binary rather than JSON; command TTL semantics must widen;
and battery devices sleep, which breaks the "always connected" assumption behind
Last Will. These are M14 planning topics, not V1 problems. See
[PRD 140](../prd/140-field-readiness.md).
