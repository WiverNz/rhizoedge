# Rhizo MQTT Protocol — v1

**Status:** normative specification. This document is the contract. It is
specific enough that the Device Simulator and the ESP32 firmware can be written
independently and interoperate.

**Conformance language:** MUST / MUST NOT / SHOULD / MAY as in RFC 2119.

> **Revised 2026-08-26, before implementation.** At the time this revision was
> accepted, M1 had not started, so v1 was still unwritten and no compatibility
> was owed. Three changes:
> the four `telemetry/*` topics became one batched `telemetry` topic carrying
> typed measurement samples plus a separate `actuator` state topic
> ([ADR-017](../adr/017-extensible-measurement-model.md)); a retained `policy`
> topic and a device→edge `events` topic were added for offline autonomy
> ([ADR-015](../adr/015-device-offline-autonomy.md)); and `device.status` now
> declares device capabilities
> ([ADR-016](../adr/016-plant-binding-and-policy-model.md)); and a live
> `time` topic carries **Edge time synchronisation over MQTT**, replacing the
> briefly-considered Edge-hosted NTP service
> ([ADR-013](../adr/013-clock-and-time-semantics.md)). See
> [versioning-policy.md](versioning-policy.md) §pre-implementation for why this
> is not a v2.

Related: [ADR-002](../adr/002-mqtt-topic-versioning-and-qos.md) (rationale),
[versioning-policy.md](versioning-policy.md) (evolution rules),
[ADR-013](../adr/013-clock-and-time-semantics.md) (time semantics).

---

## 1. Transport

| Property | Value |
|---|---|
| Protocol | MQTT 3.1.1 |
| Broker | Eclipse Mosquitto 2.x |
| Default port | 1883 (plaintext; TLS deferred to M13) |
| Clean session | `true` — MUST |
| Keepalive | 60 s (device), 30 s (edge) |
| Authentication | username/password, anonymous access disabled |

**Clean session MUST be `true`** on both devices and the edge. A persistent
session would cause the broker to queue water commands for an offline device and
deliver the backlog on reconnect, which directly contradicts SAFETY-002.
Retained messages are unaffected by session cleanliness.

Client id MUST equal the `device_id` for devices. The edge uses
`rhizo-edge-{edge_id}`.

---

## 2. Device ID grammar

```text
device_id  =  ALPHANUM  1*30(ALPHANUM / "-")  ALPHANUM
ALPHANUM   =  %x61-7A / %x30-39            ; a-z 0-9
```

- Length 3–32 characters inclusive.
- Lowercase only. Implementations MUST NOT case-fold; an id with an uppercase
  character is invalid, not normalised.
- MUST NOT contain `+`, `#`, `/`, or whitespace. This is a security requirement,
  not a style rule: such characters permit topic injection.

A receiver MUST reject any message whose topic contains an invalid `device_id`.

---

## 3. Topic hierarchy

Base namespace: `rhizo/v1/`

| Topic | Publisher | Subscriber | QoS | Retained |
|---|---|---|---|---|
| `rhizo/v1/devices/{id}/telemetry` | device | edge | 1 | no |
| `rhizo/v1/devices/{id}/actuator` | device | edge | 1 | no |
| `rhizo/v1/devices/{id}/events` | device | edge | 1 | no |
| `rhizo/v1/devices/{id}/status` | device | edge | 1 | **yes** |
| `rhizo/v1/devices/{id}/config` | edge | device | 1 | **yes** |
| `rhizo/v1/devices/{id}/policy` | edge | device | 1 | **yes** |
| `rhizo/v1/devices/{id}/time` | edge | device | 1 | **no — never** |
| `rhizo/v1/devices/{id}/commands/water` | edge | device | 1 | no |
| `rhizo/v1/devices/{id}/commands/tare` | edge | device | 1 | no |
| `rhizo/v1/devices/{id}/commands/calibrate` | edge | device | 1 | no |
| `rhizo/v1/devices/{id}/commands/result` | device | edge | 1 | no |
| `rhizo/v1/devices/{id}/events/ack` | edge | device | 1 | **no — never** |

**`telemetry` carries a batch**, not one measurement kind. One message per
sampling cycle, so the sample set shares one envelope and one deduplication key
and cannot be split by a redelivery. Adding a measurement kind costs an enum
variant, not a topic.

**`policy` is separate from `config`** deliberately. Both are retained and
versioned, but `config` tunes a device while `policy` authorises it to act
alone; they have different validation, different rollback semantics, and very
different safety weight ([ADR-015](../adr/015-device-offline-autonomy.md) §7).

**`events` is device→edge replay** of history buffered while isolated. It is not
a second telemetry channel: it carries events that already happened, with
device-generated ids, deduplicated identically to everything else.

**`events/ack` closes the replay loop** (§5.13). Without it a device has no way
to learn that its buffered history is safely on the edge, so it must either keep
replaying it for ever or discard it on a guess. Both are wrong; an explicit
acknowledgement is the only mechanism that lets a bounded buffer be emptied
without losing history.

**`time` MUST NOT be retained.** A retained timestamp is stale the instant it is
stored, and a device applying one after a reconnect would set its clock to
whenever the message was published. This is the one topic where retention would
be actively harmful in a way that is easy to introduce by accident, so it is
stated twice: here, and in the retention rules below.

### Retention rules — normative

- `status`, `config`, and `policy` MUST be published with the retain flag set.
- **All other topics MUST NOT be published with the retain flag set.**
  Publishing a retained message on any `commands/*` topic is a protocol
  violation: the broker would redeliver it on every reconnect indefinitely,
  causing repeated watering. Publishing retained telemetry is also a violation:
  it would be served to new subscribers as though current.
- **`time` in particular MUST NOT be retained.** A retained timestamp would be
  delivered to a reconnecting device as though current, moving its clock
  backwards to the moment of publication and making expired commands appear
  valid — a direct route to violating SAFETY-002.
- **`events/ack` MUST NOT be retained**, for the same shape of reason. An
  acknowledgement is a statement about one moment: "as of now, everything
  through this sequence is committed here." Retained, the broker would repeat
  that statement to a device reconnecting hours later, and the device would
  delete buffered history on the strength of a claim about a database that may
  since have been restored from an older backup. Acknowledgements are live
  messages or they are nothing.

### Subscriptions

- Edge subscribes to `rhizo/v1/devices/+/#`.
- A device MUST subscribe to exactly these **seven exact topics**, and to no
  wildcard:

  | | |
  |---|---|
  | `rhizo/v1/devices/{own_id}/config` | `rhizo/v1/devices/{own_id}/commands/water` |
  | `rhizo/v1/devices/{own_id}/policy` | `rhizo/v1/devices/{own_id}/commands/tare` |
  | `rhizo/v1/devices/{own_id}/time` | `rhizo/v1/devices/{own_id}/commands/calibrate` |
  | `rhizo/v1/devices/{own_id}/events/ack` | |

- A device MUST NOT subscribe to `telemetry`, `actuator`, `events`, `status`, or
  `commands/result`, all of which it publishes.

**Exact topics, not `commands/+`.** An earlier revision specified a
`commands/+` filter. That filter also matches `commands/result`, which is the
device's own output, and MQTT 3.1.1 offers no way to subtract a child from a
wildcard and no "no local" subscription option. A device holding it was
therefore delivered every result it published, and the rule had to be softened
to "receive it but never act on it" — a property of the dispatch code rather
than of the wire, one refactor away from being untrue, and paid for in
round-trips in the meantime.

