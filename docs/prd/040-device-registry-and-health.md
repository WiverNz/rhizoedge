# PRD 040 — Device Registry and Health

**Milestone:** M4 · **Status:** IMPLEMENTED · **Depends on:** M3

> **Revised 2026-08-26.** The registry now records **declared capabilities**
> ([ADR-016](../adr/016-plant-binding-and-policy-model.md)) and exposes
> **connectivity mode** — connected, isolated, or reconciling
> ([connectivity-modes.md](../architecture/connectivity-modes.md)). Issues M4-011
> and M4-012 were added.
>
> Capabilities matter because the edge must never assume what a device can do:
> a binding naming an undeclared sensor or actuator is rejected. Connectivity
> mode matters because "offline and monitoring only" and "offline and watering
> itself" look identical if the UI can only say *offline*.
>
> **Additional acceptance criteria:** a device with no actuators is represented
> correctly rather than as an error; a capability lost across a reboot raises an
> event; `reconciling` is a distinct, queryable state, because it is the window
> in which the edge must not issue a dose.

## Summary

Track device lifecycle — online/offline via retained status and Last Will,
last-seen, firmware and protocol version, config drift, sensor health, and
sample staleness — and expose it through the first Edge REST API endpoints.

## Problem

M3 stores measurements but has no notion of a device being *there*. Without it,
a plant whose device died silently looks identical to a plant with fresh data
whose last message happened to be a while ago. Staleness and liveness are the
inputs SAFETY-005 depends on, so they must exist before automatic watering does.

## Goals

1. Online/offline state derived from retained status and LWT.
2. Auto-registration of unknown devices — **as devices, never as plants**.
3. Sample staleness computation from `received_at`.
4. Sensor health tracking from the status message.
5. Config desired-vs-applied drift detection.
6. The first REST endpoints and health endpoints.

## Non-goals

- Plant entities and profiles (M5).
- Any watering (M6).
- Device provisioning tooling (M9/M13) — this PRD covers what happens after a
  device appears, not how it gets credentials.
- Authentication on the API (deferred; see
  [ADR-011](../adr/011-configuration-and-secrets-model.md) §5).

## User/system flows

**A new device appears:**

```text
device connects → retained status published → edge ingests
   → device_id unknown → INSERT INTO devices (status='online')
   → NO plant is created
   → operator sees it in GET /api/v1/devices and attaches a plant in M5
```

**A device dies:**

```text
power lost → broker detects keepalive timeout → publishes retained LWT
   → edge ingests status='offline' → device marked offline
   → device_event(kind='offline') recorded
   → sample_age grows → (from M6) plants on that device lock out
```

**Config drift:**

```text
operator changes config → edge bumps config_version → publishes retained
   → device applies, echoes applied_config_version in status
   → edge compares → drift=false
   (if the device never echoes) → after 2 telemetry intervals → config_drift event
```

## Functional requirements

| ID | Requirement |
|---|---|
| F-040-01 | Retained `device.status` with `status: "online"` marks the device online |
| F-040-02 | A logically new LWT payload (`status: "offline"`) marks it offline; replay of that current boot's fixed LWT identity is a no-op even after transport-marker pruning |
| F-040-03 | Unknown `device_id` is auto-registered with `status` from the message and **no plant attached** |
| F-040-04 | `last_seen_at` is updated by every newly accepted logical message effect; a transport duplicate or logically old status receipt does not refresh it |
| F-040-05 | `firmware_version`, `protocol_version`, `boot_id`, `clock_synced` recorded from status |
| F-040-06 | Sensor presence and health recorded per sensor from the status message |
| F-040-07 | `sample_age_seconds` computed as `now − max(received_at)` from the **edge** clock |
| F-040-08 | Staleness threshold `max(15 min, 3 × telemetry_interval)` exposed and used for a `stale` flag |
| F-040-09 | Stale device detection runs on a timer, not only on message arrival — a device that stops publishing produces no event to react to |
| F-040-10 | `desired_config_version` vs `applied_config_version` compared; drift exposed and evented after 2 intervals |
| F-040-11 | `clock_synced: false` recorded and surfaced as a device-level condition |
| F-040-17 | On receiving **any** `device.status`, the edge publishes `edge.time` to that device — live, `retain=false`, QoS 1 |
| F-040-18 | While a device is online, the edge republishes `edge.time` at least every `TIME_SYNC_INTERVAL_SECONDS` (300 s) |
| F-040-19 | The edge never assumes a reconnecting device is synchronised; it reads `clock_synced` from the device's status |
| F-040-12 | Device display name is patchable; `device_id` is immutable |
| F-040-13 | REST endpoints per [http-api-boundaries.md](../protocol/http-api-boundaries.md) §2.3 |
| F-040-14 | `/health/live` and `/health/ready` per [ADR-010](../adr/010-observability-strategy.md), with a per-check JSON body |
| F-040-15 | `/metrics` exposes the Prometheus text format |
| F-040-16 | API binds to `127.0.0.1` by default; CORS disabled by default, configurable |

F-040-09 deserves emphasis: liveness must be evaluated by a timer. A design that
only updates staleness when a message arrives can never notice a device that
stopped sending — which is precisely the failure it exists to catch.

## Interfaces

```text
GET   /api/v1/devices
GET   /api/v1/devices/{device_id}
PATCH /api/v1/devices/{device_id}          { "name": "…" }
GET   /api/v1/devices/{device_id}/events?since=&limit=
GET   /health/live
GET   /health/ready
GET   /metrics
```

Response shapes are specified in
[http-api-boundaries.md](../protocol/http-api-boundaries.md) §2.3.

`PUT /devices/{id}/config` and the tare/calibrate command endpoints are listed
there but land in M6, when there is a command pipeline to carry them.

