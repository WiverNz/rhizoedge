# Connectivity Modes

Until now "offline" meant one thing in this project: the cloud is unreachable.
That was never the whole picture, and it is no longer sufficient. A plant-side
device that loses Wi-Fi must remain useful and safe, not merely silent.

This document defines the modes precisely, because each degrades a *different*
capability and each has a different safe behaviour.

Related: [offline-autonomy.md](offline-autonomy.md) ·
[ADR-015](../adr/015-device-offline-autonomy.md) ·
[ADR-018](../adr/018-battery-and-deep-sleep-device-mode.md) ·
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

## 1b. Reachability: a fifth state, orthogonal to the four modes

The four modes above describe what is *broken*. A battery device introduces a
condition that is not a breakage at all: it is asleep, on purpose, and will be
back shortly ([ADR-018](../adr/018-battery-and-deep-sleep-device-mode.md)).

This is deliberately **not** a fifth mode. Sleeping is orthogonal to modes 0/A/B/C
— a sleeping device can perfectly well be in mode B, and a device that sleeps
*and* cannot reach the Edge when it wakes is in mode C like any other. What
changes is only how the Edge interprets silence.

```text
Connected                          awake and reachable
Sleeping   { expected_wake_at }    announced, bounded, and expected back
Isolated                           absent without an announcement, or overdue
Reconciling                        replaying buffered history after an absence
```

`Isolated` is the mode-C state under its existing name; it is what an
"offline unexpectedly" state would be, and it already existed. Only `Sleeping`
is new.

**The rule that makes it safe (SAFETY-021).** A device is `Sleeping` only while
it is inside a window the **Edge** computed from its own clock, and only after
the device announced the sleep:

```text
expected_wake_at = received_at(sleep announcement) + wake_interval_seconds
overdue_at       = expected_wake_at + max(wake_interval_seconds, 300 s)
```

Past `overdue_at`, the device is `Isolated`. So the new state can only ever
*defer* the offline indication, never suppress it — a flat battery, a failed wake
timer, and a stolen node all surface, late but reliably.

Two directions of interpretation carry the safety weight, and both resolve the
same way:

| Input | Interpretation |
|---|---|
| device's announced `expected_wake_ms` | advisory only; never extends the Edge's window |
| Last Will (`connection_lost`) | `Isolated` — a will is not an announcement |
| offline status with an unrecognised `reason` | `Isolated` (SAFETY-012) |
| `power.mode` absent or unrecognised in config | `always_on` — never start sleeping on a guess |

A device with a wrong clock cannot make itself look punctual, for the same reason
it cannot make stale data look fresh (SAFETY-005).

**Commands.** A sleeping device cannot be commanded, so the Edge holds the
operator's request as a durable *intent* and mints the command when the device is
actually awake, re-running the full safety gate at that moment
([ADR-018](../adr/018-battery-and-deep-sleep-device-mode.md) §3). Nothing is
retained, no TTL is widened, and the latency — up to one wake interval — is
surfaced as `pending_for_device_wake` rather than hidden.

**Time.** A waking device receives a fresh, never-retained `edge.time` before any
command is delivered to it, exactly as F-040-17 already requires. The retained
sleep announcement carries `expected_wake_ms` as a **diagnostic only**; no field
of any retained message may ever set a clock.

**Autonomy.** Deep sleep does not weaken §4's rule. The RTC timer keeps running
across a timer wake, so elapsed time is credited from a genuine monotonic source
when the RTC-retained state's checksum is valid, and falls back to "assume no
time has passed" on any other reset reason or a failed checksum. A deep-sleep
wake is not a reboot for accounting purposes; a reboot is still never a way to
earn budget (SAFETY-015).

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

A battery device's ordinary cycle looks superficially similar and is not the same
thing at all:

```text
                announce sleep              wake, publish status
   CONNECTED ────────────────────► SLEEPING ─────────────────────► CONNECTED
       │                              │
       │                              │ overdue_at passes
       │  link lost, no announcement  ▼
       └────────────────────────► ISOLATED  ──► (mode C rules apply in full)
```

The distinction is the announcement, and it is the only distinction. A battery
device that fails to reach the broker on waking is in mode C exactly like a mains
device, evaluates its offline policy on the monotonic clock exactly like a mains
device, and reconciles on reconnect exactly like a mains device. Sleeping changes
nothing about mode C; it changes only whether silence between wakes means
anything.

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
| sleeping, inside its window | "Sleeping — next wake expected around 14:45" — a normal state, **never** an error treatment |
| sleeping, overdue past `overdue_at` | "Offline unexpectedly" with the missed-wake count and how long it has been overdue |
| a dose held for a sleeping device | "Pending until device wakes", with the expected delivery time — never a spinner, never "sent" |
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
| **sleeping** | a battery device inside an announced, Edge-computed wake window — reachability, not a mode |
| **overdue** | a sleeping device past `overdue_at`; presented as offline unexpectedly, never as sleeping |
| **wake window** | `expected_wake_at`…`overdue_at`, derived from the Edge's `received_at` and the configured wake interval |
| **command intent** | a durable Edge-side record of what an operator asked for, held until the device is awake; not a command, and never on the wire |

Avoid "asleep" as a synonym for "offline" as carefully as you avoid bare
"offline". A sleeping device is expected back; that is the entire distinction.

Avoid bare "offline" in new documentation. It is the ambiguity this document
exists to remove.
