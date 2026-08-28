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
  `rhizo/v1/devices/plant-node-02/telemetry`
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
- **Actions** `--fault clock-unsync` (no `edge.time` ever applied), then trigger
  a watering condition
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

### SCEN-073 time sync on connect enables commands
- **Level** integration · **Milestone** M6 · **Proves** SAFETY-002
- **Setup** device boots with an unset wall clock
- **Actions** connect; let the edge observe the retained status; then issue a
  water command
- **Assertions** the edge publishes `edge.time` on the `time` topic with
  `retain = false`; the device applies it and reports `clock_synced: true`; the
  subsequent command is **accepted**; no command was accepted before the sync

### SCEN-074 no time sync refuses commands
- **Level** integration · **Milestone** M6 · **Proves** SAFETY-002, SAFETY-012
- **Actions** suppress the edge's `edge.time` publication entirely; connect the
  device; issue a water command
- **Assertions** refused with `reason: "clock_unsynced"`; **telemetry and
  monitoring continue normally**; the device republishes status at a bounded
  rate while unsynchronised so the edge has a retry trigger

### SCEN-075 aged-out time sync refuses commands
- **Level** integration · **Milestone** M6 · **Proves** SAFETY-002, SAFETY-012
- **Actions** sync the device, then stop `edge.time` refresh and advance the
  monotonic clock past `TIME_SYNC_MAX_AGE_SECONDS`
- **Assertions** `clock_synced` flips to false at the boundary and not before;
  a water command is refused with `clock_unsynced`; the edge surfaces this as a
  named lockout reason, not a generic error; a single fresh `edge.time` restores
  acceptance

### SCEN-076 stale edge.time is ignored
- **Level** unit + integration · **Milestone** M6 · **Proves** SAFETY-002
- **Actions** apply `edge.time` at T; then deliver `edge.time` values at T−3600 s
  and out of order
- **Assertions** the wall clock **never moves backwards**; only a value strictly
  greater than `last_applied_edge_time_ms` is applied; a command whose
  `expires_at` has passed cannot be made valid again by a replayed time message

### SCEN-079 replayed edge.time cannot hold a device synchronised
- **Level** unit + integration · **Milestone** M6 · **Proves** SAFETY-002
- **Actions** apply one valid `edge.time` at T; redeliver **exactly that message**
  repeatedly — more often than `TIME_SYNC_INTERVAL_SECONDS` — while advancing the
  monotonic clock past `TIME_SYNC_MAX_AGE_SECONDS`; then issue a water command
- **Assertions** `clock_synced == false` at the end, despite continuous traffic on
  the `time` topic; **no duplicate ever refreshed `synced_at_monotonic`**; the
  command is refused with `clock_unsynced`; a single strictly newer `edge.time`
  then restores acceptance immediately

  This is the QoS 1 case: the broker may redeliver the same message any number of
  times, so a non-decreasing rule would make the validity window measure message
  arrival instead of synchronisation freshness.

### SCEN-077 reconnect refuses until a fresh sync
- **Level** e2e · **Milestone** M8 · **Proves** SAFETY-002, SAFETY-015
- **Actions** isolate a device until its sync ages out; restore the link; issue a
  command in the same instant the connection comes back
- **Assertions** the command is refused with `clock_unsynced` if it arrives
  before the sync is applied; the very next command, after `edge.time`, is
  accepted; offline autonomy was unaffected throughout because it uses monotonic
  time

### SCEN-078 periodic refresh keeps a long-connected device synced
- **Level** integration · **Milestone** M6 · **Proves** SAFETY-002
- **Actions** hold a device connected for 24 h virtual with no reconnects
- **Assertions** the edge publishes `edge.time` every
  `TIME_SYNC_INTERVAL_SECONDS`; `clock_synced` never becomes false; no `time`
  message is ever published with `retain = true`; device/edge skew stays within
  `MAX_CLOCK_SKEW` for the whole run

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

## J. Device isolation and offline autonomy (M6 sim, M8 e2e, M9 firmware)

Mode C in [connectivity-modes.md](../architecture/connectivity-modes.md): the
device cannot reach Wi-Fi, the broker, or the edge. These scenarios are the
reason [ADR-015](../adr/015-device-offline-autonomy.md) exists, and every one of
them must be runnable against the simulator with no hardware.

