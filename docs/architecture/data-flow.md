# Data Flow

Three pipelines matter: **telemetry ingestion**, **irrigation control**, and
**cloud synchronisation**. Each is described here with its ordering guarantees
and its failure behaviour.

---

## 1. Telemetry ingestion pipeline

```text
device publishes (QoS 1)
        │
        ▼
  Mosquitto
        │
        ▼
┌───────────────────────────────────────────────────────────┐
│ edge-controller: mqtt_ingress task                        │
│                                                           │
│  1. receive Publish                                       │
│  2. parse topic  ──────────────► unknown topic → drop+metric│
│  3. deserialize envelope ──────► malformed → quarantine    │
│  4. envelope.v == 1? ──────────► else → reject+metric      │
│  5. topic device_id == envelope.device_id? ──► else reject │
│  6. stamp received_at (edge clock, authoritative)          │
└───────────────────────┬───────────────────────────────────┘
                        ▼
┌───────────────────────────────────────────────────────────┐
│ pipeline task — ONE SQLite TRANSACTION                    │
│                                                           │
│  7. INSERT INTO processed_messages(message_id) …          │
│         ON CONFLICT DO NOTHING                            │
│     rows_affected == 0 ──► DUPLICATE: rollback, count,    │
│                            and stop. No effects applied.  │
│                                                           │
│  8. range-validate each field                             │
│       valid   → keep                                      │
│       invalid → field = NULL + device_event(sensor_invalid)│
│                                                           │
│  9. INSERT measurement row                                │
│ 10. UPDATE device last_seen_at, boot_id, sequence         │
│ 11. INSERT pending_cloud_events row (outbox)              │
│                                                           │
│     COMMIT  ── all of the above, or none of it            │
└───────────────────────┬───────────────────────────────────┘
                        ▼
        in-memory plant state cache refreshed
                        │
                        ▼
        notify control_loop (best effort, non-blocking)
```

### Why the transaction boundary is where it is

Steps 7–11 are atomic. If the process is killed at any point, either the message
is recorded as processed *and* its effects are durable, or neither is true and
the broker will redeliver it. This is the mechanism behind **SAFETY-001** and
**SAFETY-010**, and it is the reason dedup lives in SQLite rather than in a
`HashSet` in memory.

### Validation is lenient per-field, strict per-message

A soil telemetry message with a valid moisture and a wildly out-of-range EC is
**not** discarded. The moisture is stored, the EC is stored as `NULL`, and a
`device_event` of kind `sensor_invalid` is recorded. Discarding the whole
message would throw away the reading the safety logic actually needs.

The exception: if the message cannot be parsed or its identity fields are
inconsistent, nothing about it can be trusted and it is quarantined whole.

### Ordering

MQTT QoS 1 over a single connection preserves order per topic, but redelivery
after reconnect can produce **out-of-order and duplicated** messages. The
pipeline therefore assumes nothing about order:

- Measurements are stored with their own timestamps and queried by time, never
  by arrival order.
- "Latest reading" means `ORDER BY received_at DESC LIMIT 1`, not "the last one
  the process saw".
- A `sequence` that moves backwards within the same `boot_id` is recorded as a
  `device_event` (`sequence_regression`) but does not reject the message.

---

## 2. Irrigation control pipeline

The control loop is a periodic tick, not an event reaction. Reacting directly to
telemetry would make behaviour depend on message timing; a tick makes it
depend on state, which is testable.

```text
control_loop tick (default every 30 s, virtual-time aware)
        │
        ▼
  for each plant with a device:
        │
        ▼
  load IrrigationState from SQLite (never from memory alone)
        │
        ▼
  gather inputs: latest soil sample + age, tank, leak,
                 profile, delivered_today_ml, last cycle time
        │
        ▼
┌──────────────────────────────────────────────────┐
│ rhizo_domain::evaluate(inputs)  — PURE           │
│                                                  │
│  a. SAFETY GATE first, always:                   │
│       leak?          → Lock(Leak)                │
│       tank low?      → Lock(TankLow)             │
│       sample stale?  → Lock(StaleData)           │
│       sample invalid?→ Lock(SensorFault)         │
│       daily cap hit? → Lock(DailyLimit)          │
│       cooldown?      → Wait                      │
│       inputs missing?→ Lock(Uncertain)  ◄ SAFETY-012 │
│                                                  │
│  b. only if the gate passes, evaluate irrigation │
└───────────────────────┬──────────────────────────┘
                        ▼
             IrrigationDecision
                        │
      ┌─────────────────┼──────────────────┬─────────────┐
      ▼                 ▼                  ▼             ▼
    Idle           Recommend           IssueDose       Lock
      │                 │                  │             │
      │                 ▼                  ▼             ▼
      │        persist recommendation  ┌────────┐   persist lockout
      │        + reasons               │ dose   │   + device_event
      │                                └───┬────┘   + metric
      │                                    │
      │        ┌───────────────────────────┘
      │        ▼
      │  TRANSACTION:
      │    INSERT command(command_id, plant, ml, issued_at, expires_at,
      │                   status='issued')
      │    UPDATE irrigation_state → WaitingForResult
      │    INSERT pending_cloud_events
      │    COMMIT
      │        │
      │        ▼
      │  publish MQTT water command (QoS 1)
      │        │
      │        ▼
      │  device validates: TTL, dedup, hard limits, local lockouts
      │        │
      │        ├── refuse → command result(status=rejected, reason)
      │        └── accept → run pump → command result(status=completed, ml)
      │        │
      │        ▼
      │  edge ingests result (same dedup path as telemetry)
      │        │
      │        ▼
      │  TRANSACTION: update command status, INSERT watering_event,
      │               UPDATE irrigation_state → WaitForAbsorption,
      │               INSERT pending_cloud_events, COMMIT
      ▼
   next tick
```

