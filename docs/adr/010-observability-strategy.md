# ADR-010 — Observability strategy

## Status

Accepted — 2026-08-25. Baseline in M0, extended per milestone.

**Extended 2026-08-26** with the operational-metrics / plant-history data-class
split and the optional Grafana profile (§Two data classes, §Grafana).

## Context

[failure-model.md](../architecture/failure-model.md) asserts that a failure is
not handled until it is observable. That commitment needs concrete mechanics:
what is logged, in what shape, which metrics exist, and how health is reported.

The system is unattended for weeks at a time on a Raspberry Pi with no operator
watching. When something goes wrong with a plant, the question is always "what
did the system think was happening, and when?" — which is a question about
correlation, not about log volume.

## Decision

### Structured logging with `tracing`

`tracing` + `tracing-subscriber`, JSON in production, pretty in development
(`RHIZO_EDGE__LOG__FORMAT`).

**Correlation fields.** Every span and event carries the identifiers relevant to
its context, as structured fields — never interpolated into the message string:

```text
device_id       plant_id        message_id      command_id
event_id        boot_id         sequence        watering_event_id
```

```rust
// wrong — unsearchable
info!("watering plant {} with {} ml", plant_id, ml);

// right
info!(plant_id = %plant_id, requested_ml = ml, mode = %mode, "issuing dose");
```

**Spans mark units of work**, so every log line inside inherits correlation:

```rust
#[tracing::instrument(skip(self, payload), fields(device_id = %device_id, message_id))]
async fn handle_message(&self, device_id: DeviceId, payload: &[u8]) -> Result<()>
```

**Levels, used consistently:**

| Level | Meaning | Examples |
|---|---|---|
| ERROR | needs a human; the system is degraded | disk full, task panic, cloud 4xx quarantine |
| WARN | self-healing but notable | MQTT reconnect, sensor invalid, SQLITE_BUSY retry |
| INFO | significant state change | dose issued, lockout set/cleared, device online/offline, startup recovery |
| DEBUG | per-message pipeline detail | message decoded, dedup hit |
| TRACE | development only | raw payloads |

The rule that keeps INFO useful: **INFO is for things that changed the world.**
Receiving a telemetry message did not change the world; issuing a dose did.
At a 300-second interval with a handful of devices, INFO should produce a few
lines per hour, which is a log a human can actually read after a two-week
absence.

**Redaction.** The config `Debug` impl prints `[redacted]` for fields named
`password`, `token`, or `secret`. Raw MQTT payloads are logged at TRACE only.

### Metrics: Prometheus text format at `/metrics`

Pull-based, exposed by the edge (`:8080/metrics`) and the cloud
(`:8081/metrics`). No push gateway, no agent — a Prometheus scrape or a `curl`
in a terminal both work.

**Metric catalogue for V1** — deliberately small. A large catalogue nobody reads
is worse than a small one that answers the real questions:

```text
# Ingestion
mqtt_messages_received_total{kind}
mqtt_decode_errors_total{reason}
mqtt_duplicate_messages_total{kind}
mqtt_reconnects_total
mqtt_connection_state                       gauge 0|1
measurements_processed_total{kind}
sensor_errors_total{sensor,reason}

# Devices
devices_online                              gauge
devices_offline                             gauge
device_restarts_total{device_id}

# Control
watering_commands_total{mode,outcome}
watering_delivered_ml_total{mode}
watering_failures_total{reason}
irrigation_state_transitions_total{from,to}
plants_locked_out                           gauge, by reason via label
lockouts_total{reason}

# Cloud
pending_cloud_events                        gauge
cloud_sync_attempts_total{outcome}
cloud_events_quarantined_total
cloud_last_success_timestamp_seconds        gauge

# Storage / runtime
sqlite_busy_total
storage_bytes                               gauge
task_panics_total{task}

# Latency (histograms)
mqtt_processing_duration_seconds
control_tick_duration_seconds
cloud_sync_duration_seconds
http_request_duration_seconds{route,status}
```

