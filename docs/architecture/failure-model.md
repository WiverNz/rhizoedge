# Failure Model

Every failure below is specified with six fields: **detection**, **expected
state**, **recovery**, **data loss**, **safety behaviour**, **observability**.
Anything not listed here is covered by SAFETY-012 — unknown means do not water.

Retry and backoff parameters: [ADR-014](../adr/014-failure-and-retry-policy.md).

---

## 1. MQTT failures

### 1.1 Broker unavailable at edge startup

- **Detection:** connection refused from `rumqttc` event loop.
- **Expected state:** edge starts anyway. API and DB come up; MQTT task retries.
  Health: `/health/live` OK, `/health/ready` **not ready**.
- **Recovery:** reconnect with backoff (1 s → 60 s cap, full jitter).
- **Data loss:** none; devices buffer nothing but will republish on their own
  schedule.
- **Safety:** all plants show `Uncertain`/`StaleData` lockout once samples age
  out. No watering. ✅ SAFETY-005, SAFETY-012.
- **Observability:** `mqtt_connection_state` gauge = 0, `mqtt_reconnects_total`,
  WARN log per attempt with attempt number and delay.

### 1.2 Broker restart while running

- **Detection:** event loop yields `Disconnected`.
- **Expected state:** subscriptions are **re-established on every reconnect**,
  never assumed to survive. Retained status/config are redelivered by the broker.
- **Recovery:** automatic; in-flight commands are unaffected because their
  authority is the SQLite row, not the connection.
- **Data loss:** QoS 1 messages published by devices while the broker was down
  are lost — the broker is the buffer and it was gone. Devices do not queue
  telemetry (bounded RAM); this is accepted, telemetry is samples not ledger.
  Command *results* are the exception: the device retries publishing a result
  until acknowledged, because that is ledger data.
- **Safety:** if the outage exceeds `max_sample_age`, plants lock out.
- **Observability:** `mqtt_reconnects_total`, gap visible in measurement history.

### 1.3 QoS 1 duplicate delivery

- **Detection:** `INSERT INTO processed_messages` affects 0 rows.
- **Expected state:** message discarded before any effect. Transaction rolled back.
- **Recovery:** none needed.
- **Data loss:** none.
- **Safety:** ✅ SAFETY-001.
- **Observability:** `mqtt_duplicate_messages_total{kind}`. A sustained nonzero
  rate is a signal of broker or device misbehaviour, not normal operation.

### 1.4 Device disconnect / Last Will

- **Detection:** broker publishes the retained LWT payload
  (`status: offline, reason: connection_lost`).
- **Expected state:** device marked offline; `last_seen_at` frozen.
- **Recovery:** device reconnects and publishes retained `online` status.
- **Data loss:** telemetry during the gap.
- **Safety:** samples age out → lockout. No command is issued to an offline
  device (it would expire unseen anyway).
- **Observability:** `devices_online` / `devices_offline` gauges,
  `device_event(kind='offline')`.

### 1.5 Malformed payload

- **Detection:** `serde_json` error, or envelope/topic `device_id` mismatch.
- **Expected state:** message rejected whole. Written to a bounded
  `quarantined_messages` table (topic, first 1 KiB of payload, error, timestamp)
  capped at 1000 rows, oldest evicted.
- **Recovery:** operator inspects quarantine via API.
- **Data loss:** the malformed message only.
- **Safety:** never partially applied. ✅ SAFETY-012.
- **Observability:** `mqtt_decode_errors_total{reason}`, ERROR log with the topic.
- **Note:** a device flooding malformed payloads must not fill the disk — hence
  the cap and the per-device rate limit on quarantine writes (10/min).

### 1.6 Delayed and out-of-order messages

- **Detection:** `sequence` lower than the last seen for the same `boot_id`, or
  `device_time_ms` earlier than the previous message.
- **Expected state:** stored normally. Queries are time-ordered, not
  arrival-ordered.
- **Recovery:** none needed.
- **Safety:** "latest sample" is `MAX(received_at)`, so a late-arriving old
  sample cannot make a plant look fresher or wetter than it is.
- **Observability:** `device_event(kind='sequence_regression')`.

### 1.7 Broker rejects publish / send buffer full

- **Detection:** `rumqttc` returns `ClientError`.
- **Expected state:** for commands, the SQLite row stays `issued`; the publish is
  retried up to 3 times, then the command is marked `failed` and the irrigation
  state returns to `Recheck` — **never** to a state that assumes water was
  delivered.
- **Safety:** a failed publish must never be recorded as a watering event.
- **Observability:** `watering_commands_total{outcome="publish_failed"}`.

---

## 2. Device failures

### 2.1 ESP32 restart (power loss, watchdog, panic)

- **Detection:** new `boot_id` in the envelope; sequence restarts.
- **Expected state:** pump off before anything else initialises. Any unfinished
  dose recorded in NVS is reported as `command_result(status='interrupted',
  delivered_ml=null)`.
- **Recovery:** edge marks the command terminal without crediting volume, moves
  to `Recheck`, and re-evaluates from fresh soil data.
