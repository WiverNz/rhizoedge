# ADR-004 — SQLite edge persistence model

## Status

Accepted — 2026-08-25. Implemented in M3.

## Context

The edge must survive crashes without re-watering (SAFETY-010), must deduplicate
QoS 1 redeliveries across restarts (SAFETY-001), and must compute a rolling
24-hour water total that cannot be reset by a restart (SAFETY-006). All three
require durable state with real transactions.

The deployment target is a Raspberry Pi with an SD card, one writer process, and
tens of writes per minute.

## Decision

### SQLite via `sqlx`, WAL mode

- **SQLite**, not Postgres-at-the-edge: no separate process to supervise, no
  network, atomic file backup, and it comfortably handles three orders of
  magnitude more write volume than this workload.
- **`sqlx`**, not `rusqlite` or `diesel`: async-native so it composes with Tokio
  without `spawn_blocking` wrappers, and compile-time-checked queries via
  `sqlx::query!` catch schema/code drift at build time. `sqlx migrate` gives
  embedded migrations with no runtime file dependency.
- **WAL mode**, `synchronous = NORMAL`, `busy_timeout = 5000`, `foreign_keys = ON`.
  WAL allows the API's reads to proceed during the pipeline's writes.
  `NORMAL` rather than `FULL` because WAL + NORMAL survives process crashes
  (the case we care about) and only risks the last transactions on OS/power
  loss — an acceptable trade for a 10× reduction in SD card wear.

### One writer

All writes go through the pipeline task. Other tasks read. This is not enforced
by the type system but by the repository API shape: write methods take
`&mut Transaction`, and only the pipeline owns a transaction factory. It reduces
`SQLITE_BUSY` to near zero and makes reasoning about ordering tractable.

### Timestamps as INTEGER milliseconds

Every time column is `INTEGER NOT NULL` holding Unix epoch milliseconds UTC.
Rationale in [time-model.md](../architecture/time-model.md) §2: efficient
indexing, unambiguous comparison, no timezone ambiguity, sub-second resolution
for pump durations.

### Schema

