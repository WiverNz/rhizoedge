# HTTP API Boundaries

Two HTTP surfaces exist: the **Edge REST API** (local operations, consumed by
the UI) and the **Cloud API** (event ingestion and historical reads). This
document defines both, and — more importantly — what neither is allowed to do.

---

## 1. The boundary rule

```text
UI → Edge REST API → domain safety gate → MQTT command → device veto → pump
```

**Every actuation request MUST pass through the domain safety gate.** There is
no endpoint, parameter, header, or debug mode that bypasses it. An HTTP handler
that publishes MQTT directly is a defect against
[ADR-006](../adr/006-irrigation-state-machine-ownership.md).

The UI has no MQTT client dependency, so the shortcut does not compile
([ADR-009](../adr/009-ui-architecture-and-rust-web-stack.md)).

---

## 2. Edge REST API

Base: `http://{edge}:8080/api/v1`
Bind: `127.0.0.1:8080` by default; a LAN address is an explicit configuration
choice. No authentication in V1 — the network is the boundary
([ADR-011](../adr/011-configuration-and-secrets-model.md) §5).

Content type `application/json`. Timestamps are RFC 3339 with `Z`.

### 2.1 Conventions

**Errors** use a consistent envelope:

```json
{
  "error": {
    "code": "safety_lockout",
    "message": "Watering is blocked: leak detected",
    "details": { "reason": "leak", "since": "2026-08-25T09:12:00Z" }
  }
}
```

| Status | Meaning |
|---|---|
| 200 | success |
| 201 | resource created |
| 202 | command accepted and published; the result is asynchronous |
| 400 | malformed request |
| 404 | unknown resource |
| **409** | **refused by a safety rule or a state conflict** |
| 422 | semantically invalid (e.g. a profile that fails validation) |
| 503 | the controller is not ready (see `/health/ready`) |

**409 is the safety response.** It carries the structured lockout reason so the
UI can explain *why* rather than showing a generic failure. It is not an error
in the system; it is the system working.

### 2.2 Health and metrics

```text
GET /health/live       200 while the process is running and no task has panicked
GET /health/ready      200 only when migrations applied, MQTT connected, and the
                       control loop ticked within 3 intervals; body lists checks
GET /metrics           Prometheus text format
```

Cloud reachability is deliberately **not** a readiness input (SAFETY-008).

### 2.3 Devices

```text
GET  /api/v1/devices
GET  /api/v1/devices/{device_id}
PATCH /api/v1/devices/{device_id}            { "name": "Window sill node" }
GET  /api/v1/devices/{device_id}/events?since=&limit=
PUT  /api/v1/devices/{device_id}/config      → 202, bumps config_version
POST /api/v1/devices/{device_id}/commands/tare       → 202
POST /api/v1/devices/{device_id}/commands/calibrate  { "run_seconds": 10 } → 202
```

`GET /api/v1/devices/{id}`:

```json
{
  "device_id": "plant-node-01",
  "name": "Window sill node",
  "status": "online",
  "firmware_version": "0.1.0",
  "clock_synced": true,
  "last_seen_at": "2026-08-25T11:30:00Z",
  "sample_age_seconds": 42,
  "connectivity": "connected",
  "power_mode": "battery",
  "wake_interval_seconds": 900,
  "expected_wake_at": "2026-08-25T11:45:00Z",
  "missed_wake_count": 0,
  "config": {
    "desired_version": 7,
    "applied_version": 7,
    "drift": false
  },
  "limits": { "max_run_seconds": 20, "max_ml_per_run": 80.0, "max_daily_ml": 500.0 },
  "sensors": {
    "soil":  { "present": true, "healthy": true },
    "tank":  { "present": true, "healthy": true },
    "leak":  { "present": true, "healthy": true },
    "weight":{ "present": false, "healthy": true }
  }
}
```

`PATCH` changes the **display name only**. `device_id` is immutable — changing
it would orphan history ([ADR-012](../adr/012-device-identity-and-provisioning.md)).

`connectivity` is one of `connected`, `sleeping`, `isolated`, or `reconciling`
([connectivity-modes.md](../architecture/connectivity-modes.md) §1b). It is
**derived at read time**, never stored, for the same reason `sample_age_seconds`
is.

`expected_wake_at` is present only while `connectivity` is `sleeping`, and is
computed from the **edge's** `received_at` for the sleep announcement plus the
announced `wake_interval_seconds`. Every other state reports `null`, including a
device that has just woken and one that went overdue: a wake instant that has
already been missed is not an expectation. A device past its window derives
`isolated` — never `sleeping` (SAFETY-021).

The deadline is re-checked on **every read**, so an overdue sleeper reports
`isolated` whether or not the liveness timer has run. `missed_wake_count` is the
one stored half, because it counts events rather than describing the present, and
it may therefore still be zero on the first read after a missed wake.