### SCEN-090 Wi-Fi loss while monitoring, no automation
- **Level** e2e · **Milestone** M8 · **Proves** SAFETY-013
- **Setup** device with sensors, **no offline policy provisioned**
- **Actions** drop Wi-Fi; let soil dry well past `trigger_below`; hold 6 h virtual
- **Assertions** the device keeps sampling and buffering; **it never actuates**;
  the edge marks it offline and locks its plants on stale data; on reconnect the
  buffered telemetry replays and history shows the dry period with no watering

### SCEN-091 Wi-Fi loss before soil becomes dry, automation enabled
- **Level** e2e · **Milestone** M8 · **Proves** SAFETY-013, SAFETY-014
- **Setup** valid policy `enabled: true`, soil comfortably above `trigger_below`
- **Actions** drop Wi-Fi; let soil dry through the threshold; hold for
  `confirm_duration` plus a full cycle
- **Assertions** confirmation elapses on the **monotonic** clock; one bounded
  dose of exactly `dose_ml` is delivered; absorption wait is honoured; the cycle
  ends when moisture passes `resume_above`; total delivered ≤
  `max_volume_per_window_ml`

### SCEN-092 Wi-Fi loss during an edge-commanded dose
- **Level** e2e · **Milestone** M8 · **Proves** SAFETY-001, SAFETY-016
- **Actions** issue a command; drop the link after the device accepts but before
  the result is published
- **Assertions** the device completes the dose and buffers the result; the edge
  sees an `issued` command with no result and does **not** re-issue; on reconnect
  the buffered result settles the original `command_id`; exactly one
  `watering_event` exists

### SCEN-093 No offline policy — refuse
- **Level** integration · **Milestone** M6 · **Proves** SAFETY-013
- **Actions** isolate a device that has never received a policy; drive soil dry
- **Assertions** decision is `Refuse(NoValidPolicy)` on every evaluation; no
  actuation; the refusal is buffered as an audit event and visible after
  reconnection

### SCEN-094 Corrupt offline policy — refuse and keep nothing
- **Level** integration · **Milestone** M6 · **Proves** SAFETY-013, SAFETY-019
- **Actions** corrupt the persisted policy blob (bit flip, truncation, bad CRC),
  restart the device, drive soil dry
- **Assertions** the corrupt policy is rejected at load; **no actuation**; a
  `policy_invalid` audit event is buffered; the device does not fall back to any
  default threshold

### SCEN-095 Policy update interrupted by power loss
- **Level** integration · **Milestone** M9 · **Proves** SAFETY-019
- **Actions** publish policy v8 to a device running v7; cut power at each step of
  validate → stage → verify → activate → acknowledge, one run per step
- **Assertions** after every interruption **exactly one valid policy is active** —
  v7 before activation, v8 after; never a mixture of fields from both; the device
  reports the version it actually holds

### SCEN-096 Offline autonomous watering respects the rolling budget
- **Level** property + e2e · **Milestone** M6 · **Proves** SAFETY-014
- **Actions** isolate; drive repeated dry cycles across 72 h virtual, more than
  the budget allows
- **Assertions** the device stops at `max_volume_per_window_ml` and refuses with
  `BudgetExhausted`; the budget replenishes only as the rolling window actually
  advances; after reconnection the edge's row-derived budget matches the device's
  reported spend

### SCEN-097 Device restart while isolated
- **Level** e2e · **Milestone** M8 · **Proves** SAFETY-014, SAFETY-015
- **Actions** isolate; deliver one autonomous dose; power-cycle the device
  repeatedly during the cooldown
- **Assertions** the cooldown resumes from its **persisted remaining duration**
  and is never shortened; `budget_used_ml` is not reset; no dose is delivered
  before the cooldown genuinely elapses; the buffered dose event survives

### SCEN-098 Isolated device with no usable wall clock
- **Level** integration · **Milestone** M6 · **Proves** SAFETY-015, SAFETY-002
- **Setup** device has never applied an `edge.time`
- **Actions** isolate; drive a full dry cycle; then reconnect and send a command
- **Assertions** **autonomous watering proceeds** on monotonic time (durations do
  not need a calendar); the buffered events carry `device_time_ms: null` and a
  valid `monotonic_ms`; the **edge command is still refused** with
  `clock_unsynced` until an `edge.time` is applied