Exact topics make it a property of the subscription set instead: the broker
cannot deliver the device its own output, so no code has to remember not to act
on it. The cost is that adding a command kind adds a subscription; that is a
one-line change to a list this specification already enumerates, made at the
same time as the new topic itself, and it is checked by conformance (§11).

**Adding a command kind is therefore a protocol change, by design.** A device
built against v1 will not receive a topic v1 does not define, which is the
correct behaviour: an unreceived command produces no result and the edge marks
it failed, whereas a wildcard would deliver a payload the device cannot parse
and must discard anyway. The wildcard bought nothing but the delivery of the
device's own results.

### QoS

QoS 1 for everything. QoS 0 and QoS 2 MUST NOT be used. Consumers MUST be
idempotent (§6).

---

## 4. Message envelope

Every payload on every topic is a JSON object with this envelope:

```json
{
  "v": 1,
  "kind": "telemetry.batch",
  "message_id": "018fd6c4-7b4a-7c31-9e2a-3f5b1d8c6a20",
  "device_id": "plant-node-01",
  "boot_id": "018fd6b0-1122-4000-8000-aabbccddeeff",
  "sequence": 81273,
  "device_time_ms": 1756121400000,
  "clock_synced": true,
  "data": { }
}
```

### Envelope fields

| Field | Type | Required | Rules |
|---|---|---|---|
| `v` | integer | yes | MUST be `1`. A receiver MUST reject any other value. |
| `kind` | string | yes | see §5; MUST match the topic |
| `message_id` | UUID string | yes | **UUIDv7**; globally unique transport identity |
| `device_id` | string | yes | MUST equal the `device_id` in the topic |
| `boot_id` | UUID string | yes for device→edge | regenerated at every device boot |
| `sequence` | integer ≥ 0 | yes for device→edge | monotonically increasing within a `boot_id` |
| `device_time_ms` | integer | no | Unix epoch ms UTC; advisory only |
| `clock_synced` | boolean | yes for device→edge | `false` means `device_time_ms` is meaningless |
| `data` | object | yes | kind-specific payload |

Notes:

- **`message_id` MUST be UUIDv7.** Time-ordered ids sort usefully as a primary
  key and give a cheap consistency check on device clocks. A device without a
  synced clock MAY emit UUIDv4 and MUST set `clock_synced: false`.
- **`device_id` is duplicated** in the payload deliberately. A receiver MUST
  reject a message where the payload `device_id` differs from the topic
  `device_id` — the mismatch indicates misrouting or spoofing, and guessing is
  worse than refusing.
- **`sequence` is not the dedup key.** It is used only for gap and regression
  detection. A device that reboots may legitimately reuse a sequence value.
- Unknown fields MUST be ignored by receivers (forward compatibility, §9).

### Numeric conventions

- All floats are JSON numbers. `NaN` and `Infinity` MUST NOT be emitted; a
  sensor producing a non-finite value MUST publish `null` for that field.
- **Measurement units come from the `kind`** (§5.1), never from the sender's
  choice. The `unit` field is a consistency check and a mismatch is a rejection.
- For non-measurement fields, units remain encoded in the field name (`_ml`,
  `_ms`, `_seconds`, `_percent`), so changing one requires a rename and is
  therefore a breaking change (§9).
- Absent optional fields MAY be omitted or sent as `null`; the two are
  equivalent.

---

## 5. Message kinds and payloads

### 5.1 Measurement kinds — normative

Every measurement is a typed `kind` with **exactly one canonical unit** and a
physical plausibility range, defined once in `rhizo-mqtt-contract` as compile-time
data the firmware can use ([ADR-017](../adr/017-extensible-measurement-model.md)).

| `kind` | Unit | Class | Valid range |
|---|---|---|---|
| `soil_moisture` | `vwc_percent` | scalar | 0.0 – 100.0 |
| `soil_temperature` | `celsius` | scalar | −20.0 – 80.0 |
| `soil_ec` | `us_cm` | scalar | 0 – 20000 |
| `soil_ph` | `ph` | scalar | 0.0 – 14.0 |
| `ambient_temperature` | `celsius` | scalar | −40.0 – 85.0 |
| `ambient_humidity` | `percent_rh` | scalar | 0.0 – 100.0 |
| `illuminance` | `lux` | scalar | 0.0 – 200000.0 |
| `pot_weight` | `gram` | scalar | 0.0 – 100000.0 |
| `tank_level` | `percent` | scalar | 0.0 – 100.0 |
| `leak_state` | `boolean` | boolean | — |
| `nitrate_concentration` | `mg_l` | scalar | 0.0 – 5000.0 |

Rules:

- A sender MUST use the canonical unit for the kind. The `unit` field is a
  **check, not a choice**: a sample whose `unit` disagrees with its `kind` MUST be
  rejected.
- A receiver MUST decode an unrecognised `kind` to `Unknown`, MUST store the
  sample, and MUST treat it as **advisory only** — it never gates actuation
  (SAFETY-012).
- `nitrate_concentration` is publishable **only** by a genuinely calibrated ion
  sensor, and SHOULD carry `calibration_ref`. There is deliberately **no kind for
  nitrogen, phosphorus, or potassium**: cheap "NPK" probes derive those from EC by
  an undisclosed formula, and publishing them would be a false claim about a real
  plant. EC is EC.

### 5.2 `telemetry.batch` → `telemetry`

One message per sampling cycle, carrying every sample taken in that cycle.

```json
{
  "v": 1, "kind": "telemetry.batch",
  "message_id": "018fd6c4-7b4a-7c31-9e2a-3f5b1d8c6a20",
  "device_id": "plant-node-01",
  "boot_id": "018fd6b0-1122-4000-8000-aabbccddeeff",
  "sequence": 81273,
  "device_time_ms": 1756121400000,
  "clock_synced": true,
  "data": {
    "batch_id": "018fd6c4-7b4a-7c31-9e2a-3f5b1d8c6a21",
    "samples": [
      { "point": "default", "kind": "soil_moisture",
        "value": 31.7, "unit": "vwc_percent", "quality": "ok",
        "sensor_id": "soil-0" },
      { "point": "default", "kind": "soil_temperature",
        "value": 21.4, "unit": "celsius", "quality": "ok",
        "sensor_id": "soil-0" },
      { "point": "default", "kind": "soil_ec",
        "value": 840, "unit": "us_cm", "quality": "uncalibrated",
        "sensor_id": "soil-0" },
      { "point": "reservoir", "kind": "tank_level",
        "value": 72.0, "unit": "percent", "quality": "ok",
        "sensor_id": "tank-0" },
      { "point": "tray", "kind": "leak_state",
        "value": false, "unit": "boolean", "quality": "ok",
        "sensor_id": "leak-0" }
    ]
  }
}
```