**Cardinality discipline.** `device_id` appears as a label only on
`device_restarts_total`, where the cardinality equals the (small) device count
and the per-device breakdown is the whole point. It is deliberately absent from
high-frequency counters — per-device ingestion rates are visible in the logs and
would otherwise multiply every series by the fleet size.

The three most operationally valuable series, worth stating explicitly:
`pending_cloud_events` (is sync healthy?), `plants_locked_out` (is anything
stuck?), and `cloud_last_success_timestamp_seconds` (how long has sync been
broken?).

### Health endpoints

```text
GET /health/live    → 200 while the process is running and no task has panicked
GET /health/ready   → 200 only when: migrations applied
                                   AND MQTT connected
                                   AND control loop ticked within 3 intervals
```

The distinction is load-bearing: `live` answers "should the supervisor restart
me?", `ready` answers "am I actually doing my job?". A process with a dead MQTT
connection is alive but not ready, and a supervisor that restarted it would
achieve nothing while an operator dashboard showing "not ready" is exactly right.

**Cloud reachability is deliberately excluded from readiness.** An edge with the
cloud down is fully functional (SAFETY-008); reporting it unready would be a
lie that could trigger a pointless restart loop.

`/health/ready` returns a JSON body listing each check, so a failing readiness
is self-diagnosing:

```json
{
  "ready": false,
  "checks": {
    "migrations": "ok",
    "mqtt": "disconnected since 2026-08-25T11:22:03Z",
    "control_loop": "ok, last tick 4s ago"
  }
}
```

### Diagnostic events as first-class data

Beyond logs and metrics, notable occurrences are persisted as rows in
`device_events` and exposed through the API. This is the difference between
observability for the operator (who sees "Leak detected 3 days ago" in the UI)
and observability for the developer (who greps the log).

Persisted event kinds: `boot`, `offline`, `online`, `leak`, `sensor_invalid`,
`sequence_regression`, `clock_unsynced`, `clock_skew`, `clock_step`,
`config_drift`, `pump_fault`, `no_delivery`, `calibration_drift`.

Every state transition of the irrigation machine is also persisted, so the
history of "what did the system think" is reconstructable months later — the
question that actually gets asked when a plant dies.

### Task supervision

Every long-running task is spawned under a supervisor that:

1. logs the panic with full context at ERROR,
2. increments `task_panics_total{task}`,
3. **exits the process non-zero.**

A process that is up but not evaluating safety is worse than a down process,
because supervision and alerting see "healthy" while nothing is watching the
plant. Failing loudly is the safe behaviour (failure-model §3.6).

### Two data classes — and why Grafana must not blur them

There are two kinds of time series in this system, and conflating them is the
mistake that makes Prometheus fall over:

| | Operational metrics | Plant history |
|---|---|---|
| Examples | `mqtt_messages_received_total`, `plants_locked_out`, tick duration | soil moisture, EC, weight, watering events, threshold crossings |
| Question answered | "is the software healthy?" | "is the plant healthy?" |
| Store | Prometheus | SQLite (edge) / PostgreSQL (cloud) |
| Cardinality | low, bounded, no `plant_id` on hot paths | one series per plant per measurement kind |
| Retention | days–weeks | years; downsampled, never aggregated away for the ledger |
| Required for operation | no | **yes** — this is the control loop's input |

**Raw plant telemetry does not go into Prometheus.** It is tempting, because
Grafana reads Prometheus and that would make dashboards free. It is wrong: it
would put per-plant, per-kind, per-point series into a store designed for
low-cardinality operational data, with retention semantics that are wrong for a
ledger, and it would make a monitoring system a dependency of irrigation.

Plant history is queried from the databases that already hold it.

### Grafana — optional, additive, never required

Grafana is an **optional deployment profile**, planned in M13
([PRD 130](../prd/130-multi-plant-home.md)), not a component of the product.