`power_mode` is `always_on` or `battery`. Until the edge publishes device
configuration (M5-019), its source is the device's own `power` declaration on
`device.status`: `battery` while the device declares it, `always_on` on any
other explicit declaration, and unchanged when a status carries no `power` block
at all. `wake_interval_seconds` is the interval of the most recent accepted sleep
announcement and is cleared when the device stops declaring battery mode. Neither
field is a safety input, and neither widens the freshness threshold the
irrigation gate uses (SAFETY-005).

### 2.4 Plants

```text
GET    /api/v1/plants
POST   /api/v1/plants
GET    /api/v1/plants/{plant_id}
PATCH  /api/v1/plants/{plant_id}
DELETE /api/v1/plants/{plant_id}
GET    /api/v1/plants/{plant_id}/measurements?from=&to=&resolution=
GET    /api/v1/plants/{plant_id}/recommendation
GET    /api/v1/plants/{plant_id}/watering-events?from=&to=&limit=
```

`GET /api/v1/plants/{id}`:

```json
{
  "plant_id": "monstera-01",
  "name": "Monstera",
  "species": "Monstera deliciosa",
  "device_id": "plant-node-01",
  "profile_id": "monstera_default",
  "state": "Healthy",
  "irrigation_state": "Normal",
  "auto_watering_enabled": true,
  "lockout": null,
  "latest": {
    "measured_at": "2026-08-25T11:30:00Z",
    "age_seconds": 42,
    "moisture_vwc": 34.1,
    "soil_temperature_c": 21.8,
    "ec_us_cm": 920,
    "pot_weight_g": 5312.4,
    "tank_level_percent": 72.0,
    "leak_detected": false
  },
  "water_budget": {
    "delivered_last_24h_ml": 80.0,
    "max_daily_ml": 300.0,
    "remaining_ml": 220.0
  },
  "last_watering": {
    "completed_at": "2026-08-21T08:14:00Z",
    "mode": "automatic",
    "delivered_ml": 40.0
  }
}
```

A locked-out plant carries:

```json
"lockout": {
  "reason": "leak",
  "since": "2026-08-25T09:12:00Z",
  "clearable": false,
  "message": "Leak detected. Watering is disabled until the leak is resolved and reset manually."
}
```

`clearable: false` means the condition must physically resolve first; the UI
renders no clear button in that case.

### 2.5 Recommendation

```text
GET /api/v1/plants/{plant_id}/recommendation
```

```json
{
  "recommendation": "water",
  "recommended_ml": 40.0,
  "confidence": 0.87,
  "reasons": [
    { "code": "moisture_below_target", "vwc": 24.1, "target_min": 28.0 },
    { "code": "dry_for", "minutes": 42, "required": 30 },
    { "code": "last_watering", "hours_ago": 148.2 }
  ],
  "blocked_by": null,
  "evaluated_at": "2026-08-25T11:30:00Z"
}
```

`recommendation` ∈ `water | no_water | blocked`. When `blocked`, `blocked_by`
carries the lockout. Reasons are structured, not prose, so the UI renders them
and tests assert on them ([ADR-006](../adr/006-irrigation-state-machine-ownership.md)).

### 2.6 Watering actions — the safety-critical endpoints

```text
POST /api/v1/plants/{plant_id}/water
     { "ml": 30.0, "mode": "manual" }        mode ∈ manual | recommended
```

Behaviour:

1. Load plant, profile, and irrigation state.
2. Run the domain safety gate. `manual` skips only the `SensorFault` and
   `StaleData` checks; every other check applies
   ([ADR-006](../adr/006-irrigation-state-machine-ownership.md)).
3. On refusal → **409** with the lockout reason. There is no override parameter.
4. On acceptance → persist the command, publish MQTT, return **202**:

```json
{
  "command_id": "018fd7b1-4c2e-7f10-a3b8-9d1e2f304050",
  "status": "issued",
  "requested_ml": 30.0,
  "expires_at": "2026-08-25T11:32:00Z"
}
```

The result is asynchronous. The caller polls the command or the plant.

#### A sleeping device: `pending_for_device_wake`

A battery device is reachable for a few seconds per wake interval. Publishing to
it immediately would deliver a command whose TTL expired while it slept, so the
edge instead persists an **intent** and mints the command at the next wake, after
re-running the full safety gate against current inputs
([ADR-018](../adr/018-battery-and-deep-sleep-device-mode.md) §3).

```json
{
  "intent_id": "018fd7c9-8a11-7003-b21c-5e7f80112233",
  "status": "pending_for_device_wake",
  "requested_ml": 30.0,
  "expected_delivery_after": "2026-08-25T11:45:00Z",
  "intent_expires_at": "2026-08-25T12:15:00Z"
}
```

Still **202**, and deliberately a different shape. **There is no `command_id`**,
because no command exists yet — the field is *absent* rather than null, so a
client that reads it unconditionally fails loudly instead of polling a null id.
`pending_for_device_wake` and `sent` are distinct states and must be presented
distinctly (M12-018); a pending dose is never rendered as sent.

Intent states: `pending_for_device_wake` → `sent` | `refused` |
`expired_before_wake`. On delivery the intent gains the `command_id` that was
allocated at that moment, so a caller can follow the handover.