- **Data loss:** the true delivered volume of the interrupted dose is unknown —
  and is treated as unknown, not as zero.
- **Safety:** ✅ SAFETY-011. Because volume is unknown, the daily cap
  conservatively counts the full requested dose for that command.
- **Observability:** `device_event(kind='boot')` with `boot_id`,
  `device_restarts_total`.

### 2.2 Wi-Fi unavailable

- **Detection:** device-side; edge sees LWT or silence.
- **Expected state:** device keeps sampling locally, retries Wi-Fi with backoff,
  keeps the pump off. It does **not** water on its own — the device has no
  irrigation logic (component-model §10).
- **Recovery:** reconnect, republish retained status, resume telemetry.
- **Safety:** an isolated device is a monitoring device only.

### 2.3 Sensor disconnected or returning garbage

- **Detection:** read error, or value outside the physical range; a value that is
  bit-identical for `stuck_sample_count` (default 20) consecutive reads.
- **Expected state:** field published as `null` with a `sensor_error` counter in
  the status message. Edge stores `NULL` and raises `device_event(sensor_invalid)`.
- **Recovery:** manual — a hardware fault needs hands.
- **Safety:** automatic watering locked out with `SensorFault`. ✅ SAFETY-005.
- **Observability:** `sensor_errors_total{sensor,reason}`, plant lockout visible
  in API and UI.

### 2.4 Device clock never syncs

- **Detection:** `clock_synced: false` in status.
- **Expected state:** telemetry continues; **every water command is refused**
  with `clock_unsynced`.
- **Recovery:** fix SNTP reachability.
- **Safety:** ✅ SAFETY-002, SAFETY-012. See [time-model.md](time-model.md) §4.
- **Observability:** `device_event(kind='clock_unsynced')`, lockout reason.

### 2.5 Command interrupted mid-pump

Covered by 2.1. The distinguishing requirement: the NVS write of
`(command_id, started_at, requested_ml)` happens **before** the GPIO goes
active, so an interruption is always detectable on the next boot.

---

## 3. Edge Controller failures

### 3.1 Process crash / restart

- **Detection:** external (supervisor, compose restart policy).
- **Expected state on boot:** migrations run; irrigation state loaded from
  SQLite; commands in `issued`/`in_flight` reconciled per SAFETY-010's recovery
  procedure; MQTT resubscribes; outbox resumes.
- **Data loss:** in-flight, uncommitted work only — by definition not durable
  and not acted upon.
- **Safety:** ✅ SAFETY-010. No command is re-published; no watering event is
  double-counted.
- **Observability:** startup INFO log listing recovered command count and plant
  states; `edge_restarts_total`.

### 3.2 SQLite locked / busy

- **Detection:** `SQLITE_BUSY` from `sqlx`.
- **Expected state:** WAL mode plus `busy_timeout = 5000 ms` makes this rare.
  Write contention is further reduced by funnelling all writes through the
  pipeline task rather than sharing a writer across tasks.
- **Recovery:** retry the transaction up to 3 times with 50/100/200 ms jitter,
  then fail the operation and log ERROR.
- **Safety:** a failed transaction means no effect was applied — the message
  will be redelivered by the broker (QoS 1) or the tick will retry. A failed
  *command-issue* transaction means no command is published. Fail-closed.
- **Observability:** `sqlite_busy_total`, `sqlite_txn_duration`.

### 3.3 Disk full

- **Detection:** `SQLITE_FULL`.
- **Expected state:** ingestion fails loudly; the control loop treats "cannot
  persist" as "cannot act" and issues no commands.
- **Recovery:** retention task prunes; operator intervention.
- **Safety:** fail-closed. A system that cannot record what it did must not do
  anything. ✅ SAFETY-012.
- **Observability:** `storage_bytes` gauge, `disk_full_total`, alert-worthy log.

### 3.4 Restart during a state transition

- **Detection:** implicit.
- **Expected state:** transitions are single SQLite transactions, so a restart
  lands either fully before or fully after. There is no half-transition.
- **Safety:** the reason state transitions were made transactional rather than
  in-memory-then-flush.

### 3.5 Duplicate command result

- **Detection:** `processed_messages` dedup, plus a terminal-status check on the
  command row.
- **Expected state:** ignored. A result arriving for a command already
  `completed` produces no second `watering_event`.
- **Safety:** ✅ SAFETY-001.

### 3.6 Control loop task panics

- **Detection:** the supervising task observes the `JoinHandle` error.
- **Expected state:** the panic is logged with full context and **the process
  exits non-zero**. It does not silently continue with a dead control loop.
- **Rationale:** a process that is up but not evaluating safety is worse than a
  process that is down, because supervision and alerting see "healthy".
- **Recovery:** supervisor restarts; SAFETY-010 recovery applies.
- **Observability:** `task_panics_total{task}`, ERROR log, non-zero exit.

---

## 4. Cloud failures

### 4.1 Cloud unavailable (connection refused, DNS failure, timeout)

- **Detection:** transport error in `cloud-client`.
- **Expected state:** events accumulate in `pending_cloud_events`. Everything
  local continues untouched.
