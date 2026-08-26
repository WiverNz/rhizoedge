# ADR-011 — Configuration and secrets model

## Status

Accepted — 2026-08-25. Baseline in M0, layers completed through M6.

**Revised 2026-08-26.** Layer L4 splits: a `PlantProfile` is now a *template*,
and the authoritative per-plant configuration is the binding/policy model of
[ADR-016](016-plant-binding-and-policy-model.md). A new layer **L3b** carries the
**offline policy** ([ADR-015](015-device-offline-autonomy.md)) — edge-authored,
device-applied, and the only configuration a device may *act* on alone.

## Context

Configuration is where safety guarantees are most easily lost. A system that
lets a remote caller raise a pump's maximum dose has no maximum dose. At the
same time, a system where every tuning value requires a reflash is unusable.

The layering, ownership, and change matrix are specified normatively in
[configuration-model.md](../architecture/configuration-model.md). This ADR
records *why* those choices were made and what was rejected.

## Decision

### Five layers, lower wins

```text
L5  plant instance        operator, UI/API
L4  plant bindings +      operator, UI/API
    measurement policies  (PlantProfile is now only a template — ADR-016)
L3b OFFLINE POLICY        edge-owned, device-applied, versioned, opt-in
                          the only config a device may ACT on alone (ADR-015)
L3  device runtime        edge-owned, retained MQTT, versioned
L2  edge instance         deployer, TOML + env
L1  firmware hard limits  compile-time, unchangeable at runtime
```

**L3b is the layer that changed the shape of this model.** Every other layer
*tunes* something; L3b *authorises* a device to act without supervision. Its
rules are correspondingly stricter:

- validated by the Edge before publication, and re-validated by the device
  against its own declared capabilities and compile-time hard limits;
- staged and activated atomically, so a bad or interrupted policy never replaces
  the last known good one (SAFETY-019);
- `enabled` defaults to `false` — autonomy is opted into per plant by a human;
- a policy requesting more than `FIRMWARE_MAX_ML_PER_RUN` is **rejected**, not
  clamped, exactly like an over-ambitious profile value.

**L4 is now bindings and policies, not one profile.** A plant's configuration is
its `SensorBinding[]`, optional `ActuatorBinding`, `MeasurementPolicy[]`,
`AlertPolicy`, and `AutomationPolicy`. `PlantProfile` survives as a named
template that seeds those values at creation and does **not** retroactively
rewrite existing plants — silently changing the irrigation rules of twelve plants
is not a feature. See [ADR-016](016-plant-binding-and-policy-model.md).

The single rule that carries the safety weight: **a lower layer always wins, and
L1 is unreachable from the network.** L4 may ask for 120 ml; if L1 caps a run at
80 ml, the device delivers 80 ml and reports `clamped: true`.

### L1 is compile-time, and lives in the shared contract crate

`FIRMWARE_MAX_RUN_SECONDS`, `FIRMWARE_MAX_ML_PER_RUN`, `FIRMWARE_MAX_DAILY_ML`
are `const` in `rhizo-mqtt-contract`. Consequences, all deliberate:

- No message, topic, API call, or cloud field can change them (SAFETY-007).
- The simulator enforces byte-identical values
  ([ADR-008](008-shared-code-simulator-and-firmware.md)).
- Changing them requires reflashing every device — a real operational cost,
  accepted as the price of a limit that cannot be talked out of.

They are *reported* in the device status message for observability. Reporting is
strictly one-way.

### L2 is a TOML file plus environment overrides

Layered: built-in defaults → `edge.toml` → `RHIZO_EDGE__*` env vars → a small
set of CLI flags. Nested keys use `__` (`RHIZO_EDGE__MQTT__BROKER_URL`).

Implemented with the `figment` or `config` crate — the choice is made in M0-005
based on which gives cleaner error messages for a malformed key, since a
misconfigured edge that starts with silently wrong values is far worse than one
that refuses to start.

**Fail fast on invalid configuration.** The edge validates the whole
configuration at startup and exits non-zero with a specific message rather than
starting with a default substituted for something the operator got wrong.

### L3 is edge-owned desired state, delivered retained

The edge is the only writer of device config. It publishes retained on
`rhizo/v1/devices/{id}/config` with a monotonic `config_version`; the device
persists it in NVS and echoes `applied_config_version` in status.

Retention is what makes this work without liveness tracking: a device that boots
three days later receives current desired state with no edge-side bookkeeping
about who is awake.

Drift (desired ≠ applied for longer than two telemetry intervals) raises a
`config_drift` device event and is visible in the API. Silent drift is the
failure mode this design exists to prevent.

### L4 validation rejects rather than clamps

A profile with `dose_ml = 200` against `FIRMWARE_MAX_ML_PER_RUN = 80` is
**rejected at write time with an explanatory error**, not silently clamped.