## Data model

Uses the `devices` and `device_events` tables from
[ADR-004](../adr/004-sqlite-edge-persistence-model.md). M4 adds no migrations —
the columns were created in M3. This is deliberate: schema churn during feature
work is how migrations end up edited after being applied.

Columns populated by M4: `status`, `last_seen_at`, `firmware_version`,
`boot_id`, `clock_synced`, `applied_config_version`, `name`.

M3 also stores the bounded status-order high water in
`status_boot_generation`, `status_sequence`, and `status_lwt_message_id`. M4
must consume that persistence outcome rather than comparing device wall time or
creating a second status consumer.

Sensor health is stored as a JSON column on `devices` (`sensors_json`) rather
than a separate table — it is a small, whole-value snapshot from the latest
status message with no independent lifecycle.

## State model

```text
Device status:

  unknown ──first message──► online ◄──────┐
                               │           │
                        LWT / clean        │ status: online
                        offline msg        │
                               ▼           │
                            offline ───────┘

Orthogonal, derived per read:
  fresh    sample_age <= max_sample_age
  stale    sample_age >  max_sample_age
```

`stale` is deliberately **derived, not stored**. A stored flag would need a
writer, and the writer would be a timer that could fail silently. Deriving it at
read time from `last_seen_at` means it cannot be wrong.

The timer in F-040-09 exists only to emit *events* and update metrics — the
authoritative answer is always computed.

## Failure modes

| Failure | Behaviour |
|---|---|
| Status/LWT from an older boot arrives after reconnect | M3's persisted `boot_generation` order rejects it; the registry and `last_seen_at` stay unchanged, while the valid receipt still gets `edge.time` |
| Device publishes status with an unparseable body | quarantined per M3; device state unchanged |
| Two devices claim the same `device_id` | broker ACLs prevent it ([ADR-012](../adr/012-device-identity-and-provisioning.md)); if seen, `boot_id` thrashing is evented |
| Device never echoes `applied_config_version` | `config_drift` event after 2 intervals; drift exposed in the API |
| Broker down | all devices eventually stale; `/health/ready` reports not-ready |
| Clock step on the edge | `sample_age` computed from the stepped clock; the clock-step handling in M6 covers the safety consequence |

## Safety implications

M4 does not enforce an invariant directly — nothing can water yet — but it
provides two inputs that SAFETY-005 depends on entirely:

- **`sample_age` computed from `received_at`** (F-040-07). If this used
  `device_time_ms`, a device with a backwards clock would make stale data look
  fresh, and SAFETY-005 would fail silently in M6.
- **Sensor health and presence** (F-040-06), which distinguishes "the sensor
  reported an invalid value" from "there is no such sensor" — and both from
  "the sensor is fine". All three are lockout-relevant and must not be conflated.

**SAFETY-012 applied to onboarding:** F-040-03 auto-registers a device but never
creates a plant. A device that appears on the network therefore has no profile,
no `auto_watering_enabled`, and no path to actuation. A newly discovered device
cannot water anything.

Also relevant: `/health/ready` deliberately **excludes cloud reachability**
(SAFETY-008) — an edge with the cloud down is fully functional and must not
report otherwise.

## Observability

Metrics added:

```text
devices_online          gauge
devices_offline         gauge
device_restarts_total{device_id}
```

`device_id` appears as a label here and nowhere else on a hot path — the
cardinality equals the device count and the per-device breakdown is the point
([ADR-010](../adr/010-observability-strategy.md)).

Events persisted: `online`, `offline`, `boot`, `config_drift`, `clock_unsynced`,
`clock_skew`.

Logging: INFO on every online/offline transition (a real state change), DEBUG
for last-seen updates.

## Testing strategy

- Unit: staleness arithmetic including the 15-minute floor and the 3× rule;
  online/offline resolution when messages arrive out of order; drift detection
  timing.
- Integration: SCEN-020 (LWT offline), SCEN-021 (reconnect with new `boot_id`,
  sequence restart not flagged as regression), retained status seen on a fresh
  subscribe, auto-registration creating a device and **not** a plant.
- API: every endpoint's shape; 404 for unknown device; PATCH changes name only;
  `/health/ready` returns 200 with the cloud stopped.
- Timer: stop the simulator, assert the stale event fires without any inbound
  message.

## Acceptance criteria

- [ ] Killing the simulator moves the device offline within the keepalive window
      and records an `offline` event.
- [ ] Restarting it restores online with a new `boot_id`, and the sequence
      restart is **not** recorded as a regression.
- [ ] `GET /api/v1/devices/{id}` reports `sample_age_seconds`, sensor health,
      and config drift.
- [ ] An unknown device appears in `GET /api/v1/devices` with no plant attached.
- [ ] Stopping telemetry (device still connected) produces a stale indication
      from the timer, with no inbound message to trigger it.
- [ ] `/health/ready` returns 200 while the cloud container is stopped.
- [ ] `/health/ready` returns 503 with `mqtt: disconnected` while Mosquitto is
      stopped.
- [ ] `PATCH` changes the display name; there is no endpoint that changes
      `device_id`.

## Dependencies

- M3 (ingestion, storage, device rows).

## Open questions

1. **Offline detection latency** is bounded by the MQTT keepalive (60 s) plus
   broker grace. A shorter keepalive detects faster at the cost of traffic;
   60 s is chosen as a default and is configurable. Not blocking.
2. **Whether sensor health should be a separate table** if per-sensor history
   becomes interesting. JSON column for now; revisit in M10 when real sensors
   produce real fault histories.

## Future work

- Per-sensor health history and trend (M10).
- Device grouping and rooms (M13).
- Firmware update status and OTA (post-V1).
