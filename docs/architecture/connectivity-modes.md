# Connectivity Modes

Until now "offline" meant one thing in this project: the cloud is unreachable.
That was never the whole picture, and it is no longer sufficient. A plant-side
device that loses Wi-Fi must remain useful and safe, not merely silent.

This document defines the modes precisely, because each degrades a *different*
capability and each has a different safe behaviour.

Related: [offline-autonomy.md](offline-autonomy.md) ·
[ADR-015](../adr/015-device-offline-autonomy.md) ·
[failure-model.md](failure-model.md)

---

## 1. The four modes

```text
MODE 0 — CONNECTED (normal)
  internet ✅   LAN ✅   broker ✅   edge ✅   device link ✅
  everything works; cloud history syncs

MODE A — CLOUD OFFLINE
  internet ✅/❌   LAN ✅   broker ✅   edge ✅   device link ✅
  the cloud endpoint is unreachable; nothing local is affected

MODE B — SITE OFFLINE
  internet ❌   LAN ✅   broker ✅   edge ✅   device link ✅
  no route off-site at all: no cloud, and no route to any external service

MODE C — DEVICE ISOLATED
  device cannot reach Wi-Fi, the broker, or the edge
  the device is alone with its sensors, its actuator, and its policy
```

Mode A is a subset of mode B. They are separated because mode B removes every
off-site dependency at once — the cloud, and anything else the deployment might
have reached outward for. Device clocks are deliberately **not** in that set:
time comes from the Edge over MQTT, so mode B does not touch it (§3).

## 2. What each mode degrades

| Capability | Mode 0 | A — cloud offline | B — site offline | C — device isolated |
|---|---|---|---|---|
| Device telemetry → edge | ✅ | ✅ | ✅ | ❌ buffered on device |
| Edge storage, API, metrics | ✅ | ✅ | ✅ | ✅ (minus that device) |
| Recommendations | ✅ | ✅ | ✅ | ❌ edge has no fresh data |
| Edge-commanded watering | ✅ | ✅ | ✅ | ❌ device unreachable |
| **Offline autonomous watering** | n/a | n/a | n/a | ✅ **if provisioned** |
| Cloud history sync | ✅ | ⏸ queued | ⏸ queued | ⏸ queued |
| Edge time sync to devices | ✅ | ✅ | ✅ **over MQTT** | ❌ no refresh |
| Device wall clock | ✅ | ✅ | ✅ **from the Edge** | ⚠️ ages out, see §4 |
| Operator UI | ✅ | ✅ | ✅ | ✅ shows device offline |

The row that changes the architecture is **offline autonomous watering**. In
every earlier version of this plan, mode C meant the plant simply went
unwatered until someone noticed. That is now a supported, explicitly provisioned
capability.

## 3. Mode B and the device clock

SAFETY-002 requires a device to refuse edge commands when its wall clock is not
synchronised. If devices took their time from the public internet, then **mode B
would silently disable all watering across the whole site** — an internet outage
would become an irrigation outage, which is precisely the coupling this project
exists to avoid.

**Decision: devices take wall time from the Edge, over the MQTT connection they
already have.** No NTP client on the device, no NTP daemon on the Edge, and no
time-server configuration field
([ADR-013](../adr/013-clock-and-time-semantics.md),
[mqtt-v1.md](../protocol/mqtt-v1.md) §5.12).

```text
device connects → publishes retained device.status
Edge sees it    → publishes edge.time  (live, retain=false, QoS 1)
device applies  → clock_synced = true
Edge repeats every 300 s while the device is online
```

Consequences:

- Mode B keeps device clocks synchronised, so commanded watering keeps working
  with no internet at all.
- The Edge is a time authority for the site, consistent with it already being the
  authority for staleness, the rolling cap, and `received_at`.
- **Time and commands share one channel.** There is no case where commands can
  reach a device but time cannot, which removes a whole class of partial-failure
  reasoning.
- A device that cannot reach the Edge is by definition in mode C, where a
  different rule applies (§4).

`clock_synced` means *"sufficiently synchronised to the Edge clock"* — the last
applied synchronisation is younger than `TIME_SYNC_MAX_AGE_SECONDS` (1800 s) —
not *"an SNTP transaction succeeded"*.

## 4. Mode C and time

An isolated device cannot refresh its wall clock, and its clock drifts. Naively
this would disable autonomy for the same reason it disables commands.

**The resolution: offline autonomy needs elapsed time, not wall time.**