### The command is persisted before it is published

Order matters and it is deliberate: the row is committed with status `issued`
**before** the MQTT publish. If the process dies between the two, recovery finds
an `issued` command with no result and can reconcile it — either by observing a
late result or by expiring it at `expires_at`. The reverse order would allow a
pump to run with no record that it was ever asked to.

### Absorption and re-check

```text
WaitForAbsorption ── absorption_wait elapsed ──► Recheck
                                                   │
                        moisture recovered ────────┴──► Normal (cycle complete)
                        still dry, doses < max ────────► DryConfirmed (next dose)
                        still dry, doses == max ───────► Lock(MaxDosesReached)
```

The full state machine is specified in
[PRD 060](../prd/060-irrigation-control-and-safety.md).

---

## 3. Cloud synchronisation pipeline

```text
any state change worth remembering
        │
        ▼
INSERT INTO pending_cloud_events(event_id, kind, payload_json,
                                 status='pending', attempts=0)
        │  (in the SAME transaction as the change itself — outbox pattern)
        ▼
┌──────────────────────────────────────────────────────┐
│ outbox_drain task (independent of control loop)      │
│                                                      │
│  SELECT … WHERE status='pending'                     │
│           AND next_attempt_at <= now                 │
│    ORDER BY created_at LIMIT batch_size (100)        │
│        │                                             │
│        ▼                                             │
│  POST /api/v1/edges/{edge_id}/events   (batch)       │
│        │                                             │
│   2xx ─┼─► mark 'synced', record synced_at           │
│        │                                             │
│   4xx ─┼─► permanent: mark 'quarantined' + alert     │
│        │   (a malformed event must not block others) │
│        │                                             │
│   5xx / timeout / DNS ─► attempts += 1               │
│        │                next_attempt_at = now +      │
│        │                full_jitter(backoff(attempts))│
│        │                status stays 'pending'       │
└────────┴─────────────────────────────────────────────┘
```

Properties this gives us:

- **The control loop never waits on the network.** Cloud latency cannot delay a
  watering decision (SAFETY-008, SAFETY-009).
- **At-least-once delivery, exactly-once effect.** The cloud deduplicates on
  `(edge_id, event_id)`; replay after an outage is safe by construction.
- **Bounded blast radius for bad data.** A 4xx quarantines one event rather than
  wedging the queue behind it forever.

Backoff parameters are specified in
[ADR-014](../adr/014-failure-and-retry-policy.md).

---

## 4. Configuration flow

```text
operator (UI/API) ──► edge SQLite (plant profile, device config)
                            │
                            ▼
                  config_version += 1
                            │
                            ▼
       publish retained on rhizo/v1/devices/{id}/config (QoS 1)
                            │
                            ▼
                     device applies, stores in NVS
                            │
                            ▼
       device echoes applied config_version in its status message
                            │
                            ▼
       edge compares desired vs applied → drift visible in API/UI
```

Retained delivery means a device that boots hours later still receives the
current configuration without the edge tracking who is awake.

**Hard safety limits are not in this flow.** They are compiled into firmware and
cannot be changed by config, cloud, or UI (SAFETY-007). See
[configuration-model.md](configuration-model.md).

---

## 5. What crosses which boundary

| Boundary | Carries | Never carries |
|---|---|---|
| device → broker | telemetry, status, command results | decisions |
| broker → edge | the above | anything authoritative about time |
| edge → device | config, bounded commands | unbounded doses |
| edge → cloud | historical events | requests for permission |
| cloud → edge | acknowledgements only (V1) | commands, config |
| UI → edge | user intent over HTTP | MQTT of any kind |