### SCEN-099 Required measurement unavailable while isolated
- **Level** integration · **Milestone** M6 · **Proves** SAFETY-017
- **Actions** isolate a plant whose policy requires tank level and leak state;
  disconnect the tank sensor; drive soil dry
- **Assertions** refusal with `RequiredMeasurementUnavailable`; **no actuation**;
  restoring the sensor allows the cycle to proceed; a plant whose policy does not
  require pot weight is unaffected by a broken scale

### SCEN-100 Offline watering followed by reconnection
- **Level** e2e · **Milestone** M8 · **Proves** SAFETY-016
- **Actions** isolate; deliver two autonomous doses; restore the link
- **Assertions** the device replays events in `device_seq` order and sets
  `"complete": true`; the edge creates `watering_event` rows with
  `origin = offline_autonomous`; the rolling budget absorbs them; the plant
  leaves `Uncertain` **only after** replay completes; **the edge issues no dose
  during reconciliation**

### SCEN-101 Duplicate offline event replay
- **Level** integration · **Milestone** M6 · **Proves** SAFETY-016
- **Actions** replay the same buffered batch three times, out of order, with a
  disconnect mid-batch
- **Assertions** one `watering_event` per distinct `event_id`; the budget counts
  each dose once; `event_id` values are byte-identical across replays

### SCEN-102 Edge restart during reconciliation
- **Level** e2e · **Milestone** M8 · **Proves** SAFETY-016, SAFETY-010
- **Actions** kill the edge midway through a replay; restart it
- **Assertions** the device replays again because it never saw an
  acknowledgement; no duplicate rows; the plant stays `Uncertain` across the
  restart and is released only when replay completes; no dose is issued in
  between

### SCEN-103 Stale policy version after reconnection
- **Level** integration · **Milestone** M6 · **Proves** SAFETY-019
- **Actions** while the device is isolated, edit the policy twice on the edge
  (v8, v9); reconnect; then have the broker redeliver the retained v8
- **Assertions** the device applies v9; the redelivered v8 is **ignored** because
  its version is lower; `applied_policy_versions` reports 9; the edge shows no
  drift

### SCEN-104 Event buffer overflow reports a gap
- **Level** integration · **Milestone** M9 · **Proves** SAFETY-020
- **Actions** isolate for long enough to overflow the ring, generating both
  telemetry and audit events
- **Assertions** **audit events survive**; telemetry is evicted first; a
  `history.gap` event carries the lost `device_seq` range and count; the edge
  stores it in `history_gaps` and the plant's history shows the gap explicitly

### SCEN-105 Advisory measurement missing does not block
- **Level** integration · **Milestone** M6 · **Proves** SAFETY-017
- **Actions** isolate a plant with an advisory ambient-temperature binding;
  remove that sensor; drive soil dry
- **Assertions** the autonomous cycle proceeds normally; the missing advisory
  measurement raises an event but **does not** gate actuation — the converse of
  SCEN-099, and equally important

### SCEN-106 Monitoring-only plant has no actuation path
- **Level** integration · **Milestone** M5 · **Proves** SAFETY-018
- **Setup** plant with sensor bindings and **no `ActuatorBinding`**
- **Actions** attempt `POST /plants/{id}/water`; attempt to enable connected
  automation; attempt to publish an offline policy for it
- **Assertions** the API returns **422 `no_actuator_bound`** — distinguishable
  from a 409 safety refusal; both automation attempts are rejected at validation;
  the plant still receives telemetry, history, thresholds, warnings, and critical
  alerts

### SCEN-107 Long isolation with the edge host down
- **Level** e2e · **Milestone** M8 · **Proves** SAFETY-008, SAFETY-013, SAFETY-014
- **Actions** stop the edge container entirely (not just the broker) for 48 h
  virtual, with two plants: one provisioned for offline autonomy, one not
- **Assertions** the provisioned plant is watered autonomously within its budget;
  the unprovisioned plant is not watered at all; both devices buffer history;
  when the edge returns, both reconcile without duplicates and the operator can
  see exactly what happened while nobody was watching

