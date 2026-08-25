# Failure Scenarios — Automated Test Catalogue

Each scenario below is an executable test. Every one names its setup, its
actions, its assertions, the invariants it proves, and the milestone that
delivers it.

Scenarios assert on **observable state** — API responses, database rows, MQTT
messages captured by a spy subscriber. Never on log strings.

---

## Format

```text
SCEN-nnn  name
  Level        unit | integration | e2e
  Milestone    when it must be green
  Proves       SAFETY-nnn, …
  Setup / Actions / Assertions
```

---

## A. Baseline operation

### SCEN-001 normal telemetry ingestion
- **Level** integration · **Milestone** M3 · **Proves** —
- **Setup** broker up, edge up, simulator publishing every 300 s (accelerated)
- **Actions** run for 10 simulated telemetry intervals
- **Assertions** 10 `measurements` rows; `received_at` monotonically increasing;
  `mqtt_messages_received_total` == 10; device `last_seen_at` current

### SCEN-002 full watering cycle
- **Level** e2e · **Milestone** M8 · **Proves** SAFETY-006, SAFETY-012
- **Setup** plant with `auto_watering_enabled = true`, initial VWC 42 %,
  `target_min` 28 %, dose 40 ml, absorption wait 15 min, max 3 doses
- **Actions** let soil dry past the threshold; run until the state machine settles
- **Assertions** the observed state sequence is exactly
  `Normal → Drying → DryConfirmed → DoseIssued → WaitForAbsorption → Recheck →
  … → Normal`; ≤ 3 doses; total delivered ≤ `max_daily_ml`; every
  `watering_event` has a matching terminal `command`; the recommendation that
  preceded the first dose carries a non-empty reason list

### SCEN-003 recommendation without automation
- **Level** integration · **Milestone** M5 · **Proves** —
- **Setup** `auto_watering_enabled = false`
- **Actions** dry the soil past the threshold and hold
- **Assertions** plant state reaches `WaterRecommended`; **zero commands
  published**; the recommendation includes `moisture_below_target` and `dry_for`

---

## B. MQTT failures

### SCEN-010 duplicate QoS 1 telemetry
- **Level** integration · **Milestone** M3 · **Proves** SAFETY-001
- **Actions** simulator publishes each message twice with the same `message_id`
- **Assertions** one `measurements` row per logical message;
  `mqtt_duplicate_messages_total` equals the duplicate count; no duplicate
  `device_events`

### SCEN-011 duplicate water command
- **Level** e2e · **Milestone** M6 · **Proves** SAFETY-001
- **Actions** publish the identical `command.water` (same `command_id`) three times
- **Assertions** the pump actuates **once**; three `command.result` messages, two
  of which are the re-published stored result; exactly one `watering_event`;
  `delivered_ml` counted once toward the daily total

### SCEN-012 broker restart
- **Level** e2e · **Milestone** M8 · **Proves** SAFETY-008 (partial)
- **Actions** stop Mosquitto for 30 s mid-run, restart
- **Assertions** edge reconnects and **re-establishes all subscriptions**;
  telemetry resumes; retained status is redelivered; no data corruption; if the
  outage exceeded `max_sample_age`, the plant locked out and then recovered

### SCEN-013 out-of-order and delayed telemetry
- **Level** integration · **Milestone** M3 · **Proves** —
- **Actions** `--fault reorder:0.3`
- **Assertions** all messages stored; "latest sample" always reflects the
  greatest `received_at`; `sequence_regression` events recorded; no rejection

### SCEN-014 malformed payload
- **Level** integration · **Milestone** M3 · **Proves** SAFETY-012
- **Actions** publish invalid JSON, then `v: 99`, then a `device_id` mismatch
- **Assertions** each is quarantined with the documented reason; no
  `measurements` row; the pipeline continues processing valid messages after each

### SCEN-015 no retained messages on command topics
- **Level** integration · **Milestone** M2 · **Proves** ADR-002
- **Actions** run a full watering cycle, then subscribe fresh
- **Assertions** a new subscriber receives retained `status` and `config` and
  **nothing** on any `commands/*` topic

### SCEN-016 device ACL isolation
- **Level** integration · **Milestone** M2 · **Proves** ADR-012
- **Actions** authenticate as `plant-node-01`, attempt to publish to
  `rhizo/v1/devices/plant-node-02/telemetry/soil`
- **Assertions** the publish is denied by the broker; no row is created for
  `plant-node-02`

---

## C. Device failures

### SCEN-020 device offline via Last Will
- **Level** integration · **Milestone** M4 · **Proves** —
- **Actions** kill the simulator without a clean disconnect
- **Assertions** the retained LWT marks the device offline within the keepalive
  window; `devices_offline` == 1; a `device_event(kind='offline')` is recorded