- **Recovery:** exponential backoff with full jitter, 1 s → 300 s cap, retried
  indefinitely.
- **Data loss:** none until the outbox cap is reached.
- **Safety:** ✅ SAFETY-008, SAFETY-009.
- **Observability:** `pending_cloud_events` gauge, `cloud_sync_failures_total{reason}`,
  `cloud_last_success_timestamp`.

### 4.2 Cloud returns 5xx

Same as 4.1 — treated as transient.

### 4.3 Cloud returns 4xx

- **Detection:** status in 400–499 except 408/429.
- **Expected state:** the event is **quarantined**, not retried forever. The
  batch continues with the remaining events.
- **Rationale:** a permanently malformed event at the head of the queue would
  otherwise block every subsequent event indefinitely.
- **Recovery:** operator inspects `pending_cloud_events WHERE status='quarantined'`.
- **Safety:** no local impact.
- **Observability:** `cloud_events_quarantined_total`, ERROR log with `event_id`.
- **429** is honoured as a rate limit: back off using `Retry-After` when present.

### 4.4 Duplicate replay after outage

- **Detection:** cloud side, unique `(edge_id, event_id)`.
- **Expected state:** the cloud returns success for an already-stored event so
  the edge can mark it synced and move on. Idempotent by construction.
- **Data loss:** none; no duplicates created.
- **Observability:** `cloud_events_deduplicated_total` on the cloud side.

### 4.5 Prolonged outage — outbox growth

- **Detection:** `pending_cloud_events` count exceeds `outbox_max_rows`
  (default 500 000, roughly weeks of a small deployment).
- **Expected state:** oldest **low-value** events (measurements) are pruned
  first; **high-value** events (watering events, lockouts, device faults,
  commands) are preserved. Pruning raises an alert-level log and increments
  `cloud_events_dropped_total`.
- **Rationale:** history is nice; the ledger of what the machine did to a plant
  is not optional.
- **Safety:** no local impact. ✅ SAFETY-008.

---

## 5. Hardware failures

### 5.1 Pump runs but delivers no water (air lock, kinked tube, dry line)

- **Detection:** after a dose, moisture does not rise by
  `recovery_delta_vwc` and — where a scale exists — pot weight does not rise.
  Two consecutive doses with no response.
- **Expected state:** `Lock(NoDeliveryDetected)` and an alert. The system stops
  rather than escalating doses into a plant that may in fact be receiving water
  the sensor cannot see.
- **Recovery:** operator inspects and clears the lockout explicitly.
- **Safety:** prevents the "pump keeps running because nothing changed" failure,
  which is the single most damaging plausible bug in this class of system.
- **Observability:** `watering_failures_total{reason="no_delivery"}`.

### 5.2 Pump stuck on (relay welded, MOSFET shorted)

- **Detection:** device-side run timer exceeds `FIRMWARE_MAX_RUN_SECONDS`;
  hardware watchdog if the task is hung.
- **Expected state:** device de-energises, marks the pump `faulted`, refuses
  further commands until reboot, and reports `pump_fault`.
- **Recovery:** hardware inspection.
- **Safety:** ✅ SAFETY-007. This is why the run-duration limit is enforced by a
  timer independent of the MQTT task (M11).
- **Observability:** `device_event(kind='pump_fault')`, plant lockout.

### 5.3 Reservoir empty

- **Detection:** tank telemetry ≤ `tank_min_percent`; or unknown tank level.
- **Expected state:** `Lock(TankLow)`. Both edge and device refuse.
- **Safety:** ✅ SAFETY-004.

### 5.4 Leak detected

- **Detection:** leak sensor asserts.
- **Expected state:** immediate `Lock(Leak)` for all plants on that device;
  automatic **and** manual watering refused; explicit operator reset required
  after the signal clears.
- **Recovery:** operator dries, inspects, and clears.
- **Safety:** ✅ SAFETY-003.
- **Observability:** `device_event(kind='leak')` — the highest-severity event the
  system produces.

### 5.5 Calibration drift

- **Detection:** systematic divergence between requested and weight-derived
  delivered volume (needs a scale, M9+).
- **Expected state:** `calibration_drift` event when divergence exceeds 25% over
  five doses. Not a lockout in V1 — a warning.
- **Recovery:** operator re-runs pump calibration.
- **Observability:** `pump_calibration_error_ratio` gauge.

---

## 6. Cross-cutting rules

1. **Fail closed.** Every failure whose correct response is ambiguous resolves
   to "do not water" plus a visible lockout.
2. **A failure is not handled until it is observable.** Every entry above names
   a metric or event; an unlogged failure is an unfixable failure.
3. **Lockouts are sticky.** Automatic recovery from a lockout is permitted only
   for conditions with an unambiguous "clear" signal (stale data becoming fresh,
   cooldown elapsing). Leak, pump fault, and no-delivery require explicit
   operator action.
4. **Local degradation is never silent.** If the edge cannot do its job, health
   endpoints and the UI say so rather than showing a stale-but-plausible screen.