---

## K. Battery and deep-sleep device mode (M5 edge, M6 delivery, M8 e2e, M9 firmware)

[ADR-018](../adr/018-battery-and-deep-sleep-device-mode.md): a device in
`PowerMode::Battery` is absent most of the time by design. These scenarios exist
to keep the two kinds of absence apart — announced and bounded, versus overdue
and unexplained — and to prove that holding a command for a sleeping device did
not weaken the command pipeline. All of them run against the simulator's battery
mode with no hardware.

### SCEN-110 Battery device sleeps and wakes on schedule
- **Level** integration · **Milestone** M5 · **Proves** SAFETY-021
- **Setup** simulator in `PowerMode::Battery`, `wake_interval_seconds` 900,
  accelerated clock
- **Actions** run for six wake cycles
- **Assertions** connectivity is `sleeping` between wakes and `connected` during
  them; `expected_wake_at` is `received_at(announcement) + 900 s` on the **edge**
  clock; the device is never `isolated`; `sample_age_seconds` never crosses the
  staleness threshold; `devices_sleeping` gauge tracks reality

### SCEN-111 A sleeper that misses its wake becomes isolated
- **Level** integration · **Milestone** M5 · **Proves** SAFETY-021
- **Setup** as SCEN-110, one full cycle observed
- **Actions** announce sleep, then never wake the simulator again; advance past
  `overdue_at` with **no inbound message at all**
- **Assertions** the transition to `isolated` is made by the liveness timer, not
  by an arriving message (F-040-09); `missed_wake_count` increments per missed
  window; a `device_wake_missed` event is raised; the device is not reported as
  `sleeping` at any point after `overdue_at`

### SCEN-112 Unannounced absence is never reported as sleep
- **Level** integration · **Milestone** M5 · **Proves** SAFETY-021, SAFETY-012
- **Setup** battery-mode device, awake
- **Actions** three variants — (a) kill the session so the LWT fires with
  `connection_lost`; (b) publish an offline status with an unrecognised `reason`;
  (c) publish a sleep announcement whose `expected_wake_ms` is a year ahead
- **Assertions** (a) and (b) derive `isolated` immediately; (c) derives
  `sleeping` bounded by the **edge-computed** window and becomes `isolated` at
  `overdue_at`, proving the device's own wake time is advisory

### SCEN-113 Manual water for a sleeping device is held and delivered at wake
- **Level** e2e · **Milestone** M8 · **Proves** SAFETY-001, SAFETY-010
- **Setup** battery device asleep, plant with an actuator binding, no lockout
- **Actions** `POST /water`; observe; wake the device; let the cycle complete;
  restart the edge between the request and the wake
- **Assertions** the response is 202 with `status: "pending_for_device_wake"`,
  `expected_delivery_after`, and **no `command_id`**; no MQTT publish occurs on
  any `commands/*` topic while the device sleeps, verified by a spy subscriber;
  at wake exactly one `command.water` is published; the intent survives the edge
  restart and still delivers exactly once; exactly one `watering_event` results

### SCEN-114 A leak that appears during sleep refuses the pending intent at wake
- **Level** e2e · **Milestone** M8 · **Proves** SAFETY-003, SAFETY-012
- **Setup** battery device asleep with a pending water intent
- **Actions** raise the leak state before the wake; wake the device
- **Assertions** the safety gate is re-run at delivery and refuses; the intent
  moves to `refused` with reason `leak`; **nothing is published** on
  `commands/water`; the plant is in a leak lockout with no clear path from the
  API; the same holds for tank-below-minimum and for a rolling-cap exhaustion
  that occurred while the device slept

### SCEN-115 Budget and cooldown across deep sleep while isolated
- **Level** e2e · **Milestone** M8 · **Proves** SAFETY-014, SAFETY-015
- **Setup** battery device, valid enabled offline policy, isolated, bone-dry soil
- **Actions** run 48 h virtual across roughly 190 sleep/wake cycles; force a cold
  reset (not a timer wake) mid-cooldown; corrupt the RTC-retained checksum on one
  wake
