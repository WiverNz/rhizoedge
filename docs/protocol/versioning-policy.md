# Versioning Policy

Three things carry versions and evolve independently: the **MQTT protocol**, the
**HTTP APIs**, and the **device configuration**. This document defines what may
change within a version and what forces a new one.

The governing constraint: **devices in the field cannot be re-flashed cheaply.**
A protocol change that breaks a deployed device turns a software task into a
hardware task, in a pot, with a ladder.

---

## 0. Pre-implementation changes are not version bumps

A version exists to protect **deployed** consumers. Until a contract has been
implemented and deployed, there is nothing to protect, and bumping it would leave
a version number nobody ever spoke.

**Rule.** While a protocol version is unimplemented — no simulator, no firmware,
no edge speaking it, nothing in the field — it may be changed freely. The change
is recorded in the affected documents with a dated note explaining what moved and
why, so a later reader does not conclude the rules were bent.

Once the first implementation lands, §1 applies in full and every subsequent
change follows the additive/breaking distinction.

**This clause was used exactly once.** On 2026-08-26, before M1 began, v1 was
revised: the four `telemetry/*` topics became one batched `telemetry` topic plus
an `actuator` topic ([ADR-017](../adr/017-extensible-measurement-model.md)), and
retained `policy` plus device→edge `events` topics were added
([ADR-015](../adr/015-device-offline-autonomy.md)). No v2 was created because
nothing had ever spoken v1.

Invoking this clause after M1 completes would be a mistake, and the reviewer's
question is simply: *has anything ever spoken this version?*

**It was correctly not used on 2026-08-28.** Battery and deep-sleep device mode
([ADR-018](../adr/018-battery-and-deep-sleep-device-mode.md)) landed after M1,
M2, M3, and M4 had shipped, and every part of it fits §1's additive list: two new
`MeasurementKind` variants, optional `power` blocks in `device.config` and
`device.status`, and one new offline `reason` whose `Unknown` resolves
conservatively. Holding a command for a sleeping device is an **edge-side**
mechanism with no wire representation at all, which is what kept the change
additive — a retained command topic, or a widened TTL, would have been neither
additive nor safe. Recorded in [mqtt-v1.md](mqtt-v1.md) §9.

---

## 1. MQTT protocol versioning

### Version placement

The version appears in **both** the topic (`rhizo/v1/...`) and the payload
(`"v": 1`).

- **Topic version** enables routing without parsing, and lets a v2 edge
  subscribe to `rhizo/v1/#` and `rhizo/v2/#` simultaneously during a migration.
- **Payload version** is a consistency check. A mismatch between the two means
  something is misconfigured; the message is rejected rather than guessed at.

### Non-breaking within v1

These MAY be done at any time without a version bump:

- **Adding a `MeasurementKind` variant.** This is the designed extension point
  ([ADR-017](../adr/017-extensible-measurement-model.md)): receivers decode
  unrecognised kinds to `Unknown`, store them, and treat them as advisory. A new
  kind therefore reaches an older edge as data rather than as an error.
- **Adding a reserved actuator kind.** Same mechanism, same conservative
  handling.
- Adding a new **optional** field to any `data` object.
- Adding a new **message kind** on a new topic (the edge, subscribing to
  `rhizo/v1/devices/+/#`, will receive it and must ignore unknown kinds).

  For an **edge→device** topic this is additive in a different and stronger
  sense, because a device subscribes to exact topics
  ([mqtt-v1.md](mqtt-v1.md) §3): a device built before the topic existed does
  not subscribe to it and is never delivered it. `event.ack` §5.13 is the
  worked example — such a device simply never learns its replay was persisted,
  keeps its buffered history, and replays it again, which is the conservative
  behaviour §5.4 required all along. **This only holds while the new topic's
  absence is safe.** A new edge→device topic whose *absence* would change a
  device's behaviour for the worse is not additive, whatever this list says.
- Relaxing a validation range (widening what is accepted).
- Adding a new optional field to the envelope.

Requirement that makes this safe: **every receiver MUST ignore unknown fields.**
Inbound types use `#[serde(default)]` and never `deny_unknown_fields`.

### Breaking — requires v2

- Removing or renaming any field.
- Making an optional field required.
- Changing a field's type.
- **Changing a unit.** Mitigated structurally: units are part of field names
  (`moisture_vwc`, `_ml`, `_ms`), so a unit change is necessarily a rename, and
  therefore necessarily caught.
- Changing the meaning of an existing value.
- Tightening a validation range in a way that rejects previously valid data.
- Changing QoS or retention semantics for an existing topic.
- Changing the deduplication key.

### The enum problem

Adding a variant to an enum — a new `command.result` `reason`, a new `kind`, a
new `status` — is **breaking for a receiver that matches exhaustively**, and
non-breaking for one that does not.

Policy: receivers MUST decode unknown enum values into an explicit `Unknown`
variant rather than failing.

```rust
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RejectReason {
    ClockUnsynced,
    Expired,
    // …
    #[serde(other)]
    Unknown,
}
```

And — this is the part that matters — **for safety-relevant enums, `Unknown`
MUST take the conservative branch.** An unrecognised leak state is not "no
leak"; it is a lockout (SAFETY-012). A receiver that treated `Unknown` as
permissive would convert a forward-compatibility mechanism into a safety hole.