Two expiries, deliberately separate. `intent_expires_at` is the edge's clock and
bounds how long it will keep trying; the wire TTL is unchanged at 120 s and is
what the device validates (SAFETY-002). `intent_expires_at` never reaches a
device.

**At most one open water intent per plant.** A second request while one is
pending returns **409** naming the pending intent, so an impatient operator
cannot queue several doses that all deliver at one wake. There is no parameter
that wakes a device, expedites a dose, or overrides any of this.

```text
GET  /api/v1/intents/{intent_id}
GET  /api/v1/commands/{command_id}
POST /api/v1/plants/{plant_id}/auto-watering/enable
POST /api/v1/plants/{plant_id}/auto-watering/disable
POST /api/v1/plants/{plant_id}/lockout/clear     { "reason": "leak" }
```

`lockout/clear` requires the condition to have physically cleared. Attempting to
clear a still-active leak returns 409. This is the explicit reset SAFETY-003
requires.

### 2.7 Profiles

```text
GET   /api/v1/profiles
POST  /api/v1/profiles
GET   /api/v1/profiles/{profile_id}
PUT   /api/v1/profiles/{profile_id}
```

Validation failures return **422** with the specific violated rule — for example
`dose_ml (200) exceeds the device hard limit FIRMWARE_MAX_ML_PER_RUN (80)`.
Values are rejected, never silently clamped
([ADR-011](../adr/011-configuration-and-secrets-model.md)).

### 2.8 Overview and sync status

```text
GET /api/v1/overview
```

One composite response for the UI dashboard — the single permitted
UI-shaped endpoint:

```json
{
  "edge_id": "home-01",
  "plants": [ /* summary per plant */ ],
  "devices_online": 1,
  "devices_offline": 0,
  "plants_locked_out": 0,
  "cloud": {
    "enabled": true,
    "reachable": false,
    "pending_events": 1423,
    "last_success_at": "2026-08-25T08:02:11Z"
  },
  "control_loop": { "last_tick_at": "2026-08-25T11:30:12Z", "healthy": true }
}
```

```text
GET /api/v1/sync/status
GET /api/v1/sync/quarantined?limit=
GET /api/v1/quarantined-messages?limit=
```

### 2.9 CORS

Disabled by default. `RHIZO_EDGE__API__CORS_ALLOWED_ORIGINS` enables specific
origins. This exists so a browser-hosted frontend can be added later without
Edge Controller changes ([ADR-009](../adr/009-ui-architecture-and-rust-web-stack.md));
V1 ships with it off.

---

## 3. Cloud API

Base: `http://{cloud}:8081/api/v1`

### 3.1 Ingestion

```text
POST /api/v1/edges/{edge_id}/events
```

```json
{ "events": [ { "event_id": "018f…", "kind": "measurement.soil", "occurred_at": "…", "payload": { } } ] }
```

Response — **200 with per-event results, even on partial failure**:

```json
{
  "results": [
    { "event_id": "018f…20", "status": "accepted" },
    { "event_id": "018f…21", "status": "duplicate" },
    { "event_id": "018f…22", "status": "rejected", "error": "unknown kind" }
  ]
}
```

- `duplicate` is a **success**; the edge marks the event synced.
- `rejected` quarantines that one event; the batch proceeds.
- 4xx only for a malformed request envelope; 5xx for genuine server faults.

Maximum batch size 500 events, 5 MiB. See
[ADR-005](../adr/005-cloud-event-model-and-idempotency.md).

### 3.2 Reads

```text
GET /api/v1/edges
GET /api/v1/edges/{edge_id}/devices
GET /api/v1/edges/{edge_id}/plants
GET /api/v1/edges/{edge_id}/plants/{plant_id}/measurements?from=&to=&resolution=
GET /api/v1/edges/{edge_id}/plants/{plant_id}/watering-events
GET /health/live   /health/ready   /metrics
```

### 3.3 What the Cloud API MUST NOT have

- **No command endpoints.** The cloud cannot water anything.
- **No configuration write endpoints.** The cloud pushes no desired state in V1
  ([ADR-003](../adr/003-edge-first-ownership-model.md)).
- **No endpoint the edge polls for instructions.** The edge→cloud relationship
  is write-only plus acknowledgement.

These absences are the architecture. An endpoint added here would move the
system from edge-first to cloud-influenced, and would require revisiting
SAFETY-008 and SAFETY-009.

---

## 4. Pagination and time ranges

Time-series endpoints accept `from`, `to` (RFC 3339), and `limit` (default 500,
max 5000). `resolution` ∈ `raw | minute | hour | day` selects server-side
downsampling; `raw` is capped at 5000 points so a year-long request cannot
exhaust memory.

List endpoints use cursor pagination:

```json
{ "items": [ ], "next_cursor": "eyJ0IjoxNzU2MTIxNDAwMDAwfQ" }
```

---

## 5. Versioning

The URL carries `/api/v1`. Additive changes (new fields, new endpoints) happen
within v1; clients ignore unknown fields. Removals, renames, and semantic
changes require `/api/v2`. Same discipline as the MQTT contract
([versioning-policy.md](versioning-policy.md)).