| Field | Type | Required | Rules |
|---|---|---|---|
| `batch_id` | UUID | yes | groups the cycle; stored so charts can align samples |
| `samples` | array | yes | 1–64 entries; an empty batch MUST NOT be published |
| `samples[].point` | string | no, default `"default"` | device_id grammar; e.g. `depth_30cm`, `ambient` |
| `samples[].kind` | string | yes | §5.1 |
| `samples[].value` | number \| boolean \| null | yes | `null` means "read failed"; see below |
| `samples[].unit` | string | yes | MUST match the kind's canonical unit |
| `samples[].quality` | string | yes | `ok` \| `uncalibrated` \| `suspect` \| `fault` |
| `samples[].sensor_id` | string | no | MUST match a declared capability when present |
| `samples[].calibration_ref` | string | no | opaque reference to a calibration record |

Normative behaviour:

- A read failure MUST publish the sample with `"value": null` and
  `"quality": "fault"`. It MUST NOT publish the last good value, and MUST NOT
  omit the sample silently — a repeated stale value would defeat both staleness
  and stuck-sensor detection.
- An uncalibrated sensor MUST publish `"quality": "uncalibrated"`. The edge stores
  it and MUST NOT use it for control (SAFETY-005, SAFETY-017).
- One sample out of range does **not** invalidate the batch. The receiver stores
  that sample with a null value and raises `sensor_invalid`, keeping the rest
  (§10).
- A device MUST publish all samples of one cycle in one batch. Splitting a cycle
  across messages breaks batch atomicity and is a conformance failure.

### 5.3 `actuator.state` → `actuator`

Published when actuator state changes, not periodically. Actuator state is state,
not a measurement, which is why it is not in the batch.

```json
"data": {
  "actuator_id": "pump-0",
  "kind": "irrigation_pump",
  "active": false,
  "last_run_ms": 6120,
  "delivered_today_ml": 90.0,
  "faulted": false
}
```

### 5.4 `device.events` → `events` (device → edge)

Replay of history buffered while the device was isolated
([ADR-015](../adr/015-device-offline-autonomy.md) §6, §8). Sent in batches after
reconnection, oldest first.

```json
"data": {
  "replay": true,
  "complete": false,
  "events": [
    { "event_id": "018fd7c0-…", "device_seq": 4411, "tier": "audit",
      "kind": "watering.offline_autonomous",
      "monotonic_ms": 8814000, "device_time_ms": null,
      "detail": { "policy_version": 7, "delivered_ml": 35.0,
                  "trigger_value": 26.4, "duration_ms": 4270 } },
    { "event_id": "018fd7c1-…", "device_seq": 4412, "tier": "audit",
      "kind": "offline.refused",
      "monotonic_ms": 32400000,
      "detail": { "reason": "tank_unknown" } },
    { "event_id": "018fd7c2-…", "device_seq": 4413, "tier": "audit",
      "kind": "history.gap",
      "detail": { "from_seq": 4100, "to_seq": 4380,
                  "lost_count": 281, "lost_tier": "telemetry" } }
  ]
}
```

| Field | Type | Rules |
|---|---|---|
| `replay` | boolean | `true` for buffered history, `false` for live events |
| `complete` | boolean | `true` on the final batch; the edge holds the plant in `Uncertain` until it sees this |
| `events[].event_id` | UUID | generated **once** at buffering time; MUST be stable across every replay |
| `events[].device_seq` | integer | strictly increasing for the lifetime of the device, across reboots; used to detect gaps and to acknowledge (§5.13) |
| `events[].tier` | string | `audit` \| `telemetry` |
| `events[].monotonic_ms` | integer | elapsed since boot — always meaningful |
| `events[].device_time_ms` | integer \| null | wall time if the clock was synced; `null` otherwise |

Normative behaviour:

- A device MUST NOT regenerate `event_id` on replay. A regenerated id defeats
  deduplication and would create duplicate history (SAFETY-016).
- A device MUST retain replayed events until the edge acknowledges them with an
  `event.ack` (§5.13), so an edge crash mid-reconciliation loses nothing.
  QoS 1 is not sufficient and MUST NOT be treated as sufficient: it gives the
  device the *broker's* acknowledgement, not the edge's, and the broker acks a
  message the edge may never have committed. An unacknowledged replay is
  repeated on the next reconnection; the edge deduplicates on `event_id`, so
  repeating costs bandwidth and nothing else, while discarding on a guess loses
  history permanently.
- A `history.gap` event MUST be emitted whenever eviction loses events, carrying
  the lost `device_seq` range and count (SAFETY-020).
- **A `history.gap` marker takes its `device_seq` when it is first sent, not
  when the loss occurs.** A run of losses is accumulated locally — range widened
  and count raised — for as long as it has not been transmitted; the moment it
  enters a replay batch it is fixed and never changes again, and any later loss
  opens a new marker with its own `event_id`.

  Both halves are load-bearing. A marker that could still change after being
  sent would be dropped by the edge's own deduplication as a duplicate of the
  smaller earlier version, permanently under-reporting the loss. And a marker
  that took its sequence at the moment of the first loss would sit *below*
  events buffered afterwards, so a cumulative acknowledgement covering those
  events would also cover a marker the edge had never received — and, because
  acknowledgement only moves forward, no later acknowledgement could ever cover
  it again. Allocating the sequence at send time makes a marker's sequence
  always higher than anything the edge could have acknowledged, because it did
  not exist when the edge spoke. The position of the loss is carried by
  `from_seq`/`to_seq`, which is where it belongs.
- **An acknowledgement never applies to an unsent gap**, which follows from the
  rule above rather than needing a special case: a device MUST NOT discard a
  gap marker that has not yet appeared in a replay batch, however high the
  acknowledged sequence.
- The edge MUST NOT issue a water command to a plant whose replay has not
  reported `"complete": true` (SAFETY-016).

### 5.5 `device.status` → `status` (retained)

Published retained on connect, on config change, and at least every
`5 × telemetry_interval` as a heartbeat.

```json
{
  "v": 1, "kind": "device.status",
  "message_id": "018fd6c4-…", "device_id": "plant-node-01",
  "boot_id": "018fd6b0-…", "sequence": 1,
  "device_time_ms": 1756121400000, "clock_synced": true,
  "data": {
    "boot_generation": 42,
    "status": "online",
    "firmware_version": "0.1.0",
    "protocol_version": 1,
    "applied_config_version": 7,
    "uptime_ms": 912344,
    "free_heap_bytes": 143216,
    "rssi_dbm": -58,
    "applied_policy_versions": { "monstera-01": 7 },
    "connectivity": { "mode": "connected", "isolated_ms": 0 },

    "capabilities": {
      "sensors": [
        { "sensor_id": "soil-0", "point": "default",
          "kinds": ["soil_moisture", "soil_temperature", "soil_ec"],
          "present": true, "healthy": true, "errors": 0,
          "calibrated": true },
        { "sensor_id": "tank-0", "point": "reservoir",
          "kinds": ["tank_level"],
          "present": true, "healthy": true, "errors": 0 },
        { "sensor_id": "leak-0", "point": "tray",
          "kinds": ["leak_state"],
          "present": true, "healthy": true, "errors": 0 }
      ],
      "actuators": [
        { "actuator_id": "pump-0", "kind": "irrigation_pump",
          "present": true, "healthy": true }
      ]
    },

    "limits": {
      "max_run_seconds": 20,
      "max_ml_per_run": 80.0,
      "max_daily_ml": 500.0
    }
  }
}
```

`limits` reports the compile-time hard limits for observability. **Reporting is
one-way.** No message can change them (SAFETY-007).

