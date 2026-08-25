# ADR-012 — Device identity and provisioning

## Status

Accepted — 2026-08-25. Grammar in M1; broker ACLs in M0; device-side flow in M9;
multi-device onboarding in M13.

## Context

`device_id` appears in every topic, every payload, every database row, and every
ACL rule. It is effectively permanent: changing it orphans a device's history.
It is also a security boundary — a device that can name itself arbitrarily can
publish into another device's topic subtree.

Decisions needed: the grammar, how an id is assigned, how credentials are
issued, and how a device is registered with the edge.

## Decision

### Grammar

```text
^[a-z0-9]([a-z0-9-]{1,30})[a-z0-9]$      3–32 characters
```

Lowercase alphanumerics and hyphens; must start and end alphanumeric.

The exclusions matter more than the inclusions. Barring `+`, `#`, `/`, and
whitespace is what prevents topic injection: a device calling itself `x/#` or
`+` would otherwise break out of its subtree and match wildcard subscriptions.
Lowercase-only removes any question about case sensitivity in topic matching
versus database collation — two systems that disagree about whether `Plant-01`
and `plant-01` are the same device is a bug waiting to happen.

Validated in `DeviceId::parse` in `rhizo-mqtt-contract`. The type has no public
constructor that skips validation, so an invalid id cannot exist in a running
system.

### Assignment: derived from MAC, overridable

Default at first boot:

```text
plant-node-<last 3 bytes of Wi-Fi MAC, lowercase hex>
e.g. plant-node-a4c1f9
```

- Unique without a central allocator, which matters because provisioning happens
  on a bench with no network to a registry.
- Stable across reflashes — the same board keeps its history.
- Not sequential, so ids do not imply an ordering that does not exist.

The derived id is written to NVS on first boot and read from NVS thereafter. An
operator may override it during provisioning (e.g. `monstera-window`), and the
NVS value always wins. Once set, it is not changed automatically — a device
whose id changed would appear as a new device with no history.

### Credentials: one MQTT account per device

```text
username = device_id
password = 32 random bytes, base64, generated at provisioning
```

Mosquitto ACL:

```text
pattern readwrite rhizo/v1/devices/%u/#
```

`%u` substitutes the authenticated username, so a device authenticated as
`plant-node-a4c1f9` can only touch its own subtree. This single line is what
makes device identity a real boundary rather than a convention, and it is why
the topic hierarchy puts `device_id` before the message kind
([ADR-002](002-mqtt-topic-versioning-and-qos.md)).

The edge uses a separate account with broader rights:

```text
user rhizo-edge
topic readwrite rhizo/v1/#
```

Anonymous access is disabled from M0.

**Password rotation** is a manual procedure in V1: regenerate, update the
Mosquitto password file, reflash or re-provision the device. Documented in M13;
not automated.

### Provisioning flow (V1: bench provisioning)

```text
1. Flash firmware.
2. Provide Wi-Fi SSID/PSK, MQTT host, and credentials via one of:
     a. a `.env`-style file consumed at build time (dev), or
     b. a serial provisioning command writing directly to NVS (preferred)
3. Device boots, reads NVS, derives or reads device_id, connects.
4. Device publishes retained status → the edge sees an unknown device.
5. Edge auto-registers it in `devices` with status 'unknown' and NO plant
   attached.
6. Operator names it and attaches a plant through the UI/API.
```

**Step 5 is the important one: auto-registration creates a device row, never a
plant.** A device with no plant produces telemetry and nothing else — it cannot
be watered because there is no plant, no profile, and no `auto_watering_enabled`
to be true. A new device that appeared on the network cannot cause actuation.
That is SAFETY-012 applied to onboarding.

Option (b) is preferred over (a) because build-time credentials mean the binary
contains secrets and every device needs its own build. A serial provisioning
command means one firmware image for all devices.

### Boot identity