### SCEN-021 device reconnect
- **Level** integration · **Milestone** M4 · **Proves** —
- **Actions** restart the simulator with a new `boot_id`
- **Assertions** device returns online; a `boot` event is recorded; the sequence
  restart is **not** flagged as a regression; history is preserved

### SCEN-022 stale sensor
- **Level** e2e · **Milestone** M8 · **Proves** SAFETY-005
- **Actions** simulator stops publishing soil telemetry while remaining connected
- **Assertions** after `max_sample_age`, the plant enters `Lock(StaleData)`; **no
  command is issued** even though moisture was last seen below target; the API
  exposes the lockout with its reason; resuming telemetry clears it automatically

### SCEN-023 invalid sensor values
- **Level** e2e · **Milestone** M8 · **Proves** SAFETY-005, SAFETY-012
- **Actions** `--fault invalid-soil:1.0` (moisture 150 %, then `null`, then NaN)
- **Assertions** rows stored with `moisture_vwc = NULL`; `sensor_invalid` events
  raised; plant enters `Lock(SensorFault)`; no command issued

### SCEN-024 stuck sensor
- **Level** integration · **Milestone** M5 · **Proves** SAFETY-005
- **Actions** `--fault stuck-sensor` for 20 consecutive readings
- **Assertions** the stuck condition is detected and reported as a sensor fault
  rather than being trusted as a genuine constant reading

### SCEN-025 clock unsynced
- **Level** e2e · **Milestone** M8 · **Proves** SAFETY-002, SAFETY-012
- **Actions** `--fault clock-unsync`, then trigger a watering condition
- **Assertions** telemetry continues normally; the water command is refused with
  `reason: "clock_unsynced"`; no actuation; the lockout names the specific
  remedy rather than a generic error

### SCEN-026 device restart mid-dose
- **Level** e2e · **Milestone** M9 · **Proves** SAFETY-011
- **Actions** `--fault restart-mid-dose`
- **Assertions** on reboot the pump is off; a `command.result` with
  `status: "interrupted"` and `delivered_ml: null` is published; the edge marks
  the command terminal, **credits the full `requested_ml`** to the daily budget
  conservatively, and moves to `Recheck` rather than assuming success or failure

---

## D. Command safety

### SCEN-030 expired command
- **Level** integration · **Milestone** M6 · **Proves** SAFETY-002
- **Actions** publish a `command.water` with `expires_at` already in the past
- **Assertions** refused with `reason: "expired"`; pump never runs; no
  `watering_event`

### SCEN-031 queued command after a long disconnect
- **Level** e2e · **Milestone** M8 · **Proves** SAFETY-002
- **Actions** disconnect the device; issue a command; reconnect after the TTL
- **Assertions** because `clean_session = true`, the broker queued nothing; even
  if delivered, the command is refused as expired; no actuation

### SCEN-032 oversized command
- **Level** integration · **Milestone** M6 · **Proves** SAFETY-007
- **Actions** publish `requested_ml: 10000` directly to the command topic,
  bypassing the edge entirely
- **Assertions** the simulator clamps to `FIRMWARE_MAX_ML_PER_RUN` and reports
  `clamped: true`, **or** rejects — never delivers more than the hard limit.
  This is the test that proves a compromised edge cannot flood the room.

### SCEN-033 device daily cap
- **Level** integration · **Milestone** M6 · **Proves** SAFETY-007
- **Actions** issue commands totalling more than `FIRMWARE_MAX_DAILY_ML`
- **Assertions** the device rejects with `over_daily_max` once the cap is
  reached, independently of the edge's own accounting

### SCEN-034 rolling 24-hour cap
- **Level** property + e2e · **Milestone** M6 · **Proves** SAFETY-006
- **Actions** drive many cycles across 72 simulated hours with random restarts
  and clock jumps
- **Assertions** at every instant, the sum of `delivered_ml` over the preceding
  24 hours never exceeds `max_daily_ml`

### SCEN-035 cooldown
- **Level** integration · **Milestone** M6 · **Proves** —
- **Actions** complete a cycle, then immediately re-dry the soil
- **Assertions** no dose is issued until `cooldown_hours` has elapsed; the API
  reports `CooldownActive` with the remaining time

---

## E. Hardware safety lockouts

### SCEN-040 leak detected
- **Level** e2e · **Milestone** M8 · **Proves** SAFETY-003
- **Actions** `--fault leak` during normal operation, then attempt a manual dose
  via `POST /plants/{id}/water`
- **Assertions** immediate `Lock(Leak)`; automatic watering stops; **the manual
  API call returns 409**; clearing while the leak is still asserted also returns
  409; only after the leak clears does an explicit reset succeed