```sql
-- Identity and lifecycle -----------------------------------------------------
CREATE TABLE devices (
    device_id              TEXT PRIMARY KEY,
    name                   TEXT,
    firmware_version       TEXT,
    boot_id                TEXT,
    last_sequence          INTEGER,
    status                 TEXT NOT NULL DEFAULT 'unknown',  -- online|offline|unknown
    clock_synced           INTEGER NOT NULL DEFAULT 0,
    last_seen_at           INTEGER,
    desired_config_version INTEGER NOT NULL DEFAULT 0,
    applied_config_version INTEGER,
    created_at             INTEGER NOT NULL
);

CREATE TABLE plant_profiles (
    profile_id   TEXT PRIMARY KEY,
    name         TEXT NOT NULL,
    profile_json TEXT NOT NULL,       -- validated on write (configuration-model L4)
    updated_at   INTEGER NOT NULL
);

CREATE TABLE plants (
    plant_id              TEXT PRIMARY KEY,
    device_id             TEXT NOT NULL REFERENCES devices(device_id),
    profile_id            TEXT NOT NULL REFERENCES plant_profiles(profile_id),
    name                  TEXT NOT NULL,
    species               TEXT,
    pot_volume_ml         REAL,
    soil_type             TEXT,
    auto_watering_enabled INTEGER NOT NULL DEFAULT 0,   -- opt-in, SAFETY-012
    lockout_reason        TEXT,
    lockout_since         INTEGER,
    created_at            INTEGER NOT NULL
);

-- Ingestion ------------------------------------------------------------------
CREATE TABLE processed_messages (
    message_id  TEXT PRIMARY KEY,
    device_id   TEXT NOT NULL,
    kind        TEXT NOT NULL,
    received_at INTEGER NOT NULL
);
CREATE INDEX idx_processed_received ON processed_messages(received_at);

CREATE TABLE measurements (
    id                 INTEGER PRIMARY KEY AUTOINCREMENT,
    device_id          TEXT NOT NULL REFERENCES devices(device_id),
    measurement_point  TEXT NOT NULL DEFAULT 'default',   -- multi-depth ready (PRD 140)
    received_at        INTEGER NOT NULL,   -- edge clock: AUTHORITATIVE
    device_time_ms     INTEGER,            -- device clock: advisory only
    boot_id            TEXT,
    sequence           INTEGER,
    moisture_vwc       REAL,
    soil_temperature_c REAL,
    ec_us_cm           INTEGER,
    pot_weight_g       REAL,
    tank_level_percent REAL,
    leak_detected      INTEGER
);
CREATE INDEX idx_meas_device_time ON measurements(device_id, received_at DESC);

CREATE TABLE device_events (
    event_id    TEXT PRIMARY KEY,
    device_id   TEXT NOT NULL,
    kind        TEXT NOT NULL,       -- offline|boot|leak|sensor_invalid|…
    severity    TEXT NOT NULL,       -- info|warning|critical
    detail_json TEXT,
    occurred_at INTEGER NOT NULL
);
CREATE INDEX idx_devevents_device_time ON device_events(device_id, occurred_at DESC);

CREATE TABLE quarantined_messages (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    topic       TEXT NOT NULL,
    payload     BLOB,                -- truncated to 1 KiB
    error       TEXT NOT NULL,
    received_at INTEGER NOT NULL
);

-- Control --------------------------------------------------------------------
CREATE TABLE commands (
    command_id   TEXT PRIMARY KEY,        -- UUIDv7, idempotency key
    device_id    TEXT NOT NULL,
    plant_id     TEXT REFERENCES plants(plant_id),
    kind         TEXT NOT NULL,           -- water|tare|calibrate
    requested_ml REAL,
    mode         TEXT NOT NULL,           -- manual|recommended|automatic
    issued_at    INTEGER NOT NULL,
    expires_at   INTEGER NOT NULL,
    status       TEXT NOT NULL,           -- issued|in_flight|completed|rejected
                                          -- |expired|failed|interrupted
    published_at INTEGER,
    settled_at   INTEGER,
    reason       TEXT
);
CREATE INDEX idx_commands_open ON commands(status, expires_at);

CREATE TABLE watering_events (
    watering_event_id TEXT PRIMARY KEY,
    plant_id          TEXT NOT NULL REFERENCES plants(plant_id),
    command_id        TEXT UNIQUE REFERENCES commands(command_id),  -- NULL = detected
    mode              TEXT NOT NULL,     -- manual|recommended|automatic|detected
    started_at        INTEGER NOT NULL,
    completed_at      INTEGER,
    requested_ml      REAL,
    delivered_ml      REAL,
    status            TEXT NOT NULL,
    reason_json       TEXT
);
CREATE INDEX idx_watering_plant_time ON watering_events(plant_id, completed_at DESC);

CREATE TABLE irrigation_state (
    plant_id               TEXT PRIMARY KEY REFERENCES plants(plant_id),
    state                  TEXT NOT NULL,
    state_since            INTEGER NOT NULL,
    doses_this_cycle       INTEGER NOT NULL DEFAULT 0,
    cycle_started_at       INTEGER,
    last_cycle_completed_at INTEGER,
    wait_until             INTEGER,
    active_command_id      TEXT REFERENCES commands(command_id),
    updated_at             INTEGER NOT NULL
);

-- Cloud outbox ---------------------------------------------------------------
CREATE TABLE pending_cloud_events (
    event_id        TEXT PRIMARY KEY,     -- UUIDv7, cloud idempotency key
    kind            TEXT NOT NULL,
    value_tier      TEXT NOT NULL,        -- 'high' | 'low'  (pruning policy)
    payload_json    TEXT NOT NULL,
    status          TEXT NOT NULL,        -- pending|synced|quarantined
    attempts        INTEGER NOT NULL DEFAULT 0,
    next_attempt_at INTEGER NOT NULL,
    last_error      TEXT,
    created_at      INTEGER NOT NULL,
    synced_at       INTEGER
);
CREATE INDEX idx_outbox_ready ON pending_cloud_events(status, next_attempt_at);
```

### The persist-and-dedup transaction

The mechanism behind SAFETY-001 and SAFETY-010:

```rust
let mut tx = pool.begin().await?;

let inserted = sqlx::query!(
    "INSERT INTO processed_messages (message_id, device_id, kind, received_at)
     VALUES (?, ?, ?, ?) ON CONFLICT(message_id) DO NOTHING",
    message_id, device_id, kind, received_at
).execute(&mut *tx).await?.rows_affected();

if inserted == 0 {
    tx.rollback().await?;          // duplicate: no effects, ever
    metrics::duplicate_messages(kind);
    return Ok(Outcome::Duplicate);
}

// … measurement insert, device update, outbox insert …

tx.commit().await?;                // all or nothing
```

The dedup marker and the effects share a transaction. There is no window in
which one is durable without the other.

### `commands.command_id` is a natural primary key

Not a surrogate `INTEGER` id with a unique index. Making the idempotency key the
primary key means a duplicate insert fails at the storage layer rather than
relying on application logic to check first — the guarantee holds even if
someone writes the check-then-insert race.

### Retention

`processed_messages` older than 7 days, `pending_cloud_events` with
`status='synced'` older than 24 hours, and `quarantined_messages` beyond 1000
rows are pruned by a periodic task. `measurements` are pruned at 90 days.

`watering_events`, `commands`, and `device_events` are **never auto-pruned** —
they are the record of what the machine did to a living thing.

The 7-day dedup horizon must exceed the longest plausible redelivery window. A
broker holds QoS 1 messages only for the duration of a session, so the real
window is minutes; 7 days is deliberate over-provisioning at a cost of a few
megabytes.

### Migrations

`sqlx::migrate!()` embedded from `migrations/edge/`. Forward-only, numbered,
never edited after being applied anywhere. Migrations run automatically at
startup before any other subsystem — a controller that cannot migrate must not
serve traffic or make decisions.

## Alternatives considered

**`rusqlite` with `spawn_blocking`.** Rejected: workable, but every repository
call becomes a closure across a thread boundary, which makes transactions
awkward to compose and error handling noisier. `sqlx` compile-time query
verification is also a real defect-prevention win for a schema this size.

**Dedup in an in-memory LRU.** Rejected outright: it does not survive a restart,
which is precisely the SAFETY-010 case. Considered as a *cache in front of* the
table and rejected as premature — the table lookup is a primary-key hit.

**Storing timestamps as ISO-8601 TEXT.** Rejected: larger indexes, string
comparison, and it invites accidental local-time storage.

**Event-sourcing the whole edge.** Rejected as over-engineering for one plant,
though the outbox is an event log for the subset that matters.

**Postgres at the edge.** Rejected: a second supervised process and a network
socket, to serve one writer at tens of writes per minute.

## Consequences

Positive:

- Crash safety is a property of the transaction boundary, not of careful coding.
- Backup is `sqlite3 edge.sqlite ".backup out.sqlite"` or a file copy of a
  checkpointed WAL — trivial to automate.
- `sqlx::query!` means a schema change that breaks a query fails the build.

Negative, accepted:

- `sqlx::query!` requires a database at compile time or a checked-in
  `.sqlx/` offline cache. We check in the offline cache and CI verifies it is
  current (issue M3-004), which is one more thing that can go stale.
- SQLite has no native `TIMESTAMPTZ`, `UUID`, or `JSONB`; we use TEXT/INTEGER
  and validate in Rust.
- A single-writer design means a future need for concurrent writers is a
  refactor, not a configuration change. Judged very unlikely at this scale.

## Risks

- **SD card wear on a Pi.** WAL + `synchronous=NORMAL` + 90-day retention keeps
  write volume low, but SD cards remain the most likely hardware failure.
  *Mitigation:* documented recommendation to place the database on internal or
  USB storage ([deployment-model.md](../architecture/deployment-model.md) §2),
  and a documented backup procedure (issue M13-008).
- **Migration failure on a device with real history.** *Mitigation:* migrations
  are forward-only and additive; a pre-migration backup is taken automatically
  when the schema version changes (issue M3-003).
- **`processed_messages` growth if the retention task dies.** *Mitigation:* the
  retention task is supervised like every other task; its failure exits the
  process (failure-model §3.6), and `storage_bytes` is a monitored gauge.

## Follow-up

- [PRD 030](../prd/030-edge-ingestion-and-storage.md) — full data model requirements.
- M3-003…M3-010 implement schema, migrations, repositories, and the dedup transaction.
- [ADR-005](005-cloud-event-model-and-idempotency.md) — the corresponding cloud schema.
