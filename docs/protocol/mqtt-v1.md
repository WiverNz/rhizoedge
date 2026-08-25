# Rhizo MQTT Protocol — v1

**Status:** normative specification. This document is the contract. It is
specific enough that the Device Simulator and the ESP32 firmware can be written
independently and interoperate.

**Conformance language:** MUST / MUST NOT / SHOULD / MAY as in RFC 2119.

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
| `rhizo/v1/devices/{id}/telemetry/soil` | device | edge | 1 | no |
| `rhizo/v1/devices/{id}/telemetry/weight` | device | edge | 1 | no |
| `rhizo/v1/devices/{id}/telemetry/tank` | device | edge | 1 | no |
| `rhizo/v1/devices/{id}/telemetry/pump` | device | edge | 1 | no |
| `rhizo/v1/devices/{id}/status` | device | edge | 1 | **yes** |
| `rhizo/v1/devices/{id}/config` | edge | device | 1 | **yes** |
| `rhizo/v1/devices/{id}/commands/water` | edge | device | 1 | no |
| `rhizo/v1/devices/{id}/commands/tare` | edge | device | 1 | no |
| `rhizo/v1/devices/{id}/commands/calibrate` | edge | device | 1 | no |
| `rhizo/v1/devices/{id}/commands/result` | device | edge | 1 | no |

### Retention rules — normative

- `status` and `config` MUST be published with the retain flag set.
- **All other topics MUST NOT be published with the retain flag set.**
  Publishing a retained message on any `commands/*` topic is a protocol
  violation: the broker would redeliver it on every reconnect indefinitely,
  causing repeated watering. Publishing retained telemetry is also a violation:
  it would be served to new subscribers as though current.

### Subscriptions

- Edge subscribes to `rhizo/v1/devices/+/#`.
- Device subscribes to `rhizo/v1/devices/{own_id}/config` and
  `rhizo/v1/devices/{own_id}/commands/+` — and MUST NOT subscribe to
  `commands/result`, which it publishes.

### QoS

QoS 1 for everything. QoS 0 and QoS 2 MUST NOT be used. Consumers MUST be
idempotent (§6).

---

## 4. Message envelope

Every payload on every topic is a JSON object with this envelope:

```json
{
  "v": 1,
  "kind": "telemetry.soil",
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
| `message_id` | UUID string | yes | **UUIDv7**; globally unique; the deduplication key |
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
- Units are encoded in field names (`_vwc`, `_c`, `_us_cm`, `_ml`, `_g`,
  `_percent`, `_ms`, `_seconds`). Changing a unit therefore requires renaming a
  field, which is a breaking change (§9).
- Absent optional fields MAY be omitted or sent as `null`; the two are
  equivalent.

---

## 5. Message kinds and payloads

### 5.1 `telemetry.soil` → `telemetry/soil`

```json
{
  "v": 1, "kind": "telemetry.soil",
  "message_id": "018fd6c4-7b4a-7c31-9e2a-3f5b1d8c6a20",
  "device_id": "plant-node-01",
  "boot_id": "018fd6b0-1122-4000-8000-aabbccddeeff",
  "sequence": 81273,
  "device_time_ms": 1756121400000,
  "clock_synced": true,
  "data": {
    "measurement_point": "default",
    "moisture_vwc": 31.7,
    "temperature_c": 21.4,
    "ec_us_cm": 840
  }
}
```

| Field | Type | Required | Valid range |
|---|---|---|---|
| `measurement_point` | string | no, default `"default"` | device_id grammar |
| `moisture_vwc` | float \| null | yes | 0.0 – 100.0 |
| `temperature_c` | float \| null | no | −20.0 – 80.0 |
| `ec_us_cm` | integer \| null | no | 0 – 20000 |

`measurement_point` supports multi-probe and multi-depth devices without a
protocol change. V1 devices send `"default"` or omit it.

### 5.2 `telemetry.weight` → `telemetry/weight`

```json
"data": { "pot_weight_g": 5312.4, "stable": true }
```

| Field | Type | Required | Range |
|---|---|---|---|
| `pot_weight_g` | float \| null | yes | 0.0 – 100000.0 |
| `stable` | boolean | no, default `true` | `false` while the reading is settling |

### 5.3 `telemetry.tank` → `telemetry/tank`

```json
"data": { "tank_level_percent": 72.0, "leak_detected": false }
```

| Field | Type | Required | Range |
|---|---|---|---|
| `tank_level_percent` | float \| null | yes | 0.0 – 100.0 |
| `leak_detected` | boolean \| null | yes | — |

`leak_detected: null` means the sensor is absent or faulty. The edge MUST treat
`null` as `Unknown`, which is a lockout (SAFETY-012) — **not** as `false`.

Leak state changes MUST be published immediately, not deferred to the next
telemetry interval.

### 5.4 `telemetry.pump` → `telemetry/pump`

Published when the pump state changes, not periodically.

```json
"data": {
  "pump_active": false,
  "last_run_ms": 6120,
  "delivered_today_ml": 90.0,
  "faulted": false
}
```

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
    "status": "online",
    "firmware_version": "0.1.0",
    "protocol_version": 1,
    "applied_config_version": 7,
    "uptime_ms": 912344,
    "free_heap_bytes": 143216,
    "rssi_dbm": -58,
    "sensors": {
      "soil":  { "present": true,  "healthy": true,  "errors": 0 },
      "weight":{ "present": false, "healthy": true,  "errors": 0 },
      "tank":  { "present": true,  "healthy": true,  "errors": 0 },
      "leak":  { "present": true,  "healthy": true,  "errors": 0 }
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

`status` MUST be one of `"online"` or `"offline"`.

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
  "data": { "status": "offline", "reason": "connection_lost" }
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
7. tank_level_percent == null
        → reject(tank_unknown)          [SAFETY-012]
8. tank_level_percent <= tank.min_percent
        → reject(tank_low)              [SAFETY-004]
9. pump faulted or pump.enabled == false
        → reject(pump_unavailable)
10. requested_ml > FIRMWARE_MAX_ML_PER_RUN
        → CLAMP to FIRMWARE_MAX_ML_PER_RUN, set clamped = true   [SAFETY-007]
11. delivered_today_ml + effective_ml > FIRMWARE_MAX_DAILY_ML
        → reject(over_daily_max)        [SAFETY-007]
12. run_ms = effective_ml / pump.ml_per_second * 1000
    if run_ms > FIRMWARE_MAX_RUN_SECONDS * 1000
        → CLAMP run_ms, recompute effective_ml, set clamped = true [SAFETY-007]
13. persist (command_id, started_at, requested_ml) to NVS   [SAFETY-011]
14. actuate
```

Steps 10 and 12 clamp; every other failure rejects. Step 13 **MUST** complete
before step 14, so an interrupted dose is detectable on the next boot.

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
measure the delivered volume. It is subject to steps 1–9 and 12 above, and its
delivered volume counts toward `FIRMWARE_MAX_DAILY_ML`.

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
    "delivered_today_ml": 130.0
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

## 6. Deduplication — normative

**Receivers MUST deduplicate on `message_id` alone.**

The edge:

1. attempts `INSERT INTO processed_messages(message_id) … ON CONFLICT DO NOTHING`
2. if 0 rows affected, the message is a duplicate: the transaction is rolled
   back and **no effect of any kind is applied**
3. otherwise processing continues in the same transaction

The dedup marker and the message's effects MUST share one transaction, so a
crash cannot make one durable without the other (SAFETY-001, SAFETY-010).

The device deduplicates water commands on `command_id`, keeping the last 16 in
NVS with their outcomes. A repeat MUST re-publish the stored result and MUST NOT
actuate.

`(device_id, boot_id, sequence)` MUST NOT be used as a dedup key. It is used
only to detect gaps and regressions, which are recorded as diagnostic events.

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
3. Subscribe to `config` and `commands/+`.
4. Publish retained `status: online`.
5. Resume telemetry on schedule. A device MUST NOT flush a backlog of buffered
   telemetry beyond its 16-sample ring.
6. Republish any pending `command.result` from NVS.

**Edge:**

1. Re-establish **all** subscriptions on every reconnect. Subscriptions MUST NOT
   be assumed to survive a reconnect.
2. Republish retained `config` for every known device if the broker may have
   lost retained state (detected by an absent retained status on resubscribe).
3. Do not re-issue in-flight commands; reconcile them per SAFETY-010.

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
- [ ] retained on `status` and `config` only; never on `commands/*` or telemetry
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

Fixtures for automated conformance testing live in
`test/fixtures/protocol/` and are run by both workspaces
([ADR-008](../adr/008-shared-code-simulator-and-firmware.md)).