`boot_generation` is a positive monotonic counter persisted by the device and
incremented before each boot is announced. It orders status effects across
random `boot_id` values; it is not a replacement for `boot_id`, and it MUST NOT
be derived from the device wall clock. Losing or rolling this counter back is a
persistent-state fault, not permission to make an old boot current again.

Within one `boot_generation`, normal status publications are ordered by the
envelope `sequence`. The fixed `sequence: 0` LWT is one terminal logical status
identified by its `message_id`. The Edge stores only the current generation,
normal-status high-water sequence, and current boot's LWT identity. Therefore
an old retained status or LWT cannot refresh `last_seen_at`, even after its
short-lived transport marker has been pruned.

`status` MUST be one of `"online"` or `"offline"`.

#### Capabilities — normative

`capabilities` is how a device **declares** what it can do. The edge MUST NOT
assume any capability that was not declared: `device == pump controller` is not
an assumption this protocol permits
([ADR-016](../adr/016-plant-binding-and-policy-model.md)).

| Field | Rules |
|---|---|
| `sensors[].sensor_id` | device_id grammar; stable across reboots; unique per device |
| `sensors[].kinds` | the measurement kinds this sensor can produce (§5.1) |
| `sensors[].point` | default measurement point for its samples |
| `sensors[].calibrated` | absent means "not applicable"; `false` means samples will carry `quality: "uncalibrated"` |
| `actuators[].actuator_id` | stable, unique per device |
| `actuators[].kind` | `irrigation_pump` in V1. `valve`, `grow_light`, `fan`, `heater`, `humidifier`, `fertiliser_dosing_pump` are **reserved** — representable, with no implementation and no automation semantics |

A device with **no actuators** is a normal, fully supported device. An edge MUST
reject a binding or a policy naming a capability the device did not declare.

`applied_policy_versions` maps `plant_id` to the offline policy version currently
active on this device, so the edge can detect policy drift the same way it
detects config drift (§5.11).

`connectivity.mode` is the device's own view: `connected`, or `isolated` with
`isolated_ms` giving the elapsed duration. It is advisory — the edge determines
liveness from message arrival — but it is what lets the UI say "this device ran
alone for six hours" after a reconnection.

### 5.6 Last Will and Testament

Every device MUST configure an LWT at connect time:

```text
topic  : rhizo/v1/devices/{device_id}/status
qos    : 1
retain : true
payload:
```

```json
{
  "v": 1, "kind": "device.status",
  "message_id": "<UUID generated at connect time>",
  "device_id": "plant-node-01",
  "boot_id": "018fd6b0-…", "sequence": 0,
  "clock_synced": false,
  "data": { "boot_generation": 42, "status": "offline", "reason": "connection_lost" }
}
```

The LWT payload is fixed at connect time, so its `message_id` is generated then.
A receiver MUST tolerate an LWT arriving with a `message_id` it has already seen
(if the device reconnected and disconnected on the same will) — the dedup path
handles it correctly by ignoring the duplicate.

On a clean disconnect a device SHOULD publish `status: offline` with
`reason: "shutdown"` before disconnecting.

### 5.7 `device.config` → `config` (retained, edge → device)

```json
{
  "v": 1, "kind": "device.config",
  "message_id": "018fd7a0-…",
  "device_id": "plant-node-01",
  "data": {
    "config_version": 7,
    "telemetry_interval_seconds": 300,
    "pump": { "ml_per_second": 8.2, "enabled": true },
    "tank": { "min_percent": 15.0 },
    "sensors": { "soil": true, "weight": false, "tank": true, "leak": true }
  }
}
```

| Field | Type | Range |
|---|---|---|
| `config_version` | integer | monotonically increasing, edge-owned |
| `telemetry_interval_seconds` | integer | 10 – 3600 |
| `pump.ml_per_second` | float | 0.1 – 100.0 |
| `pump.enabled` | boolean | — |
| `tank.min_percent` | float | 0.0 – 100.0 |

Device behaviour:

1. On receipt, validate. An invalid config MUST be rejected and the previously
   applied config retained; the device reports the old
   `applied_config_version` and publishes a `device.status` unchanged.
2. A valid config is persisted to NVS and applied.
3. `applied_config_version` in the next `device.status` MUST reflect it.
4. A config with `config_version` less than or equal to the applied version MUST
   be ignored (protects against retained-message replay after a rollback).

**There is no time-server field.** The device's wall clock comes from the Edge
over the MQTT connection it already has (§5.12), so there is nothing to
configure: the Edge is reachable by definition or the device is isolated, in
which case no configuration would help.

**`config` MUST NOT contain safety limits.** The device ignores any field it
does not recognise, so an attempt to smuggle `max_ml_per_run` has no effect.

### 5.8 `command.water` → `commands/water` (edge → device)

```json
{
  "v": 1, "kind": "command.water",
  "message_id": "018fd7b1-…",
  "device_id": "plant-node-01",
  "data": {
    "command_id": "018fd7b1-4c2e-7f10-a3b8-9d1e2f304050",
    "requested_ml": 40.0,
    "issued_at_ms": 1756121500000,
    "expires_at_ms": 1756121620000
  }
}
```

| Field | Type | Rules |
|---|---|---|
| `command_id` | UUIDv7 | idempotency key; the device dedups on this |
| `requested_ml` | float | > 0.0 |
| `issued_at_ms` | integer | edge clock, Unix epoch ms UTC |
| `expires_at_ms` | integer | MUST be > `issued_at_ms` |

#### Device validation — normative order

The device MUST evaluate these checks in this order and MUST publish a
`command.result` for every outcome:

```text
1. command_id already in the dedup ring
        → AlreadyExecuted: re-publish the STORED result. MUST NOT actuate.
2. clock_synced == false
        → reject(clock_unsynced)        [SAFETY-002, SAFETY-012]
3. now_ms > expires_at_ms + MAX_CLOCK_SKEW_MS
        → reject(expired)               [SAFETY-002]
4. requested_ml <= 0 or not finite
        → reject(malformed_command)
5. leak_detected == true
        → reject(leak_detected)         [SAFETY-003]
6. leak_detected == null (unknown)
        → reject(leak_unknown)          [SAFETY-012]
7. tank_level_percent == null, not finite, or tank.min_percent not finite
        → reject(tank_unknown)          [SAFETY-012]
8. tank_level_percent <= tank.min_percent
        → reject(tank_low)              [SAFETY-004]
9. pump faulted, pump.enabled == false,
   or pump.ml_per_second not finite or <= 0
        → reject(pump_unavailable)
10. requested_ml > FIRMWARE_MAX_ML_PER_RUN
        → CLAMP to FIRMWARE_MAX_ML_PER_RUN, set clamped = true   [SAFETY-007]
11. delivered_today_ml not finite,
    or delivered_today_ml + effective_ml > FIRMWARE_MAX_DAILY_ML
        → reject(over_daily_max)        [SAFETY-007]
12. run_ms = effective_ml / pump.ml_per_second * 1000
    if run_ms > FIRMWARE_MAX_RUN_SECONDS * 1000
        → CLAMP run_ms, recompute effective_ml, set clamped = true [SAFETY-007]
13. persist (command_id, started_at, requested_ml) to NVS   [SAFETY-011]
14. actuate
```

