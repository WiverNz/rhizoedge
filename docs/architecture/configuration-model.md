# Configuration Model

Configuration is layered by **who owns it** and **what may change it**. The
critical property: no layer reachable from the network can weaken a hard safety
limit.

Decision record: [ADR-011](../adr/011-configuration-and-secrets-model.md).

---

## 1. The five layers

```text
┌──────────────────────────────────────────────────────────────┐
│ L5  Plant instance settings      owner: operator (UI/API)    │
│     auto_watering_enabled, profile assignment, plant name    │
├──────────────────────────────────────────────────────────────┤
│ L4  Plant profile                owner: operator (UI/API)    │
│     moisture targets, dose sizes, cooldown, daily cap,       │
│     EC thresholds                                            │
├──────────────────────────────────────────────────────────────┤
│ L3  Device runtime config        owner: edge, device-applied │
│     telemetry interval, pump calibration, tank minimum,      │
│     sensor enable flags       (retained MQTT, versioned)     │
├──────────────────────────────────────────────────────────────┤
│ L2  Edge instance config         owner: deployer             │
│     broker URL, DB path, cloud URL, log level, tick period,  │
│     edge_id                    (TOML file + env overrides)   │
├──────────────────────────────────────────────────────────────┤
│ L1  Firmware HARD SAFETY LIMITS  owner: nobody at runtime    │
│     FIRMWARE_MAX_RUN_SECONDS, FIRMWARE_MAX_ML_PER_RUN,       │
│     FIRMWARE_MAX_DAILY_ML, MAX_CLOCK_SKEW                    │
│     compile-time constants — changing them requires          │
│     reflashing the device                                    │
└──────────────────────────────────────────────────────────────┘
```

**The rule that makes this safe:** a lower layer always wins. L4 may request a
120 ml dose; if L1 caps a single run at 60 ml, the device delivers at most 60 ml
and reports `clamped`. There is no message, config topic, API call, or cloud
field that can raise an L1 value (SAFETY-007).

---

## 2. What each layer contains

### L1 — Firmware hard limits (compile-time)

```rust
pub const FIRMWARE_MAX_RUN_SECONDS: u32 = 20;
pub const FIRMWARE_MAX_ML_PER_RUN: f32 = 80.0;
pub const FIRMWARE_MAX_DAILY_ML: f32 = 500.0;
pub const MAX_CLOCK_SKEW_SECONDS: i64 = 5;
pub const COMMAND_DEDUP_RING: usize = 16;
```

These live in `rhizo-mqtt-contract` (so the simulator enforces the identical
values) and are compiled into both the simulator and the firmware. They are
deliberately generous relative to normal use and tight relative to damage: 80 ml
is more than any single sensible houseplant dose and far less than a flood.

Not present in any config topic. Not readable as mutable state. The device
reports them in its status message for observability, but reporting is one-way.

### L2 — Edge instance config

Source order, later wins:

```text
1. built-in defaults (in code)
2. /etc/rhizo/edge.toml  (or ./edge.toml in dev)
3. RHIZO_EDGE__* environment variables
4. command-line flags (a small set: --config, --log-level)
```

Nested keys use a double underscore: `RHIZO_EDGE__MQTT__BROKER_URL`.

```toml
edge_id = "home-01"          # stable identity, used as the cloud partition key

[mqtt]
broker_url = "mqtt://localhost:1883"
client_id  = "rhizo-edge-home-01"
username   = "rhizo-edge"
# password comes from RHIZO_EDGE__MQTT__PASSWORD, never the file

[storage]
path = "/var/lib/rhizo/edge.sqlite"

[control]
tick_interval_seconds = 30
command_ttl_seconds   = 120

[cloud]
enabled  = false             # cloud is opt-in; absence is the default
base_url = "http://localhost:8081"

[api]
bind = "127.0.0.1:8080"      # loopback by default; widening is explicit

[log]
level  = "info"
format = "json"
```

Secrets are never written to the TOML file. See §5.

### L3 — Device runtime config

Edge-owned desired state, delivered as a **retained** MQTT message on
`rhizo/v1/devices/{device_id}/config` with QoS 1.

```json
{
  "v": 1,
  "config_version": 7,
  "telemetry_interval_seconds": 300,
  "pump": { "ml_per_second": 8.2, "enabled": true },
  "tank": { "min_percent": 15.0 },
  "sensors": { "soil": true, "weight": false, "tank": true, "leak": true }
}
```

- `config_version` is a monotonically increasing `u32` owned by the edge.
- The device persists the applied config in NVS and echoes
  `applied_config_version` in every status message.
- The edge compares desired vs applied and exposes the drift through the API.
  Persistent drift raises a `config_drift` device event.
- Retention means a device that boots days later still gets current config with
  no edge-side tracking of who is awake.