### SCEN-041 leak state unknown
- **Level** integration · **Milestone** M6 · **Proves** SAFETY-012
- **Actions** publish tank telemetry with `leak_detected: null`
- **Assertions** treated as `Unknown` → lockout. Specifically **not** treated as
  `false`.

### SCEN-042 tank empty
- **Level** e2e · **Milestone** M8 · **Proves** SAFETY-004
- **Actions** `--fault tank-empty`
- **Assertions** `Lock(TankLow)`; no command issued; the device would also refuse
  independently; refilling clears the lockout automatically

### SCEN-043 tank level unknown
- **Level** integration · **Milestone** M6 · **Proves** SAFETY-004, SAFETY-012
- **Actions** run with `--sensors soil` (no tank sensor at all)
- **Assertions** automatic watering is locked out. Absence of a tank reading is
  not permission to pump.

### SCEN-044 pump delivers nothing
- **Level** e2e · **Milestone** M8 · **Proves** —
- **Actions** `--fault pump-no-delivery`; allow two doses
- **Assertions** after two doses with no moisture and no weight response,
  `Lock(NoDeliveryDetected)`; escalation stops; the lockout requires an explicit
  operator clear

### SCEN-045 pump stuck on
- **Level** integration · **Milestone** M9 · **Proves** SAFETY-007
- **Actions** `--fault pump-stuck-on`
- **Assertions** the independent run-duration timer de-energises at
  `FIRMWARE_MAX_RUN_SECONDS`; `pump_fault` is reported; further commands are
  refused with `pump_unavailable`

---

## F. Edge failures

### SCEN-050 edge restart preserves history
- **Level** integration · **Milestone** M3 · **Proves** —
- **Actions** ingest 50 messages, restart the edge, ingest 50 more
- **Assertions** 100 rows; device state restored from SQLite; no duplicate rows

### SCEN-051 edge restart mid-command
- **Level** e2e · **Milestone** M8 · **Proves** SAFETY-010
- **Actions** kill the edge immediately after the command publish, before the
  result arrives; restart
- **Assertions** the command is **not** re-published; the late result is matched
  to the existing `command_id`; exactly one `watering_event`; the daily total
  counts it once

### SCEN-052 edge restart mid-absorption
- **Level** integration · **Milestone** M6 · **Proves** SAFETY-010
- **Actions** restart while a plant is in `WaitForAbsorption`
- **Assertions** the state and its `wait_until` are restored from SQLite — not
  reset to a default; the cycle completes normally with the correct dose count

### SCEN-053 SQLite busy
- **Level** unit · **Milestone** M3 · **Proves** —
- **Actions** hold a write lock while the pipeline attempts a transaction
- **Assertions** retry with backoff, then a clean failure; the message is not
  marked processed, so redelivery reprocesses it correctly

### SCEN-054 control loop panic
- **Level** integration · **Milestone** M6 · **Proves** —
- **Actions** inject a panic into the control loop task
- **Assertions** the panic is logged with context; `task_panics_total`
  increments; **the process exits non-zero** rather than continuing with a dead
  control loop

---

## G. Cloud failures

### SCEN-060 cloud unavailable
- **Level** e2e · **Milestone** M8 · **Proves** SAFETY-008
- **Actions** stop `cloud-api` for the entire scenario; run a full watering cycle
- **Assertions** ingestion, storage, recommendations, automatic watering, the
  REST API, and metrics all function; `pending_cloud_events` grows;
  `/health/ready` stays **200** (cloud is not a readiness input)

### SCEN-061 cloud outage does not bypass safety
- **Level** e2e · **Milestone** M8 · **Proves** SAFETY-009
- **Actions** run the identical scenario twice — once cloud-up, once cloud-down —
  with a fixed seed
- **Assertions** the issued command sequences are **identical** (modulo ids and
  timestamps); every lockout occurs in both runs

### SCEN-062 cloud recovery and replay
- **Level** e2e · **Milestone** M8 · **Proves** SAFETY-008
- **Actions** cloud down for 500 events, then restarted
- **Assertions** the outbox drains; every event reaches PostgreSQL exactly once
  by `(edge_id, event_id)`; re-POSTing the same batch returns `duplicate` and
  creates no rows; `pending_cloud_events` returns to 0

### SCEN-063 cloud rejects one event
- **Level** integration · **Milestone** M7 · **Proves** —
- **Actions** inject an event the cloud rejects with 4xx into the middle of a batch
- **Assertions** that event is quarantined; the other 499 sync; the queue is not
  blocked; `cloud_events_quarantined_total` increments