Steps 10 and 12 clamp; every other failure rejects. Step 13 **MUST** complete
before step 14, so an interrupted dose is detectable on the next boot.

**Non-finite guard inputs are `Unknown`, never permission.** §4 forbids emitting
`NaN` or `Infinity`, so a non-finite value reaching the gate means the reading or
the configuration is unusable, not that the condition is satisfied. Every
comparison against `NaN` is false, so a gate written only as `value <= limit`
would *pass* a `NaN` and water on unusable evidence — the exact SAFETY-012
failure. Steps 7, 9 and 11 therefore name their non-finite inputs explicitly and
map each to the refusal its usable counterpart would produce: an unreadable tank
level or an unusable tank minimum is `tank_unknown` (not `tank_low`, which is a
*measured* condition), an unusable pump calibration is `pump_unavailable`, and an
unreadable rolling total is `over_daily_max` because a device that cannot prove
it is under budget MUST assume it is not.

Step 9 covers `pump.ml_per_second` for a second reason: step 12 divides by it. A
device whose calibration is absent or non-positive cannot compute a bounded run
duration, so the pump is genuinely unavailable and MUST be refused before the
division is reached.

The reference implementation of steps 1–12 is
`rhizo_mqtt_contract::validate_water_command`, which both the simulator and the
firmware call. There MUST NOT be a second implementation
([ADR-008](../adr/008-shared-code-simulator-and-firmware.md)).

### 5.9 `command.tare` and `command.calibrate`

```json
"data": { "command_id": "018fd7c2-…", "issued_at_ms": …, "expires_at_ms": … }
```

`command.calibrate`:

```json
"data": {
  "command_id": "018fd7c3-…",
  "run_seconds": 10.0,
  "issued_at_ms": …, "expires_at_ms": …
}
```

`command.calibrate` runs the pump for a fixed duration so the operator can
measure the delivered volume, and its delivered volume counts toward
`FIRMWARE_MAX_DAILY_ML`.

#### Calibration goes through the full §5.8 gate — normative

A calibration run is a real dose into a real pot from a real reservoir. It can
overflow, it can run a pump dry, and it can be issued while a leak is detected.
Nothing about the operator's intent changes what the water does, so **every step
of §5.8 applies, in order, with no exemption**.

A device MUST convert the request to the volume it implies and put that through
`validate_water_command` unchanged:

```text
synthetic_ml := run_seconds × config.pump.ml_per_second
```

where `pump.ml_per_second` is the value from the currently applied
`device.config` (§5.6) — the same figure the device uses to time an ordinary
dose. The synthetic volume is what the gate sees as `requested_ml`; the
`command_id`, `issued_at_ms`, and `expires_at_ms` are the calibration's own.

Consequences, which follow rather than being separate rules:

- Steps 10 and 11 apply. A calibration whose implied volume exceeds
  `FIRMWARE_MAX_ML_PER_RUN` is **clamped**, and the result reports
  `clamped: true` together with the duration that actually ran — which is the
  number a calibration needs, and is why clamping is more useful here than
  refusing. One that would exceed the daily total is refused with
  `over_daily_max`.
- A device with no leak sensor refuses a calibration for `leak_unknown`, like
  any other actuation (step 6, SAFETY-012).
- The delivered volume is recorded against the rolling 24-hour total. A
  calibration is not free water.

An operator wanting a longer run reduces `run_seconds` until the implied volume
is inside the limit, or performs several runs and sums the measurements.

**A device MUST NOT implement a second validator for calibration**, nor a subset
copy of the §5.8 checks. The synthetic-volume mapping exists precisely so that
one gate serves both paths: a subset would be a second implementation of the
rules, which ADR-008 forbids because it makes every simulator-based safety test
prove something about only one of them.

### 5.10 `command.result` → `commands/result` (device → edge)

```json
{
  "v": 1, "kind": "command.result",
  "message_id": "018fd7b5-…",
  "device_id": "plant-node-01",
  "boot_id": "018fd6b0-…", "sequence": 81275,
  "device_time_ms": 1756121506120, "clock_synced": true,
  "data": {
    "command_id": "018fd7b1-4c2e-7f10-a3b8-9d1e2f304050",
    "status": "completed",
    "requested_ml": 40.0,
    "delivered_ml": 40.0,
    "duration_ms": 4878,
    "clamped": false,
    "reason": null,
    "delivered_today_ml": 130.0,
    "origin": "edge_command"
  }
}
```

| `status` | Meaning | Credits volume? |
|---|---|---|
| `completed` | pump ran to completion | yes, `delivered_ml` |
| `rejected` | refused by a validation step; pump never ran | no |
| `interrupted` | device restarted mid-dose; volume unknown | **conservatively, the full `requested_ml`** |
| `failed` | pump reported a hardware error mid-run | conservatively, `requested_ml` |

`reason` is required when `status` is `rejected` and MUST be one of:

```text
clock_unsynced   expired          malformed_command   leak_detected
leak_unknown     tank_unknown     tank_low            pump_unavailable
over_daily_max
```

Results MUST be retried until the broker acknowledges the QoS 1 publish, for up
to 60 s. A result that cannot be published MUST be persisted to NVS and
published after the next boot — a result is ledger data, not a sample.

---

### 5.11 `device.policy` → `policy` (retained, edge → device)

The offline policy a device may act on when isolated
([ADR-015](../adr/015-device-offline-autonomy.md)). One message carries the
policies for every plant this device serves.

```json
{
  "v": 1, "kind": "device.policy",
  "message_id": "018fd7a1-…", "device_id": "plant-node-01",
  "data": {
    "policies": [
      {
        "plant_id": "monstera-01",
        "policy_version": 7,
        "enabled": true,

        "actuator": {
          "actuator_id": "pump-0",
          "dose_ml": 35.0,
          "max_doses_per_cycle": 3,
          "absorption_wait_ms": 900000
        },

        "control_measurement": {
          "kind": "soil_moisture",
          "point": "default",
          "trigger_below": 28.0,
          "resume_above": 34.0,
          "confirm_duration_ms": 1800000,
          "max_age_ms": 900000
        },

        "required_measurements": [
          { "kind": "tank_level",  "point": "reservoir", "max_age_ms": 1800000 },
          { "kind": "leak_state",  "point": "tray",      "max_age_ms": 1800000 }
        ],
        "advisory_measurements": [
          { "kind": "soil_temperature", "point": "default" }
        ],

        "limits": {
          "cooldown_ms": 21600000,
          "max_volume_per_window_ml": 300.0,
          "window_ms": 86400000
        },

        "safety": {
          "require_leak_clear": true,
          "require_tank_above_percent": 15.0,
          "require_pump_healthy": true
        }
      }
    ]
  }
}
```

| Field | Rules |
|---|---|
| `policy_version` | `u32`, edge-owned, strictly monotonic per plant |
| `enabled` | **default `false`**; offline autonomy is opted into per plant |
| `actuator.dose_ml` | a **value**, never a formula; the only dose the device may deliver |
| `control_measurement.kind` | MUST be a **recognised scalar** kind (§5.1). `trigger_below` / `resume_above` / `confirm_duration_ms` are numeric-threshold semantics with no meaning for a boolean kind such as `leak_state`, and an unrecognised kind is advisory and can never gate actuation (SAFETY-012). A boolean safety input is a **veto**, declared under `safety` or `required_measurements`, never the control measurement. |
| `control_measurement.resume_above` | MUST be > `trigger_below` — hysteresis |
| `required_measurements` | absent or stale ⇒ refuse to actuate (SAFETY-017) |
| `advisory_measurements` | recorded; MUST NOT gate actuation |
| all durations | milliseconds, measured on the device's **monotonic** clock |