- **Assertions** across ordinary timer wakes the RTC monotonic elapsed time is
  credited, so cooldowns expire and the rolling budget accrues normally; the cold
  reset and the failed checksum each fall back to "no time has passed" — the
  cooldown is not shortened and the budget is not replenished; total delivered
  never exceeds the policy budget or `FIRMWARE_MAX_DAILY_ML`

### SCEN-116 An undelivered intent expires, and a delivered one carries a fresh TTL
- **Level** e2e · **Milestone** M8 · **Proves** SAFETY-002
- **Setup** battery device with `wake_interval_seconds` 900
- **Actions** (a) request a dose and never wake the device, past
  `intent_expires_at`; (b) request a dose and wake the device normally
- **Assertions** (a) the intent becomes `expired_before_wake`, nothing is
  published, and the plant is untouched; (b) the delivered `command.water` has
  `issued_at` within seconds of the wake — **not** of the operator's request —
  and its `expires_at` is `issued_at + command_ttl`; the device accepts it with
  `clock_synced: true`, having received a fresh `edge.time` first

### SCEN-117 The device stays awake for a whole watering cycle
- **Level** e2e · **Milestone** M8 · **Proves** SAFETY-001, SAFETY-011
- **Setup** battery device, dose delivered at wake, `awake_budget_seconds`
  deliberately shorter than the dose duration
- **Actions** run the dose to completion; then cut power mid-dose on a second run
- **Assertions** the device does not sleep while the pump is energised — the
  awake budget does not truncate an active cycle; `command.result` is published
  and acknowledged **before** the sleep announcement; on the power-cut run the
  device boots pump-off and reports `status: "interrupted"` with
  `delivered_ml: null` at its next wake, and no second dose is issued for it

---

## Coverage matrix

| Invariant | Scenarios |
|---|---|
| SAFETY-001 | SCEN-010, SCEN-011, SCEN-113, SCEN-117 |
| SAFETY-002 | SCEN-025, SCEN-030, SCEN-031, SCEN-073, SCEN-074, SCEN-075, SCEN-076, SCEN-077, SCEN-078, SCEN-079, SCEN-116 |
| SAFETY-003 | SCEN-040, SCEN-041, SCEN-114 |
| SAFETY-004 | SCEN-042, SCEN-043, SCEN-081 |
| SAFETY-005 | SCEN-022, SCEN-023, SCEN-024, SCEN-070 |
| SAFETY-006 | SCEN-034, SCEN-071, SCEN-072 |
| SAFETY-007 | SCEN-032, SCEN-033, SCEN-045 |
| SAFETY-008 | SCEN-012, SCEN-060, SCEN-062, SCEN-082 |
| SAFETY-009 | SCEN-061 |
| SAFETY-010 | SCEN-051, SCEN-052, SCEN-113 |
| SAFETY-011 | SCEN-026, SCEN-117 |
| SAFETY-012 | SCEN-014, SCEN-023, SCEN-025, SCEN-041, SCEN-043, SCEN-074, SCEN-075, SCEN-112, SCEN-114 |
| SAFETY-013 | SCEN-090, SCEN-093, SCEN-094, SCEN-107 |
| SAFETY-014 | SCEN-091, SCEN-096, SCEN-097, SCEN-107, SCEN-115 |
| SAFETY-015 | SCEN-097, SCEN-098, SCEN-077, SCEN-115 |
| SAFETY-016 | SCEN-092, SCEN-100, SCEN-101, SCEN-102 |
| SAFETY-017 | SCEN-099, SCEN-105 |
| SAFETY-018 | SCEN-106 |
| SAFETY-019 | SCEN-094, SCEN-095, SCEN-103 |
| SAFETY-020 | SCEN-104 |
| SAFETY-021 | SCEN-110, SCEN-111, SCEN-112 |

SAFETY-018 and SAFETY-020 have one scenario each, because each states a single
crisp property with no interesting variations; the rest carry two or more.

Every invariant has at least two scenarios except SAFETY-009, SAFETY-018, and
SAFETY-020, which have one each by nature — SAFETY-009 is a differential test
that subsumes every other scenario. SAFETY-011 gained a second in SCEN-117,
where converging to pump-off has to survive a power cut inside a battery
device's awake window as well as an ordinary reset, and is still verified
physically in M11.
