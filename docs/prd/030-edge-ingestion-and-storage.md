# PRD 030 — Edge Ingestion and Storage

**Milestone:** M3 · **Status:** PLANNED · **Depends on:** M1, M2

## Summary

The Edge Controller consumes MQTT telemetry, validates it, deduplicates QoS 1
redeliveries durably, and persists measurements and events to SQLite — with a
transaction boundary that makes crash recovery correct by construction.

## Problem

QoS 1 is at-least-once. A naive consumer creates duplicate rows on every
reconnect, and a consumer that deduplicates in memory creates duplicates on
every restart. Both defects become watering defects once M6 adds actuation, so
the mechanism must be right before there is anything to actuate.

## Goals

1. An MQTT consumer that reconnects and **re-subscribes** reliably.
2. Validation that is lenient per-field and strict per-message.
3. Durable deduplication sharing a transaction with the effects it guards.
4. SQLite schema, migrations, and repositories per
   [ADR-004](../adr/004-sqlite-edge-persistence-model.md).
5. Graceful shutdown and correct restart recovery.
6. Ingestion metrics and structured logs.

## Non-goals

- Device health and online/offline semantics (M4).
- Plant state and recommendations (M5).
- Any command issue or actuation (M6).
- Cloud sync (M7) — the `pending_cloud_events` table is created here but not
  drained.

## User/system flows

```text
simulator publishes → Mosquitto → edge mqtt_ingress
   → parse topic → decode envelope → check identity → stamp received_at
   → [ TRANSACTION: dedup insert → validate fields → measurement insert
                    → device touch → outbox insert → COMMIT ]
   → in-memory latest-sample cache refreshed
```

Restart:

```text
start → run migrations (Fatal on failure) → open pool (WAL)
      → restore device registry from SQLite
      → subscribe → resume
```

## Functional requirements

### Ingestion

| ID | Requirement |
|---|---|
| F-030-01 | `rumqttc` event loop with reconnect using the M0 backoff (base 1 s, cap 60 s, unlimited) |
| F-030-02 | **All subscriptions re-established on every reconnect**; never assumed to survive |
| F-030-03 | Topic parsed before payload; unknown topics dropped with a metric |
| F-030-04 | Envelope decoded; `v != 1` rejected |
| F-030-05 | Topic `device_id` vs payload `device_id` mismatch → rejected and quarantined |
| F-030-06 | `received_at` stamped from the **edge** clock, and is authoritative |
| F-030-07 | Malformed messages quarantined, capped at 1000 rows, rate-limited to 10/min/device |
| F-030-08 | Out-of-range fields stored as `NULL` with a `sensor_invalid` device event; the message is **not** discarded |
| F-030-09 | `sequence` regression within a `boot_id` recorded as an event, not rejected |
| F-030-10 | `boot_id` change recorded as a `boot` event; sequence restart not flagged |

### Deduplication and persistence

| ID | Requirement |
|---|---|
| F-030-20 | `INSERT INTO processed_messages … ON CONFLICT DO NOTHING`; 0 rows affected ⇒ duplicate |
| F-030-21 | **The dedup marker and every effect share one transaction.** Rollback on duplicate applies nothing. |
| F-030-22 | Dedup key is `message_id` alone |
| F-030-23 | All writes flow through the single pipeline task |
| F-030-24 | `SQLITE_BUSY` retried 3× with 50/100/200 ms jitter, then a clean failure that leaves the message unprocessed |
| F-030-25 | WAL, `synchronous=NORMAL`, `busy_timeout=5000`, `foreign_keys=ON` |

### Schema and migrations

| ID | Requirement |
|---|---|
| F-030-30 | Migrations from `migrations/edge/`, embedded, forward-only |
| F-030-31 | Migrations run before any other subsystem; failure is **Fatal** |
| F-030-32 | Automatic backup taken when the schema version changes |
| F-030-33 | Tables per [ADR-004](../adr/004-sqlite-edge-persistence-model.md) |
| F-030-34 | `.sqlx` offline cache committed; CI verifies it is current |

### Lifecycle

| ID | Requirement |
|---|---|
| F-030-40 | Graceful shutdown: stop accepting, finish the in-flight transaction, close the pool, exit 0 |
| F-030-41 | Retention task prunes `processed_messages` > 7 d, synced outbox rows > 24 h, quarantine > 1000 rows |
| F-030-42 | Every long-running task is supervised; a panic logs, increments `task_panics_total`, and **exits the process non-zero** |

## Interfaces

```rust
// rhizo-storage
pub struct EdgeDb { pool: SqlitePool }
impl EdgeDb {
    pub async fn connect(path: &Path) -> Result<Self, StorageError>;
    pub async fn migrate(&self) -> Result<(), StorageError>;
    pub async fn begin(&self) -> Result<Transaction<'_>, StorageError>;
}

pub struct Ingest<'t>(&'t mut Transaction<'t>);
impl<'t> Ingest<'t> {
    /// Returns Duplicate without applying anything if message_id was seen.
    pub async fn mark_processed(&mut self, id: Uuid, device: &DeviceId, kind: MessageKind)
        -> Result<Dedup, StorageError>;
    pub async fn insert_measurement(&mut self, m: &MeasurementRow) -> Result<(), StorageError>;
    pub async fn touch_device(&mut self, d: &DeviceTouch) -> Result<(), StorageError>;
    pub async fn record_event(&mut self, e: &DeviceEventRow) -> Result<(), StorageError>;
    pub async fn enqueue_cloud(&mut self, e: &OutboxRow) -> Result<(), StorageError>;
}

pub enum Dedup { New, Duplicate }
```