#### Device behaviour — normative

A device MUST apply **validate → stage → verify → activate → acknowledge**, and
MUST NOT begin using a policy before activation completes (SAFETY-019):

```text
1. parse                      failure ⇒ keep active policy, report, STOP
2. validate:
     - actuator_id is a declared capability
     - every referenced kind/point is producible by a declared sensor
     - dose_ml     <= FIRMWARE_MAX_ML_PER_RUN
     - dose_ml * max_doses_per_cycle <= max_volume_per_window_ml
     - max_volume_per_window_ml <= FIRMWARE_MAX_DAILY_ML
     - resume_above > trigger_below
     - every duration > 0
                              failure ⇒ keep active policy, report, STOP
3. write to staging with CRC
4. read back and verify CRC   failure ⇒ keep active policy, report, STOP
5. atomically activate
6. persist and report applied_policy_versions in device.status
```

Further requirements:

- A policy whose `policy_version` is **less than or equal to** the applied
  version MUST be ignored. This defends against a retained-message replay after a
  rollback silently regressing the device.
- Power loss at any step MUST leave exactly one valid policy active — the
  previous one before step 5, the new one after (SAFETY-019).
- A policy MUST NOT contain any field that could raise a firmware hard limit.
  Unrecognised fields are ignored, so an attempt to smuggle `max_ml_per_run` has
  no effect (SAFETY-007).
- Removing a plant's policy is expressed by publishing it with
  `"enabled": false` and a higher `policy_version`, **not** by omitting it. An
  omitted plant retains its last policy, because a dropped MQTT message must not
  be able to silently disable — or silently enable — autonomy.

### 5.12 `edge.time` → `time` (edge → device, **never retained**)

The device's wall clock is synchronised from the Edge over the MQTT connection it
already has. There is no NTP client on the device and no NTP daemon on the Edge
([ADR-013](../adr/013-clock-and-time-semantics.md)).

```json
{
  "v": 1, "kind": "edge.time",
  "message_id": "018fd8b2-…",
  "device_id": "plant-node-01",
  "data": { "edge_time_ms": 1756121400123 }
}
```

| Field | Type | Required | Rules |
|---|---|---|---|
| `edge_time_ms` | integer | yes | the Edge's wall clock, Unix epoch ms UTC, sampled at publish time |

#### Triggering — normative

No request topic exists. The device's retained `device.status` already announces
it, and that is the Edge's trigger:

1. On receiving **any** `device.status` from a device, the Edge MUST publish
   `edge.time` to that device.
2. While a device is online, the Edge MUST publish `edge.time` to it at least
   every `TIME_SYNC_INTERVAL_SECONDS`.
3. A device holding no valid synchronisation SHOULD republish its retained
   `device.status` (carrying `clock_synced: false`) at most once every 60 s until
   it is synchronised. The existing status message is the request; adding a
   dedicated request topic would be a second way to say the same thing.

#### Applying a synchronisation — normative

```text
on receipt of edge.time:
  if edge_time_ms <= last_applied_edge_time_ms
      → stale or duplicate
      → IGNORE: do not set the wall clock,
                do not update last_applied_edge_time_ms,
                and MUST NOT update synced_at_monotonic
  else:                                       -- strictly newer
      set wall clock from edge_time_ms
      last_applied_edge_time_ms := edge_time_ms
      synced_at_monotonic       := monotonic_now
```

The rule is **strictly increasing**, not merely non-decreasing, and the
difference matters. MQTT QoS 1 permits redelivery, so the same `edge_time_ms`
can arrive any number of times. If an equal value refreshed
`synced_at_monotonic`, a single captured or redelivered message replayed
indefinitely would hold `clock_synced` true forever while the device learned
nothing new about the Edge's clock — the validity window would measure *message
arrival*, not *synchronisation freshness*. **Only a strictly newer Edge timestamp
may extend the validity window.**

Refusing to go backwards fails in the safe direction for the other half of the
rule: an older `edge.time` arriving after a newer one — MQTT gives no ordering
guarantee across a reconnect — would move the device clock backwards and make
expired commands look valid again. A device clock slightly *ahead* of the Edge
expires commands sooner, which is conservative.

Because the Edge samples its wall clock at publish time and publishes at most
every `TIME_SYNC_INTERVAL_SECONDS`, two genuinely distinct synchronisations
practically never carry an equal millisecond value, so the strict rule costs
nothing in normal operation.

The device treats `edge_time_ms` as the time at **receipt**. The resulting error
is one-way broker latency — milliseconds on a LAN, three orders of magnitude
inside `MAX_CLOCK_SKEW_SECONDS`. **No round-trip estimation, offset filtering, or
NTP-style discipline is required or permitted**; this mechanism only has to be
comfortably inside the skew allowance, and a real clock algorithm here would be
unjustified complexity in firmware.

#### Validity — normative

```text
clock_synced  ==  (monotonic_now − synced_at_monotonic) < TIME_SYNC_MAX_AGE_SECONDS
```

`clock_synced` therefore means **"sufficiently synchronised to the Edge clock"**,
not "an SNTP transaction succeeded".

| Constant | Value | Reason |
|---|---|---|
| `TIME_SYNC_INTERVAL_SECONDS` | 300 | Edge push cadence while a device is online |
| `TIME_SYNC_MAX_AGE_SECONDS` | 1800 | tolerates five consecutive missed syncs before commands are refused |

Both are compile-time constants in `rhizo-mqtt-contract`, **not configurable**.
Drift over the full 1800 s is about 180 ms even for a poor ±100 ppm oscillator,
so the max age is not bounded by drift — it bounds how long a device may keep
accepting commands without confirming that the Edge is still there and still
agrees about the time.

#### Reconnection and rejection — normative

- After connecting or reconnecting, a device MUST NOT accept a water command
  until synchronisation is established. Until then every water command is refused
  with `reason: "clock_unsynced"` (§5.8 step 2).
- **Telemetry, sampling, and status publication continue regardless.** A device
  with no valid synchronisation is still a fully functioning sensor node; it is
  only actuation on Edge authority that is withheld.
- Losing MQTT means losing synchronisation refresh. Offline autonomy is
  unaffected because it runs on the monotonic clock
  ([ADR-015](../adr/015-device-offline-autonomy.md)) — but on reconnect, Edge
  commands stay refused until a fresh `edge.time` is applied.
- The Edge MUST NOT assume a reconnecting device is synchronised. It learns the
  device's own view from `clock_synced` in `device.status`.

---

### 5.13 `event.ack` → `events/ack` (edge → device, **never retained**)

The other half of §5.4. A device buffers history while isolated and replays it on
reconnection; this is how it learns that the replay is safely on the edge and the
buffer can be emptied.

```json
{
  "v": 1, "kind": "event.ack",
  "message_id": "018fd8c0-…",
  "device_id": "plant-node-01",
  "data": { "boot_id": "018fd6b0-…", "through_device_seq": 4413 }
}
```