```text
operational metrics  →  Prometheus  →  Grafana
plant history        →  SQLite / PostgreSQL  →  Tauri UI (primary)
                                             →  Grafana via SQL datasource (optional)
```

Hard boundaries:

- **Nothing depends on it.** Monitoring, recommendations, watering, offline
  autonomy, and alerts all function with Grafana absent, uninstalled, and
  unheard of. It is not in the M8 acceptance environment.
- **It is not how a normal user learns whether a plant is safe.** That is the
  Tauri UI's job ([ADR-009](009-ui-architecture-and-rust-web-stack.md)). Grafana
  is an engineering and operations surface: fleet dashboards, long-range
  history, correlation while debugging.
- **It is read-only.** No control path, no configuration, no actuation.

Rejected: making Grafana the operator UI. It cannot express a safety refusal, it
has no notion of a lockout that needs an explicit human reset, and a dashboard
that shows a chart is not a substitute for an interface that says *why watering
is blocked and what will clear it*.

### What is deliberately not done in V1

- **No distributed tracing / OpenTelemetry exporter.** There are two hops and
  one operator; `tracing` fields provide the correlation. The subscriber is
  structured so an OTel layer can be added later without touching call sites.
- **No log shipping.** `journalctl` and `docker logs` are sufficient.
- **No Grafana in V1's required path.** Planned as an optional M13 profile
  (see above); nothing before it may depend on it.
- **No alerting rules.** The metrics are shaped so Prometheus alerting rules can
  be written; writing them is a deployment concern (M13).
- **No per-message metric labels** beyond `kind` — see cardinality above.

## Alternatives considered

**`log` + `env_logger`.** Rejected: no spans, so no automatic correlation, and
structured fields would be hand-formatted into strings — unsearchable.

**Push-based metrics (StatsD, push gateway).** Rejected: adds a dependency that
must itself be available, for a system whose entire premise is working when
things are unavailable.

**OpenTelemetry from day one.** Rejected as premature for a two-service system;
the collector would be a third service to run and supervise.

**A comprehensive metric catalogue up front.** Rejected explicitly. Metrics
nobody looks at cost cardinality and maintenance; the catalogue grows when a
real question cannot be answered.

**Cloud reachability in `/health/ready`.** Rejected — see above; it would
contradict SAFETY-008.

## Consequences

Positive:

- Every log line about a plant can be correlated by `plant_id` or `command_id`
  across ingestion, control, and sync.
- The three-series dashboard (`pending_cloud_events`, `plants_locked_out`,
  `cloud_last_success_timestamp_seconds`) answers most operational questions.
- Persisted device events mean the operator-facing story does not depend on log
  retention.
- A silently broken control loop is impossible: it either ticks or the process
  exits.

Negative, accepted:

- JSON logs are unpleasant to read raw. Mitigated by pretty format in
  development and by `jq` in production.
- `#[instrument]` on hot paths has measurable overhead. At this message rate it
  is irrelevant; the annotation is omitted on per-sample inner functions.
- Persisting device events adds write volume. Bounded by their low frequency and
  covered by retention.

## Risks

- **Metric cardinality creep** as someone adds `device_id` to a hot counter.
  *Mitigation:* the catalogue above is normative; additions are reviewed, and
  M3-012 adds a test asserting the exported series count stays under a threshold
  for a fixed fixture.
- **Log volume from a misbehaving device** flooding the disk. *Mitigation:*
  decode errors are rate-limited per device (10/min) before logging, matching
  the quarantine rate limit.
- **`/health/ready` flapping** during normal MQTT reconnects. *Mitigation:* the
  MQTT check tolerates a disconnect shorter than one reconnect-backoff cycle
  before reporting unready.

## Follow-up

- M0-006 sets up `rhizo-telemetry` with the subscriber and registry.
- M3-011 adds the ingestion metrics; M3-012 adds the cardinality test.
- M4-007 adds health endpoints.
- M6-014 adds control and lockout metrics.
- M7-009 adds cloud sync metrics.