Given that rule, adding a variant becomes non-breaking, which is why it is
permitted within v1.

### Migration to v2

```text
1. Specify v2 in docs/protocol/mqtt-v2.md; keep mqtt-v1.md unchanged.
2. Edge subscribes to BOTH rhizo/v1/# and rhizo/v2/#.
3. Edge publishes commands/config on the version the device announced
   (protocol_version in device.status).
4. Devices are re-flashed to v2 individually, at any pace.
5. v1 support is removed only when no device has reported v1 for 90 days,
   verified against the device registry — not from memory.
```

v1 and v2 devices coexist on one broker indefinitely. There is no flag day.

### Deprecation

A field deprecated within v1 is marked in `mqtt-v1.md`, still emitted, still
accepted, and removed only in v2. Deprecation is documentation; removal is a
version bump.

---

## 2. HTTP API versioning

`/api/v1/...` in the path, for both the Edge and Cloud APIs.

**Non-breaking:** adding endpoints, adding optional request fields, adding
response fields, widening accepted values.

**Breaking:** removing or renaming a field, changing a type, changing status-code
semantics, making an optional request field required.

Clients MUST ignore unknown response fields.

HTTP versioning is materially cheaper than MQTT versioning: the UI ships
alongside the edge and both are updated together. The Cloud API is the exception —
edges in the field may lag — so the **cloud ingestion endpoint is treated with
MQTT-level caution** and must accept older event shapes indefinitely. An event
shape that the cloud can no longer project is still stored in `synced_events`
and reported as `rejected` for projection only, so history is never lost
([ADR-005](../adr/005-cloud-event-model-and-idempotency.md)).

---

## 3. Device configuration versioning

`config_version` is a monotonically increasing `u32` owned by the edge.

Rules:

- The edge increments it on every config change.
- A device MUST ignore a config whose `config_version` is less than or equal to
  the applied version. This defends against retained-message replay after a
  rollback, where the broker might deliver an older retained config.
- A device MUST ignore config fields it does not recognise, which is what makes
  adding a config field non-breaking across mixed firmware versions.
- The device echoes `applied_config_version` in `device.status`; the edge
  surfaces drift.

`config_version` is not a protocol version. It orders configurations; it does
not describe their shape.

---

## 4. Database schema versioning

Both SQLite and PostgreSQL use forward-only numbered migrations
(`sqlx migrate`).

- Migrations are **never edited after being applied anywhere**, including a
  developer machine. A mistake is corrected by a new migration.
- Migrations must be **additive where possible**. Dropping a column requires a
  deliberate two-step: stop writing it in release N, drop it in release N+1.
- The edge takes an automatic backup when the schema version changes
  ([ADR-004](../adr/004-sqlite-edge-persistence-model.md)).
- Migration failure at startup is Fatal: the process exits rather than serving
  with an unknown schema ([ADR-014](../adr/014-failure-and-retry-policy.md)).

---

## 5. Firmware versioning

Semantic versioning: `MAJOR.MINOR.PATCH`, reported in `device.status` as
`firmware_version`, alongside `protocol_version`.

The two are independent: firmware 0.4.2 may speak protocol v1. The edge routes
on `protocol_version`, never on `firmware_version`.

**Changing a hard safety limit is always a MAJOR bump**, even if nothing else
changes, because it alters the device's safety contract and every deployed unit
must be accounted for.

---

## 6. Compatibility testing

The protocol fixture corpus in `test/fixtures/protocol/` is the mechanism that
keeps this policy honest:

- Fixtures are **append-only**. A fixture committed for v1 must decode
  successfully for as long as v1 is supported. Deleting or editing one is a
  breaking change and is caught in review.
- `valid/` fixtures must decode and re-encode to an equivalent value.
- `invalid/` fixtures must be rejected with the documented error variant.
- Both workspaces run the same corpus
  ([ADR-008](../adr/008-shared-code-simulator-and-firmware.md)), so a change
  that breaks the firmware's view of the protocol fails the host build too.

When a field is added, a new fixture exercising it is added. When a receiver is
changed, the whole corpus must still pass. That is the entire compatibility
guarantee, expressed as tests rather than as intentions.

---

## 7. Summary table

| Change | MQTT | HTTP | Config | DB |
|---|---|---|---|---|
| Add optional field | v1 ok | v1 ok | ok | additive migration |
| Add endpoint / topic / kind | v1 ok | v1 ok | — | — |
| Add enum variant | v1 ok, if `Unknown` is conservative | v1 ok | ok | ok |
| Widen a range | v1 ok | v1 ok | ok | ok |
| Rename field | **v2** | **v2** | breaking | two-step migration |
| Change type | **v2** | **v2** | breaking | two-step migration |
| Change unit | **v2** (forced rename) | **v2** | breaking | two-step migration |
| Make optional required | **v2** | **v2** | breaking | — |
| Tighten a range | **v2** | **v2** | breaking | — |
| Change QoS / retention | **v2** | — | — | — |
| Change dedup key | **v2** | — | — | — |
| Change a hard safety limit | — | — | — | firmware MAJOR |