| Field | Type | Required | Rules |
|---|---|---|---|
| `boot_id` | UUID | yes | the device boot this acknowledgement is addressed to |
| `through_device_seq` | integer | yes | every event at or below this `device_seq` is durably committed on the edge |

#### Cumulative, not a list — normative

`through_device_seq` is a **prefix**: it says "everything up to and including
this sequence is committed", not "these particular events are committed". A list
of `event_id`s would be the obvious alternative and is the wrong shape here.

A device buffer is bounded and a replay is built as `device_seq`-ordered slices
of the whole buffer, so every batch the edge can commit *is* a prefix — the
information a list would carry beyond a prefix does not exist. Against that, a
list grows with the backlog, so the acknowledgement for the worst outage is the
largest message, arriving exactly when the link is least able to carry it; and
it forces the device to hold a set and compute a difference, on the part with
the least RAM. A prefix is a single integer, is idempotent by construction, and
degrades to a no-op rather than to partial deletion.

#### Publishing — normative

- The edge MUST NOT publish an `event.ack` before the transaction that persists
  those events has **committed**. Acknowledging on receipt, or from a buffer, or
  optimistically before a commit that may still fail, tells the device to delete
  history the edge does not have. The order is: receive → persist → commit →
  acknowledge.
- `through_device_seq` MUST be the highest sequence such that every event at or
  below it has been committed. If a batch is committed out of order — which QoS
  1 permits — the edge acknowledges only up to the last contiguous sequence, and
  the device keeps replaying the rest. A prefix that skips a hole is a lie about
  what the edge holds.
- The edge MUST set `boot_id` to the `boot_id` of the replay being acknowledged.
- `event.ack` MUST NOT be retained (§3).
- The edge MAY acknowledge once per replay or once per batch. Neither is more
  correct; batching costs a round-trip, per-batch costs a message.
- **When the edge holds no contiguous prefix, it publishes no `event.ack` at
  all.** `device_seq` is zero-based, so 0 is a real sequence and cannot double
  as "nothing committed"; a device receiving `through_device_seq: 0` discards
  sequence 0. This is reachable whenever a device's remaining buffer starts
  above everything the edge holds — after the edge lost its replay progress, or
  where the events below the buffer were acknowledged to an earlier edge. The
  device simply replays again. Silence is the only truthful message here, and
  is a clarification of the prefix rule above rather than a change to it: no
  field is added, removed, or retyped.
- An acknowledgement is advisory in one direction only: losing one costs a
  repeated replay, so the edge is not required to retry it. It MUST NOT retry it
  by *raising* the sequence to cover events committed since — that is a new
  acknowledgement and is fine — but MUST NOT lower it.

#### Applying — normative

On receiving an `event.ack`, a device:

1. **MUST ignore it if `boot_id` is not the device's current `boot_id`.** A
   delayed acknowledgement from an earlier boot says nothing about the history
   this boot holds, and `device_seq` continues across reboots, so honouring one
   would delete events buffered since it was sent.
2. **MUST ignore it if `through_device_seq` exceeds the highest `device_seq` the
   device has ever allocated**, and MUST NOT clamp it to that highest value. A
   sequence the device never issued cannot have been committed by anyone; the
   acknowledgement is corrupt or misaddressed, and clamping would turn that into
   "delete everything". Nothing is deleted and nothing is recorded.
3. **MUST ignore it if `through_device_seq` is at or below one already applied.**
   Acknowledgement only moves forward. A duplicate is a no-op; a lower one is
   not a rewind.
4. Otherwise, discards every buffered event with `device_seq <= through_device_seq`
   and records `through_device_seq` as the highest applied.

- The discard and the record MUST be one durable step. A device that recorded
  the acknowledgement without discarding would replay events it has already
  been told about — harmless. One that discarded without recording, then
  restarted, would be unable to tell what it had already discarded. If only one
  can happen, it must be the first.
- A device MUST NOT publish anything in response to an `event.ack`. There is no
  acknowledgement of the acknowledgement; the next replay carries the same
  information, and a device that has nothing left to replay says so with the
  empty `"complete": true` batch of §8.
- A device MUST NOT discard a `history.gap` marker that has not yet been sent,
  whatever the acknowledged sequence (§5.4).

#### What is not here

This section defines the **wire mechanism** and the device's obligations. The
edge-side reconciliation that decides *when* a plant leaves `Uncertain`, and
what a gap marker means for a watering decision, is safety policy and belongs to
M6 (SAFETY-016, SAFETY-020). M3 owns durable ingest and the acknowledgement
itself: persist, commit, acknowledge. Neither milestone may implement the other's
half — an edge that acknowledged without persisting would satisfy M3's shape and
destroy M6's guarantee.

---

## 6. Deduplication — normative

Receivers MUST use `message_id` as the transport deduplication key. Because
transport markers are retained for a bounded period, every durable effect MUST
also have a stable logical identity or order key that survives marker pruning.

The edge:

1. attempts `INSERT INTO processed_messages(message_id) … ON CONFLICT DO NOTHING`
2. if 0 rows affected, the transport message is a duplicate: the transaction is rolled
   back and **no effect of any kind is applied**
3. otherwise processing continues in the same transaction

The dedup marker and the message's effects MUST share one transaction, so a
crash cannot make one durable without the other (SAFETY-001, SAFETY-010).

For `device.status`, the independent logical order is
`(boot_generation, sequence)` for normal publications plus the fixed LWT
`message_id` for the current boot. A logically old status is a no-op and MUST
NOT refresh `last_seen_at`. Its receipt still triggers the live `edge.time`
response required by §5.10; response ownership is independent of projection
acceptance.

The device deduplicates water commands on `command_id`, keeping the last 16 in
NVS with their outcomes. A repeat MUST re-publish the stored result and MUST NOT
actuate.

`(device_id, boot_id, sequence)` MUST NOT be used as a dedup key. It is used
only to detect gaps and regressions, which are recorded as diagnostic events.

**Replayed offline events** (§5.4) deduplicate durably on their device-generated
`event_id` inside the same transaction as the bounded transport marker. The device MUST
generate each `event_id` once, at buffering time, and MUST NOT regenerate it on
replay — a regenerated id would defeat deduplication and create duplicate
watering history (SAFETY-016).

Deduplication is also why a `history.gap` marker is immutable once sent (§5.4):
a marker republished with a widened range carries the `event_id` the edge has
already seen, so it is discarded as a duplicate and the extra loss it now
describes is never recorded.

---

## 7. Ordering

Receivers MUST NOT assume ordered delivery. Specifically:

- "Latest sample" means the greatest `received_at`, not the most recently
  processed message.
- A `sequence` that decreases within one `boot_id` is recorded as a
  `sequence_regression` device event but MUST NOT cause rejection.
- A `boot_id` change means the device restarted; `sequence` restarting from a
  low value is expected and MUST NOT be treated as a regression.

---

## 8. Reconnection behaviour

**Device:**

1. Configure LWT before connecting.
2. Connect with `clean_session = true`.
3. Subscribe to the seven exact topics of §3 — `config`, `policy`, `time`,
   `events/ack`, and the three `commands/*` topics. No wildcard: a device MUST
   NOT subscribe to a filter that matches a topic it publishes.