No HTTP interface yet — the read API is M4.

## Data model

Exactly the schema in [ADR-004](../adr/004-sqlite-edge-persistence-model.md).
M3 creates all tables (so later milestones add rows rather than migrations
during feature work) but only populates `devices`, `measurements`,
`device_events`, `processed_messages`, `quarantined_messages`, and
`pending_cloud_events`.

Indexes that matter for M3:

```sql
idx_meas_device_time  ON measurements(device_id, received_at DESC)
idx_processed_received ON processed_messages(received_at)
```

The first serves "latest sample", which the control loop will call every tick
for every plant from M6 onward.

## State model

The ingestion pipeline is stateless per message; all state is in SQLite. The
MQTT connection has a small lifecycle:

```text
Disconnected ──► Connecting ──► Connected ──► Subscribed
      ▲                                            │
      └──────────────── error / drop ──────────────┘
```

`Subscribed` is a distinct state from `Connected` because F-030-02 requires
re-subscription; a connection that is up but unsubscribed is not ready.

## Failure modes

Per [failure-model.md](../architecture/failure-model.md) §1 and §3. The ones M3
must demonstrate:

| Failure | Behaviour |
|---|---|
| Broker down at startup | edge starts; `/health/ready` not ready (endpoint lands in M4); reconnect loop runs |
| Broker restart | reconnect **and re-subscribe**; retained messages redelivered |
| Duplicate QoS 1 | rolled back, counted, no effect |
| Malformed payload | quarantined, pipeline continues |
| One bad field | stored as NULL, message kept |
| `SQLITE_BUSY` | retried, then the message is left unprocessed for redelivery |
| Disk full | Fatal for the write; loud ERROR |
| Migration failure | Fatal; process exits before serving |
| Pipeline task panic | logged, counted, process exits non-zero |

## Safety implications

M3 delivers the **mechanism** for SAFETY-001 and SAFETY-010 even though neither
can be violated yet (there are no commands until M6):

- **SAFETY-001** — F-030-20/21. The transaction boundary is what makes duplicate
  suppression survive a crash. Tested here as
  `it_duplicate_qos1_creates_one_row`, and again in M6 against commands.
- **SAFETY-010** — the same boundary, plus F-030-31's fail-fast migration.
- **SAFETY-005** — F-030-06 establishes `received_at` as authoritative, which is
  what makes staleness computable in M6. Using device time here would quietly
  break SAFETY-005 three milestones later.
- **SAFETY-012** — F-030-08's per-field nulling means a bad EC reading does not
  discard a good moisture reading, and a missing field is `None` rather than a
  default.

## Observability

Metrics introduced ([ADR-010](../adr/010-observability-strategy.md)):

```text
mqtt_messages_received_total{kind}      mqtt_decode_errors_total{reason}
mqtt_duplicate_messages_total{kind}     mqtt_reconnects_total
mqtt_connection_state                   measurements_processed_total{kind}
sensor_errors_total{sensor,reason}      sqlite_busy_total
storage_bytes                           task_panics_total{task}
mqtt_processing_duration_seconds
```

Logging: DEBUG per message, WARN for decode errors and reconnects, INFO for
connection state changes and startup recovery, ERROR for Fatal conditions.
`device_id` and `message_id` on every pipeline event.

A cardinality test (M3-012) asserts the exported series count stays below a
threshold for a fixed fixture, guarding against `device_id` creeping onto hot
counters.

## Testing strategy

- Unit: dedup transaction (new, duplicate, rollback-leaves-nothing); range
  validation nulling; `classify()` per error variant; migration idempotency.
- Integration with a real broker: SCEN-001, SCEN-010, SCEN-012, SCEN-013,
  SCEN-014, SCEN-050, SCEN-053.
- Restart: ingest 50, restart, ingest 50, assert 100 rows and no duplicates.
- Broker restart: assert **re-subscription** specifically, not merely reconnection —
  a reconnect without re-subscribe is silent data loss and is easy to miss.

## Acceptance criteria

- [ ] Simulator telemetry appears in `measurements` with `received_at` set from
      the edge clock.
- [ ] Publishing the same `message_id` twice produces one row and increments
      `mqtt_duplicate_messages_total`.
- [ ] Restarting the edge preserves all history; the device registry is restored
      from SQLite.
- [ ] Restarting Mosquitto results in reconnection **and** re-subscription;
      telemetry resumes without operator action.
- [ ] A message with `moisture_vwc: 150` is stored with `moisture_vwc = NULL`,
      the other fields intact, and a `sensor_invalid` event recorded.
- [ ] Invalid JSON is quarantined and the next valid message is processed.
- [ ] `SIGTERM` shuts down cleanly with exit 0 and no partial transaction.
- [ ] A forced panic in the pipeline task exits the process non-zero.

## Dependencies

- M1 (contract types), M2 (a device to ingest from), M0 (telemetry, config, CI).

## Open questions

1. **Whether to cache "latest sample" in memory** or query per control tick.
   M3 caches for the read path; M6's control loop reads from SQLite regardless
   ([ADR-006](../adr/006-irrigation-state-machine-ownership.md)), so the cache is
   an optimisation, not a source of truth. Decided in M3-010.
2. **Retention default for `measurements`** (90 days) — a starting figure;
   downsampling is deferred to M13.

## Future work

- Hourly downsampling of old measurements (M13).
- Multi-measurement-point ingestion beyond the reserved column (M14).
- Backup/restore tooling (M13).