Each boot generates a fresh `boot_id` (UUIDv4 from the hardware RNG; v7 is not
available before SNTP sync). Together with `sequence` it distinguishes messages
from different boot sessions, which a bare sequence number cannot do after a
power cut. See [ADR-002](002-mqtt-topic-versioning-and-qos.md).

`boot_id` is *not* identity — it changes every boot. `device_id` is identity and
is stable.

### Device ↔ plant relationship

V1: one device, one plant, but the schema models it as
`plants.device_id → devices.device_id`, a many-to-one, so one device can serve
several plants later without migration. `measurements.measurement_point`
similarly defaults to `'default'` so multi-probe and multi-depth deployments do
not need a schema change ([ADR-004](004-sqlite-edge-persistence-model.md)).

Neither capability is *implemented* in V1 — only the shape is reserved, which
costs one column and one default.

### What identity is not, in V1

- **Not authenticated cryptographically.** A password is a shared secret; anyone
  who reads it can impersonate the device. Adequate for a home LAN, inadequate
  for anything else.
- **Not attested.** There is no proof the firmware is genuine; signed firmware
  is deferred.
- **Not revocable at scale.** Revocation means editing the Mosquitto password
  file and reloading.

These are V1 limitations recorded deliberately. The upgrade path — per-device
X.509 certificates with TLS mutual auth — is an M13/M14 topic, and the
`device_id`-as-username structure maps directly onto a certificate CN, so the
ACL model survives that transition unchanged.

## Alternatives considered

**UUID as `device_id`.** Rejected: unreadable in a topic, in a log, and on a
label stuck to a pot. Human-legible ids matter enormously during hardware
bring-up.

**Sequential ids assigned by the edge** (`device-001`). Rejected: requires the
edge to be reachable during provisioning, and creates an allocation authority
that must be consistent across reinstalls.

**Full MAC as the id** (`plant-node-a4c1f9e2b301`). Rejected: 18 characters of
mostly-redundant hex; three bytes give 16.7 million values, ample for any
plausible fleet, with collisions detectable at registration.

**Shared MQTT credentials for all devices.** Rejected: it makes the ACL
meaningless, since every device could publish as any other. The per-device
account costs one line in a password file.

**Registering devices only after operator approval** (rejecting telemetry from
unknown devices). Rejected: it discards data during the window before someone
notices, and the auto-registered-but-plantless state already prevents actuation.

**Cloud-issued identity.** Rejected: the cloud is optional
([ADR-003](003-edge-first-ownership-model.md)); identity cannot depend on it.

## Consequences

Positive:

- Device ids are readable in topics, logs, the UI, and on a physical label.
- The `%u` ACL makes the identity boundary real with one configuration line.
- A newly appearing device cannot water anything.
- The schema already accommodates multi-plant and multi-probe devices.

Negative, accepted:

- Per-device credentials mean per-device provisioning; onboarding twenty devices
  is twenty operations. M13 adds tooling; V1 has one device.
- MAC-derived ids can collide in principle (3 bytes). Detected at registration
  and resolved by an operator override.
- No cryptographic identity in V1, so the LAN is the trust boundary.

## Risks

- **An operator changes `device_id` after deployment**, orphaning history.
  *Mitigation:* documented as a one-way decision; the API offers a device
  *rename* (display name) that is distinct from the id, so the common need does
  not require the dangerous operation.
- **Credentials leaking via a build-time config** if option (a) is used in
  production. *Mitigation:* option (b) is the documented default for anything
  beyond a bench, and `.env` is gitignored.
- **ACL misconfiguration** silently granting broad access. *Mitigation:* M2-012
  is an integration test asserting a device account cannot publish to another
  device's topic.

## Follow-up

- M0-008 configures Mosquitto authentication and ACLs.
- M1-002 implements `DeviceId` and its grammar tests.
- M2-012 tests ACL enforcement.
- M4-003 implements auto-registration without plant attachment.
- M9-004 implements NVS identity and serial provisioning.
- M13-002 covers multi-device onboarding tooling.