4. Publish retained `status: online`. This is what triggers the Edge to publish
   `edge.time` (§5.12); until one is applied, the device MUST refuse water
   commands with `clock_unsynced`.
5. Resume telemetry on schedule. A device MUST NOT flush a backlog of buffered
   telemetry beyond its bounded ring.
6. Republish any pending `command.result` from NVS.
7. **Replay buffered offline events** (§5.4) in `device_seq` order, in batches,
   setting `"complete": true` on the final batch. Any accumulated `history.gap`
   is sealed and takes its `device_seq` at this point (§5.4). Events MUST be
   retained until an `event.ack` (§5.13) covers them — not merely until the
   broker has acked the publish.

**Edge:**

1. Re-establish **all** subscriptions on every reconnect. Subscriptions MUST NOT
   be assumed to survive a reconnect.
2. Republish retained `config` for every known device if the broker may have
   lost retained state (detected by an absent retained status on resubscribe).
3. Do not re-issue in-flight commands; reconcile them per SAFETY-010.
4. **Hold every plant on a reconnecting device in `Uncertain`** until its event
   replay reports `"complete": true` and has been committed. Issuing a dose on
   top of an autonomous dose delivered moments ago is exactly the failure
   SAFETY-016 prevents.
5. **Publish `event.ack` (§5.13) only after the transaction persisting the
   replayed events has committed**, covering the highest contiguous
   `device_seq`. Acknowledging earlier tells a device with a bounded buffer to
   delete history the edge does not have.
6. Republish the retained `policy` if the broker may have lost retained state,
   detected the same way as for `config`.

---

## 9. Compatibility and versioning

Within `v1`:

- **Additive changes only.** New optional fields MAY be added at any time.
  Receivers MUST ignore unknown fields — `#[serde(default)]` and no
  `deny_unknown_fields` on inbound types.
- Making an optional field required, removing a field, renaming a field,
  changing a type, or changing a unit is **breaking** and requires `v2`.
- Adding an enum variant (a new `reason`, a new `kind`) is breaking for a
  receiver that matches exhaustively. Receivers MUST therefore treat unknown
  enum values as an explicit "unknown" variant and handle it conservatively —
  for safety-relevant enums, "unknown" means the safe branch (SAFETY-012).

A `v2` uses the `rhizo/v2/` namespace, allowing v1 and v2 devices to coexist on
one broker indefinitely. Full process in
[versioning-policy.md](versioning-policy.md).

---

## 10. Malformed message handling

| Condition | Receiver behaviour |
|---|---|
| Not valid JSON | quarantine, `mqtt_decode_errors_total{reason="json"}` |
| `v` != 1 | reject, `reason="version"` |
| `kind` inconsistent with topic | reject, `reason="kind_mismatch"` |
| payload `device_id` != topic `device_id` | reject, `reason="device_mismatch"` |
| invalid `device_id` grammar | reject, `reason="device_id_grammar"` |
| missing required envelope field | reject, `reason="envelope"` |
| field outside its valid range | **store the message; set that field to null**; raise `sensor_invalid` |
| one sample in a batch out of range | store the batch; null **that sample only**; raise `sensor_invalid`; keep the rest |
| unrecognised `kind` in a sample | **store the sample**, mark it advisory-only; never gate actuation on it |
| `unit` disagrees with the sample's `kind` | reject that sample, `reason="unit_mismatch"`; keep the rest of the batch |
| empty `samples` array | reject the message, `reason="empty_batch"` |
| replayed event with a duplicate `event_id` | ignored by the dedup path — the normal, expected outcome |
| `NaN` / `Infinity` | treat as out of range |

The last row is the important asymmetry: a message with one bad field is
partially usable and MUST NOT be discarded whole — the good fields may be
exactly what the safety logic needs. A message whose *identity* is inconsistent
is untrustworthy in its entirety and MUST be rejected.

Quarantine is bounded: at most 1000 stored messages, at most 10 quarantine
writes per minute per device.

---

## 11. Conformance checklist

An implementation is conformant when:

- [ ] `clean_session = true`; LWT configured before connect
- [ ] the four device subscriptions are established on **every** connect
- [ ] messages arriving on `commands/result` are ignored, never acted on
- [ ] retained on `status`, `config`, and `policy` only; never on `commands/*`, telemetry, events, actuator, or time
- [ ] QoS 1 everywhere
- [ ] envelope complete and `device_id` consistent with the topic
- [ ] `message_id` is UUIDv7 when the clock is synced
- [ ] `sequence` monotonic within a `boot_id`; `boot_id` fresh each boot
- [ ] water command validation follows §5.8 in order
- [ ] NVS persistence precedes actuation
- [ ] a `command.result` is published for every command, including rejections
- [ ] results are retried and survive a reboot
- [ ] `validate_water_command` is the only actuation gate
- [ ] unknown fields are ignored; unknown enum values map to a safe branch
- [ ] telemetry is published as **one batch per sampling cycle**, never split
- [ ] every sample carries `kind`, canonical `unit`, and `quality`
- [ ] a read failure publishes `value: null` + `quality: "fault"`, never a stale value
- [ ] an unrecognised `kind` is stored and treated as advisory only
- [ ] `capabilities` is declared in status; the edge assumes nothing undeclared
- [ ] `policy` is validated, staged, verified, then activated atomically
- [ ] a policy with `policy_version` ≤ applied is ignored
- [ ] an invalid or interrupted policy update leaves the previous policy active
- [ ] the device subscribes to seven **exact** topics and to no wildcard
- [ ] no subscription matches a topic the device publishes, `commands/result`
      included — checked as a subscription-set property, not as "ignored on
      receipt"
- [ ] buffered events keep a stable `event_id` across every replay
- [ ] the final replay batch sets `"complete": true`
- [ ] buffer overflow emits a `history.gap` event with range and count
- [ ] a `history.gap` marker is immutable once sent, and takes its `device_seq`
      at send time
- [ ] audit-tier events are never evicted to make room for telemetry
- [ ] replayed events are discarded only on `event.ack`, never on the broker's
      publish ack
- [ ] `event.ack` is published **non-retained**, QoS 1, and only after the
      persisting transaction has committed
- [ ] an `event.ack` for another `boot_id` is ignored
- [ ] an `event.ack` beyond the highest issued `device_seq` deletes nothing and
      is **not** clamped
- [ ] a duplicate `event.ack` is idempotent; a lower one does not regress
- [ ] an applied `event.ack` survives a device restart
- [ ] an unsent `history.gap` survives any acknowledgement
- [ ] `time` is published **non-retained**, QoS 1
- [ ] the Edge sends `edge.time` on every `device.status` and at least every 300 s
- [ ] an `edge.time` older than **or equal to** the last applied one is ignored
- [ ] an ignored `edge.time` does **not** refresh `synced_at_monotonic`, so a
      replayed message cannot keep `clock_synced` alive
- [ ] `clock_synced` reflects synchronisation **age**, not SNTP success
- [ ] water commands are refused with `clock_unsynced` until sync is established
- [ ] telemetry continues while unsynchronised

Fixtures for automated conformance testing live in
`test/fixtures/protocol/` and are run by both workspaces
([ADR-008](../adr/008-shared-code-simulator-and-firmware.md)).