The reasoning: silent clamping means the operator believes something false about
their system, and discovers it during an incident. An error at the moment of
editing teaches them the real limit while they are paying attention.

Other rejected-at-write conditions: `target_min >= target_max`,
`dose_ml * max_doses_per_cycle > max_daily_ml`, non-positive intervals.

### L5 defaults to off

`auto_watering_enabled` is `false` for every newly created plant. Automation is
opted into explicitly, per plant, by a human. This is SAFETY-012 applied to
configuration: the default for an unconfigured system is to do nothing.

### Cloud pushes no configuration in V1

Stated as a decision rather than an omission. Cloud-pushed desired state
introduces split-brain between local edits and remote state, and creates a
network path that alters device behaviour — which requires an authentication
story V1 does not have. Deferred to M14 planning
([ADR-003](003-edge-first-ownership-model.md)).

### Secrets

V1 threat model: trusted home LAN, one operator, no multi-tenancy, no Internet
exposure.

| Secret | Development | Home deployment |
|---|---|---|
| MQTT password (edge) | `.env`, gitignored | systemd `EnvironmentFile=`, mode 0600 |
| MQTT per-device credentials | generated `deploy/mosquitto/passwd` | same, one per device at provisioning |
| PostgreSQL password | `.env` / compose env | environment variable |
| Cloud API token | unused in V1 | reserved for M14 |

Rules:

1. No secret is committed. `.env` and `deploy/mosquitto/passwd` are gitignored;
   `.env.example` documents the shape with placeholders.
2. **No secret in the TOML file.** A config file gets pasted into bug reports;
   an environment variable does not.
3. No secret is logged — enforced by the redacting `Debug` impl
   ([ADR-010](010-observability-strategy.md)).
4. Anonymous MQTT access is disabled from M0. Mosquitto ACLs restrict each
   device to its own topic subtree via the `%u` pattern
   ([ADR-012](012-device-identity-and-provisioning.md)).

**What V1 does not have, stated plainly:** no TLS on MQTT, no authentication on
the Edge REST API, no per-device certificates, no signed firmware. Anyone on the
home LAN who can reach port 8080 can water a plant. This is an accepted V1
limitation, not an oversight, and it is the first thing to change for any
deployment that is not a single trusted home network. It is deliberately
excluded from M0–M12 so security work does not block the software milestones.

## Alternatives considered

**Everything in one config file including hard limits.** Rejected: it makes the
safety limit editable by anyone who can write the file, and by anything that can
push a file.

**Hard limits in NVS, settable via a privileged provisioning command.**
Rejected for V1: it creates a network path to the limit, which needs
authentication and an audit trail to be trustworthy. Revisit only with per-device
certificates in place.

**Cloud-pushed desired state.** Deferred — see above.

**Clamping out-of-range profile values silently.** Rejected — see L4 above.

**A single flat env-var configuration with no file.** Rejected: nested structure
in env vars alone is painful to read and easy to typo, and there is no natural
place to put comments explaining a tuning choice.

**Vault, SOPS, or sealed secrets.** Rejected as disproportionate for one home
deployment with three secrets.

## Consequences

Positive:

- The safety limit is unreachable from every network path, by construction.
- Configuration drift between desired and applied is visible rather than silent.
- Operators learn real limits at edit time rather than during an incident.
- A new plant cannot water itself until someone says so.

Negative, accepted:

- Changing a hard limit requires reflashing. Intentional, and a genuine cost.
- Three configuration surfaces (TOML/env, MQTT config topic, database rows) is
  more than one; each has a distinct owner and lifetime, which is why they are
  separate.
- No secret management tooling means rotating the MQTT password is a manual
  procedure. Documented in M13.

## Risks

- **An operator puts a password in `edge.toml`** because it is the obvious
  place. *Mitigation:* the config type has no password field readable from the
  file layer — the field exists only in the env layer, so a file entry is
  ignored, and startup logs a warning if a password-shaped key appears in the
  file. Issue M0-005.
- **Config drift going unnoticed** if the drift event is not surfaced.
  *Mitigation:* drift is both a device event and a field in the device API
  response, and the UI shows it on the device page (M12-005).
- **Someone adds a "force" parameter to the water endpoint** to work around a
  lockout. *Mitigation:* reviewed as a safety change; `safety_003_leak_blocks_manual_api`
  asserts the endpoint refuses regardless of parameters.

## Follow-up

- [configuration-model.md](../architecture/configuration-model.md) — normative layering and change matrix.
- M0-005 implements L2 loading and validation.
- M0-008 configures Mosquitto authentication and ACLs.
- M5-003 implements profile validation.
- M6-013 implements config publication and drift detection.