| Rule | Clock needed |
|---|---|
| dry confirmation duration | monotonic |
| hysteresis | none — value comparison |
| cooldown between cycles | monotonic |
| absorption wait | monotonic |
| measurement staleness | monotonic |
| rolling daily volume cap | monotonic + persisted accumulator |
| **edge command TTL** | **wall clock synchronised to the Edge — refused otherwise** |

Every offline rule is a *duration*, and a monotonic timer measures durations
correctly without ever knowing the date. So an isolated device may act
autonomously, while still refusing any edge command it cannot time-validate.

Across a reboot the monotonic clock resets. The device therefore persists its
budget accumulator and cooldown deadline, and on boot with no trustworthy wall
clock it **assumes no time has passed**: the cooldown keeps running from its
persisted remaining duration and the daily budget is not replenished. That is the
conservative direction — a reboot can only ever delay watering, never grant more
of it. See SAFETY-015.

## 5. Mode transitions

```text
                 link lost                    link restored
  MODE 0/A/B ─────────────────► MODE C ──────────────────────► MODE 0/A/B
       │                          │                                 │
       │                          │ device evaluates offline policy │
       │                          │ buffers events                  │
       │                          │                                 ▼
       │                          │                        RECONCILIATION
       │                          │                   device replays buffered
       │                          │                   events; edge ingests
       │                          │                   idempotently; no dose is
       │                          │                   double-counted, no dose
       │                          │                   is re-issued for water
       │                          │                   already delivered
       ▼                          ▼
  edge commands             offline policy only
  (TTL-validated)           (monotonic, bounded)
```

**Entering mode C** is detected by the device (MQTT/Wi-Fi failure), not
announced. The edge learns of it through the Last Will or through silence.

Losing MQTT also stops time-sync refresh, so an isolated device's `clock_synced`
eventually ages out. That is correct and harmless: no Edge command can reach it
anyway, and offline autonomy runs on the monotonic clock. **On reconnect, Edge
commands stay refused until a fresh `edge.time` has been applied.**

**Leaving mode C** is the dangerous transition, because two parties now hold
partial history. The rules are in
[offline-autonomy.md](offline-autonomy.md) §6 and are enforced by SAFETY-016.

**The edge must not immediately issue a dose on reconnect.** A device that has
been isolated may have watered minutes ago. The edge treats the plant as
`Uncertain` until reconciliation completes and the buffered events have been
applied to the rolling budget.

## 6. What never changes across modes

These hold in every mode, including full isolation:

- Firmware hard limits are compile-time and unreachable by any message
  (SAFETY-007).
- A leak refuses all watering, autonomous included (SAFETY-003).
- Unknown tank level refuses watering (SAFETY-004, SAFETY-012).
- A required measurement that is missing or stale refuses autonomous watering
  (SAFETY-005, SAFETY-017).
- The rolling volume cap is never exceeded, whichever party issued the water
  (SAFETY-006, SAFETY-014).
- Uncertainty means do not actuate — never a guessed default (SAFETY-012).

Offline autonomy is a **narrower** capability than connected operation, not a
mode with relaxed rules.

## 7. What the operator sees

Connectivity mode is surfaced, never inferred by the operator from silence:

| Device state | UI presentation |
|---|---|
| connected | normal |
| offline, no offline policy | "Offline — monitoring stopped, no autonomous control" |
| offline, policy present, automation off | "Offline — monitoring locally, autonomous watering disabled" |
| offline, policy present, automation on | "Offline — autonomous control active (policy vN)" |
| reconnected, reconciling | "Syncing offline history…" |
| reconciled | normal, with offline events visible in history |

A plant that was watered autonomously shows that dose in its history with
`origin: offline_autonomous`, so the operator can always tell who acted.

## 8. Terminology

Use these terms consistently across all documents; `rhizo-docscheck` does not
check prose, so this is a review responsibility.

| Term | Means |
|---|---|
| **cloud offline** | mode A — the cloud endpoint is unreachable |
| **site offline** | mode B — no internet route; LAN intact |
| **device isolated** / **offline autonomous mode** | mode C |
| **connected mode** | modes 0/A/B from a device's point of view |
| **time sync** | the Edge's `edge.time` message and the device's applied synchronisation to it |
| **offline policy** | the persisted, versioned, validated per-plant rule set a device may act on in mode C |
| **reconciliation** | the post-reconnect merge of buffered device events into edge history |

Avoid bare "offline" in new documentation. It is the ambiguity this document
exists to remove.