### SCEN-064 cloud 5xx then recovery
- **Level** integration · **Milestone** M7 · **Proves** —
- **Actions** cloud returns 500 for 5 attempts, then succeeds
- **Assertions** backoff delays increase and stay within the jitter bounds; the
  batch eventually syncs; no events lost or duplicated

### SCEN-065 outbox cap and value-tiered pruning
- **Level** integration · **Milestone** M7 · **Proves** —
- **Actions** cloud down; generate events past `outbox_max_rows`
- **Assertions** low-tier (measurement) events are pruned oldest-first; **every
  high-tier event — watering, command, lockout, device fault — survives**;
  `cloud_events_dropped_total` increments; an alert-level log is emitted

---

## H. Time anomalies

### SCEN-070 device clock skew
- **Level** integration · **Milestone** M6 · **Proves** SAFETY-005
- **Actions** `--fault clock-skew:-21600` (device six hours behind)
- **Assertions** staleness is computed from `received_at`, so samples are
  correctly seen as fresh; a `clock_skew` event is raised; **the skewed
  `device_time_ms` never makes stale data look current**

### SCEN-071 edge clock forward step
- **Level** integration · **Milestone** M6 · **Proves** SAFETY-006
- **Actions** step the edge `TestClock` forward by 2 hours mid-scenario
- **Assertions** the step is detected; all plants enter `Uncertain` lockout for
  one cooldown; the rolling window cannot be exploited to grant an extra dose

### SCEN-072 edge clock backward step
- **Level** integration · **Milestone** M6 · **Proves** SAFETY-006
- **Actions** step the clock back by 1 hour
- **Assertions** the rolling window includes more history and becomes more
  conservative; no additional dose is permitted; the step is logged

---

## I. Multi-device and scale (M13)

### SCEN-080 cross-plant isolation
- **Level** e2e · **Milestone** M13 · **Proves** — (guards every invariant at scale)
- **Setup** 5 devices, 10 plants; a fixed seed; a recorded control run
- **Actions** re-run the identical scenario, but force every failure mode in turn
  on plant A: leak, tank empty, stale sensor, invalid values, device offline,
  interrupted dose, pump fault
- **Assertions** plant B's complete state history — irrigation states, lockouts,
  commands, watering events — is **byte-identical** to the control run. Any
  difference is cross-plant interference and is a defect, not a tolerance.

### SCEN-081 shared reservoir depletion
- **Level** e2e · **Milestone** M13 · **Proves** SAFETY-004
- **Setup** two devices assigned to one reservoir, plants on both
- **Actions** drain the reservoir below `min_percent`; then make the two devices
  report **disagreeing** levels
- **Assertions** every plant drawing on that reservoir locks out; the **lowest**
  reported level governs; an `Unknown` reading from either device makes the
  reservoir unknown and locks out. Refilling clears all of them.

### SCEN-082 notification does not block control
- **Level** integration · **Milestone** M13 · **Proves** SAFETY-008 (principle)
- **Actions** configure a notification channel that hangs indefinitely, then
  trigger a leak on one plant
- **Assertions** `control_tick_duration_seconds` is unaffected; the lockout is
  applied on the same tick as with no channel configured; the failed delivery is
  recorded in `notification_log`.

### SCEN-083 notification storm coalescing
- **Level** integration · **Milestone** M13 · **Proves** —
- **Actions** lock out 10 plants simultaneously
- **Assertions** notifications are rate-limited and coalesced; one leak produces
  one notification rather than one per tick; nothing is silently dropped without
  a `notification_log` row.

---

## Coverage matrix

| Invariant | Scenarios |
|---|---|
| SAFETY-001 | SCEN-010, SCEN-011 |
| SAFETY-002 | SCEN-025, SCEN-030, SCEN-031 |
| SAFETY-003 | SCEN-040, SCEN-041 |
| SAFETY-004 | SCEN-042, SCEN-043, SCEN-081 |
| SAFETY-005 | SCEN-022, SCEN-023, SCEN-024, SCEN-070 |
| SAFETY-006 | SCEN-034, SCEN-071, SCEN-072 |
| SAFETY-007 | SCEN-032, SCEN-033, SCEN-045 |
| SAFETY-008 | SCEN-012, SCEN-060, SCEN-062, SCEN-082 |
| SAFETY-009 | SCEN-061 |
| SAFETY-010 | SCEN-051, SCEN-052 |
| SAFETY-011 | SCEN-026 |
| SAFETY-012 | SCEN-014, SCEN-023, SCEN-025, SCEN-041, SCEN-043 |

Every invariant has at least two scenarios except SAFETY-009 and SAFETY-011,
which have one each by nature — SAFETY-009 is a differential test that
subsumes every other scenario, and SAFETY-011 is a single device behaviour
verified again physically in M11.
