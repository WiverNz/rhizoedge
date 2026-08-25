# ADR-002 — MQTT topic hierarchy, versioning, QoS, and retention

## Status

Accepted — 2026-08-25. Specified in [docs/protocol/mqtt-v1.md](../protocol/mqtt-v1.md), implemented in M1.

## Context

MQTT is the seam between devices and the control plane. It is also the seam that
must survive the transition from simulator to ESP32 to (eventually) a LoRaWAN
gateway. Getting the topic grammar and delivery semantics wrong is expensive to
correct later because devices in the field cannot be re-flashed cheaply.

The decisions needed: namespace and version placement, per-topic QoS, which
messages are retained, session persistence, envelope shape, and how duplicates
are detected.

## Decision

### Topic grammar

```text
rhizo/v1/devices/{device_id}/telemetry/{soil|weight|tank|pump}
rhizo/v1/devices/{device_id}/status
rhizo/v1/devices/{device_id}/config
rhizo/v1/devices/{device_id}/commands/{water|tare|calibrate}
rhizo/v1/devices/{device_id}/commands/result
```

- **Version in the topic, not only the payload.** `rhizo/v1/...` lets a v2 edge
  subscribe to both `rhizo/v1/#` and `rhizo/v2/#` during a migration, and lets a
  v1-only device coexist with v2 devices on the same broker indefinitely. A
  payload-only version would force every consumer to parse before it can route.
- **`device_id` before the message kind.** This orders the tree by the thing that
  has an owner and a lifecycle. It makes per-device ACLs in Mosquitto a simple
  prefix rule (`topic readwrite rhizo/v1/devices/%u/#`), which is what makes
  per-device credentials practical in [ADR-012](012-device-identity-and-provisioning.md).
- **`commands/result` is a single topic**, not one per command type. Results are
  correlated by `command_id`, not by topic, so the edge subscribes once.
- **Version is also carried in the payload** (`"v": 1`) as a consistency check.
  A mismatch between topic version and payload version is a rejected message —
  it means something is misconfigured and guessing is worse than refusing.

### Device ID grammar

```text
^[a-z0-9]([a-z0-9-]{1,30})[a-z0-9]$        3–32 characters
```

Lowercase alphanumerics and hyphens only. This excludes `+`, `#`, `/`, and
whitespace, which is what prevents a device id from breaking out of its topic
subtree — a device that could name itself `x/#` would be a topic-injection
vulnerability.

### QoS per topic

| Topic | QoS | Reasoning |
|---|---|---|
| `telemetry/*` | 1 | loss is tolerable but should be rare; QoS 2 costs a round trip for no benefit given consumers are idempotent |
| `status` | 1 | must not be lost — it drives online/offline state |
| `config` | 1 | must not be lost — it is desired state |
| `commands/*` | 1 | must not be lost; duplicates handled by `command_id` |
| `commands/result` | 1 | ledger data; the device retries until acked |

**QoS 1 everywhere, never QoS 2.** QoS 2's exactly-once guarantee is per-hop and
per-session; it does not survive a device reboot or an edge restart, which are
exactly the cases we care about. Real exactly-once has to be built at the
application layer regardless, so we build it there (`message_id` + the
persist-and-dedup transaction) and take QoS 1's lower overhead.

### Retention

| Topic | Retained | Reasoning |
|---|---|---|
| `status` | **yes** | a subscriber must learn device state without waiting for the next heartbeat |
| `config` | **yes** | a device booting days later must receive current desired state with no edge-side liveness tracking |
| `telemetry/*` | **no** | a retained sample would be served as if current to every new subscriber — an actively dangerous stale reading |
| `commands/*` | **no** | a retained command would be re-delivered on every reconnect forever; the exact opposite of SAFETY-002 |
| `commands/result` | **no** | results are events, not state |

Retaining a command topic is the single most damaging mistake available in this
protocol, so it is called out explicitly here and asserted by a test (M2-010).

### Session persistence

`clean_session = true` (MQTT 3.1.1) / `Clean Start = true` with
`Session Expiry = 0` (MQTT 5) on **both** devices and the edge.

Rationale: a persistent session makes the broker queue commands for an offline
device and deliver the backlog on reconnect. That is precisely the scenario
SAFETY-002 exists to prevent. Retained status and config are unaffected by
session cleanliness, so nothing of value is lost.

### MQTT protocol level

MQTT 3.1.1 as the baseline. Mosquitto 2.x supports MQTT 5, and `rumqttc`
supports v5 via a separate module, but the ESP-IDF MQTT client's v5 support adds
configuration surface for no feature this design needs. Revisit only if
per-message expiry or reason codes become load-bearing; the application-level
`expires_at` already covers our TTL need in a way that works identically on both
protocol levels.

### Envelope

Every payload is an envelope with a typed `data` field:

```json
{
  "v": 1,
  "kind": "telemetry.soil",
  "message_id": "018fd6c4-7b4a-7c31-9e2a-3f5b1d8c6a20",
  "device_id": "plant-node-01",
  "boot_id": "018fd6b0-1122-7000-8000-aabbccddeeff",
  "sequence": 81273,
  "device_time_ms": 1756121400000,
  "clock_synced": true,
  "data": { "moisture_vwc": 31.7, "temperature_c": 21.4, "ec_us_cm": 840 }
}
```

- **`message_id` is a UUIDv7** — time-ordered, so it sorts usefully as a
  primary key and gives a cheap sanity check on device clocks, unlike UUIDv4.
- **`boot_id`** is generated fresh at every device boot. Together with
  `sequence` it distinguishes "sequence 5 from this boot" from "sequence 5 from
  the boot before the power cut", which a bare sequence number cannot do.
- **`device_id` is duplicated** in the payload even though the topic carries it.
  A mismatch means misrouting or a spoofing attempt, and the message is rejected.
- **`clock_synced`** tells the edge whether `device_time_ms` means anything and
  drives the SAFETY-002 refusal path in [ADR-013](013-clock-and-time-semantics.md).

### Deduplication contract

The consumer deduplicates on `message_id` alone. `(device_id, boot_id, sequence)`
is used only for gap and regression detection, never for dedup, because a device
that reboots mid-second could legitimately reuse a sequence value while
`message_id` remains globally unique.

### Compatibility policy

Within `v1`:

- **Additive changes only.** New optional fields may be added; consumers ignore
  unknown fields (`#[serde(default)]`, no `deny_unknown_fields` on inbound
  types).
- Removing a field, renaming a field, changing a unit, or changing a type is a
  **v2** change requiring a new topic namespace.
- Units are part of the field name (`moisture_vwc`, `temperature_c`,
  `ec_us_cm`, `_ml`, `_ms`, `_percent`) so that a unit change is necessarily a
  rename, and therefore necessarily breaking, and therefore caught.

## Alternatives considered

**QoS 2 for commands.** Rejected: its guarantee does not span reboots, and it
does not remove the need for `command_id` idempotency. Extra round trips, no
extra safety.

**Persistent sessions with per-message expiry (MQTT 5).** Rejected for V1:
it makes broker configuration part of the safety argument. Application-level
`expires_at` checked on the device is auditable, works on any broker, and is
identical in the simulator.

**Version in the payload only** (`rhizo/devices/...` + `"v": 1`). Rejected:
forces parse-before-route and makes mixed-version fleets harder.

**Flat topics** (`rhizo/v1/telemetry/soil/{device_id}`). Rejected: breaks the
one-prefix-per-device ACL rule that makes per-device credentials cheap.

**Protobuf or CBOR payloads.** Rejected for V1: JSON is debuggable with
`mosquitto_sub`, which matters enormously while bringing up unfamiliar hardware.
Revisit for the field/LoRaWAN version where payload size is a hard constraint —
the envelope is deliberately shaped so a binary encoding can be swapped in
behind the same types (M14).

## Consequences

Positive:

- One subscription pattern (`rhizo/v1/devices/+/#`) covers everything the edge
  needs, and per-device ACLs are a one-line Mosquitto rule.
- The "no retained commands, no persistent sessions" pair makes the stale-command
  failure mode structurally impossible rather than merely handled.
- JSON keeps hardware bring-up debuggable with standard CLI tools.

Negative, accepted:

- JSON envelopes are ~250–350 bytes per telemetry message. At 300-second
  intervals this is irrelevant on Wi-Fi and unacceptable on LoRaWAN — a known
  M14 problem, recorded rather than solved.
- No broker-side buffering means telemetry published during a broker restart is
  lost. Accepted: telemetry is a sample stream, not a ledger.
- `boot_id` + `sequence` + `message_id` is three identity fields, which is more
  envelope overhead than a naive design. Each earns its place (see above).

## Risks

- **A future contributor retains a command topic** while debugging and forgets.
  *Mitigation:* an integration test asserts the broker holds no retained message
  on any `commands/*` topic after a scenario completes (issue M2-010).
- **Device id spoofing on a shared broker.** A device authenticating as `A` could
  publish as `B` if ACLs were misconfigured. *Mitigation:* Mosquitto ACL pattern
  `%u` ties the topic prefix to the authenticated username, plus the edge's
  topic/payload `device_id` consistency check. Issue M0-008.

## Follow-up

- [docs/protocol/mqtt-v1.md](../protocol/mqtt-v1.md) — the normative specification.
- [docs/protocol/versioning-policy.md](../protocol/versioning-policy.md) — the v1→v2 process.
- M1-001…M1-009 implement the contract crate.
- M2-010 asserts no retained command topics.
- M0-008 configures Mosquitto ACLs.