**`pump.ml_per_second` is a calibration value, not a safety limit.** A wrong
calibration changes accuracy; it cannot exceed L1 bounds because the firmware
also clamps on *duration*.

### L4 — Plant profile

Stored in edge SQLite, editable through the API/UI, versioned by
`updated_at`. Profiles are reusable across plants.

```yaml
id: monstera_default
name: "Monstera — peat/perlite mix"

moisture:
  target_min_vwc: 28.0
  target_max_vwc: 48.0
  recovery_delta_vwc: 6.0        # rise that counts as "responded to water"
  dry_confirm_minutes: 30        # continuous dryness before DRY_CONFIRMED

watering:
  dose_ml: 40.0
  max_doses_per_cycle: 3
  absorption_wait_minutes: 15
  cooldown_hours: 6
  max_daily_ml: 300.0
  command_ttl_seconds: 120

sensors:
  max_sample_age_minutes: 15     # floor; effective value is max(this, 3× interval)
  tank_min_percent: 15.0

ec:
  warning_high_us_cm: 1800
```

Validation on write rejects incoherent profiles: `target_min >= target_max`,
`dose_ml * max_doses_per_cycle > max_daily_ml`, non-positive intervals,
`dose_ml > FIRMWARE_MAX_ML_PER_RUN` (rejected with an explanatory error rather
than silently clamped, so the operator learns the real limit).

### L5 — Plant instance

```text
plants.auto_watering_enabled   bool, operator-controlled, default false
plants.profile_id              FK
plants.name, species, pot_volume_ml, soil_type
plants.lockout_reason          set by the system, cleared explicitly
```

`auto_watering_enabled` defaults to **false** for a newly created plant. Opting
in to automation is an explicit act (SAFETY-012).

---

## 3. Change matrix

| Setting | UI/API | Edge config file | Retained MQTT | Cloud | Requires reflash |
|---|---|---|---|---|---|
| Firmware hard limits | ✗ | ✗ | ✗ | ✗ | ✓ |
| Pump calibration | ✓ | ✗ | ✓ (delivery) | ✗ | ✗ |
| Telemetry interval | ✓ | ✗ | ✓ (delivery) | ✗ | ✗ |
| Tank minimum | ✓ | ✗ | ✓ (delivery) | ✗ | ✗ |
| Plant profile values | ✓ | ✗ | ✗ | ✗ | ✗ |
| Auto-watering on/off | ✓ | ✗ | ✗ | ✗ | ✗ |
| Broker URL, DB path | ✗ | ✓ | ✗ | ✗ | ✗ |
| Cloud URL / enabled | ✗ | ✓ | ✗ | ✗ | ✗ |
| Anything at all | ✗ | ✗ | ✗ | **✗ — cloud pushes no config in V1** | — |

The last row is a deliberate V1 scope decision, not an oversight. Cloud-pushed
desired state is a real feature with real failure modes (split brain between two
edges, stale desired state overriding local reality) and it is not needed for
the V1 goal. It is deferred to M14 planning.

---

## 4. Configuration and safety interaction

Two guards prevent configuration from becoming a safety bypass:

1. **Clamping is one-directional and reported.** Any value the device clamps
   produces a `command_result` with `clamped: true` and the effective value, so
   over-ambitious configuration is visible rather than silent.
2. **The edge validates against L1 at write time.** The API rejects a profile
   with `dose_ml = 200` when `FIRMWARE_MAX_ML_PER_RUN = 80`, because a config
   the device will always clamp is a misconfiguration the operator should know
   about immediately, not at 3 a.m.

---

## 5. Secrets

V1 threat model: a trusted home LAN, one operator, no multi-tenancy.

| Secret | Storage (dev) | Storage (home deployment) |
|---|---|---|
| MQTT broker password (edge) | `.env`, gitignored | environment variable from a systemd unit with `EnvironmentFile=` mode 0600 |
| MQTT per-device credentials | `deploy/mosquitto/passwd` (gitignored, generated) | same, generated per device at provisioning |
| PostgreSQL password | `.env` / compose env | environment variable |
| Cloud API token | not used in V1 | reserved for M14 |

Rules:

- No secret is ever committed. `deploy/mosquitto/passwd` and `.env` are in
  `.gitignore`; `.env.example` documents the shape with placeholder values.
- No secret appears in a config TOML that might be copied into a bug report.
- No secret is logged. The `tracing` setup redacts fields named `password`,
  `token`, `secret` — enforced by a `Debug` impl on the config type that prints
  `[redacted]`.
- Anonymous MQTT access is disabled in the Mosquitto configuration from M0.

Future (M13/M14): per-device X.509 certificates, TLS on the broker, signed
firmware, cloud authentication. Explicitly out of scope for M0–M12 so that
security work does not block the software milestones.
