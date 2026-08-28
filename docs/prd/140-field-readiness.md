# PRD 140 — Field Readiness Architecture

**Milestone:** M14 · **Status:** PLANNED (architecture only) · **Depends on:** M13

> **Revised 2026-08-26.** Two planning items were added: **optional Helm
> packaging** for server-side components (M14-007) and the **future actuator
> capability model** (M14-008).
>
> Helm is a packaging option for `cloud-api` and the optional observability
> stack. **The plant-side edge controller is explicitly out of scope**: the
> component whose entire purpose is working when things fail should not acquire a
> scheduler's failure modes. Home deployment remains Compose or systemd, and
> Kubernetes is never required for an indoor plant.
>
> The actuator item verifies that the reserved kinds are genuinely representable
> and specifies what each would need — with particular attention to the
> **valve-stuck-open** failure, which is worse than a stuck pump because a valve
> on a pressurised supply has no natural duration bound and needs a hardware-level
> fail-closed bound independent of firmware.
>
> Device offline autonomy ([ADR-015](../adr/015-device-offline-autonomy.md))
> partially addresses this PRD's "assumes an always-connected device" finding for
> the **home** case. The **radio** case is unchanged and still requires a v2
> protocol: a sleeping LoRaWAN node cannot evaluate an absolute TTL, and that
> remains genuinely unresolved.

## Summary

Establish the architecture and the honest constraint list for greenhouse and
agricultural deployments — multi-depth probes, RS485 buses, LoRaWAN/LTE-M/NB-IoT,
gateways, irrigation zones, weather inputs, and battery power — **without
speculative implementation**.

## Problem

The long-term vision is field agriculture. The risk is building for it now: a
farm-scale platform designed before one plant works reliably would be expensive,
unvalidated, and probably wrong.

The opposite risk is real too. Some V1 decisions are cheap now and very expensive
later — a data model that assumes one measurement per plant, or an MQTT contract
that assumes an always-connected device. This PRD identifies which those are,
confirms the ones already handled, and names the ones that will genuinely require
new design.

## Goals

1. Identify V1 assumptions that break at field scale.
2. Confirm the expansion points already reserved.
3. Specify what a v2 protocol would need for constrained radio links.
4. Document the constraints honestly, including the ones with no good answer yet.
5. Recommend the minimum abstractions worth adding in V1 — and reject the rest.

## Non-goals

**Implementing any of it.** No LoRaWAN code, no zone entities, no weather client,
no multi-depth ingestion, **no PCB, no schematic, and no part numbers**. M14
produces documentation and, at most, two or three small reserved seams.

Battery operation itself is explicitly **not** deferred here any more: it landed
in v1 across M5, M6, M9, M10, M12, and M13
([ADR-018](../adr/018-battery-and-deep-sleep-device-mode.md)). What remains in
M14 is the outdoor power case — solar, charging, enclosure, and the seasonal
arithmetic — planned in M14-009 and settled by M10-012's measurements.

Also out of scope permanently: **deriving N/P/K from EC.** See §"Fertility" below.

## User/system flows

None. M14 delivers no user-facing behaviour. The flows below are the *future*
ones being designed for.

```text
multi-depth probe ──RS485──► field controller ──LoRaWAN──► gateway
   ──MQTT──► edge platform ──► zone valve commands
```

## Functional requirements

M14 has no functional requirements in the usual sense. It has **findings** and a
short list of reserved seams.

### Already reserved in V1 (no further work needed)

| Concern | Reservation | Where |
|---|---|---|
| Multiple measurement points per device | `measurements.point` column, defaulted to `'default'` | [ADR-004](../adr/004-sqlite-edge-persistence-model.md) |
| Multiple plants per device, and multiple devices per plant | many-to-many `sensor_bindings`; optional `actuator_bindings` | [ADR-016](../adr/016-plant-binding-and-policy-model.md) |
| Multiple edge instances | `edge_id` partitioning throughout the cloud schema | [ADR-005](../adr/005-cloud-event-model-and-idempotency.md) |
| Transport independence | MQTT contract carries no transport concern | [ADR-002](../adr/002-mqtt-topic-versioning-and-qos.md) |
| Hardware independence | trait-based sensor and pump adapters | [PRD 090](090-esp32-rust-firmware.md) |
| Protocol evolution | `rhizo/v2/` namespace, coexistence supported | [versioning-policy.md](../protocol/versioning-policy.md) |

These cost one column and one default each, and they are the reason the field
version is an extension rather than a rewrite.

### V1 assumptions that break — the honest list

| Assumption | Breaks because | Severity |
|---|---|---|
| ~~Devices are always connected~~ | ~~Last Will and online/offline become meaningless~~ | **resolved in v1** |
| ~~Command TTL of 120 s~~ | ~~a sleeping device may not wake for an hour~~ | **resolved in v1** |
| ~~Mains power~~ | ~~battery devices must sleep and cannot hold a TCP session~~ | **resolved in v1** |
| ~~Edge time sync~~ | ~~a sleeping device receives `edge.time` rarely~~ | **resolved in v1** |
| JSON payloads (~300 B) | LoRaWAN payloads are ~50 B and duty-cycled | **high** |
| Telemetry every 300 s | duty-cycle limits allow a few messages per hour | **high** |
| A device can be pushed to | a duty-cycled radio device cannot receive on demand at all | **high** |
| One pump per device | zones use shared pumps and per-zone valves | medium |
| A plant is the unit of irrigation | a field irrigates zones, not individuals | medium |
| Soil moisture is one number | a root-zone profile is a curve over depth | medium |
| Weather is irrelevant | rain forecast dominates irrigation decisions | medium |
| The LAN is the security boundary | a field gateway is on a public network | **high** |

### Correction, 2026-08-28 — the sleep problem was not the radio problem

This document previously bundled five "high" rows into one finding: *the V1
architecture assumes a device that is awake*, load-bearing for Last Will, for
command TTL, and therefore for SAFETY-002.

**That bundling was wrong, and separating it resolved four of the five.**
[ADR-018](../adr/018-battery-and-deep-sleep-device-mode.md) added a
battery-powered **Wi-Fi** node inside v1, with no protocol bump and no change to
command TTL:

- Last Will still means *unexpectedly absent*, because sleep is **announced** and
  bounded by an Edge-computed window (SAFETY-021).
- Command TTL is unchanged at 120 s, because the command is **minted at the
  wake**, not at the request; the latency lives in an Edge-side intent that never
  reaches a device.
- A waking device receives a fresh, never-retained `edge.time` before any command
  is delivered — the mechanism F-040-17 already provided.
- Mains power is no longer assumed.

The battery Wi-Fi node was never the same problem as LoRaWAN. It is not
duty-cycled, its payloads are not size-constrained, it speaks TCP and MQTT and
JSON, and it can be pushed to *while awake*. It broke exactly one assumption of
the five, and that one was solvable.

**What genuinely remains is the radio**, and it is a real v2 problem: a
duty-cycled device that cannot be pushed to at all needs the polling model in the
design directions below, binary payloads, and a staleness model that scales with
the duty cycle. The general lesson is worth keeping: a group of symptoms with one
plausible shared cause is a hypothesis, not a finding, and testing it here was
worth four rows.

### Design directions

**Sleeping, radio-connected devices.** A v2 protocol would need to invert
command delivery: rather than the edge pushing a short-lived command, the device
polls for pending commands on wake, and the edge holds them with an expiry it
knows the device will evaluate. TTL becomes a wake-count or an absolute time the
device can verify with a slow-drift RTC. Last Will is replaced by an expected
next-contact time — a device is "offline" when it misses its window, not when a
TCP session drops.

Half of that already exists. **The edge-side holding is built** — the durable
command intent of
[ADR-018](../adr/018-battery-and-deep-sleep-device-mode.md) §3 — and the
"expected next-contact time" is the wake window of SAFETY-021. What v2 still has
to add is the *pull*: a device that cannot be pushed to must ask for its pending
work, which needs a new topic pair and a round trip that a Wi-Fi device does not
need and therefore does not have. The unsolved part is narrower than this
document originally recorded, and it is specifically **TTL without a
synchronised clock** on a device the edge cannot reach on demand.

### Battery and solar power (M14-009)

Battery operation is no longer a field-only topic — a balcony pot on a battery is
an M13 home deployment. What M14-009 plans is the **outdoor** case: solar,
charging, enclosure, and the seasonal arithmetic.

```text
solar panel → LiFePO4-compatible charger/controller → battery
            → low-Iq regulation → load switches → ESP32 / sensors / pump
```

Solar is a power source, **not a control architecture**. The battery is the
buffer and the fallback; the panel only refills it. **No watering or safety
decision may read solar availability, battery voltage, or state of charge as
permission** ([ADR-018](../adr/018-battery-and-deep-sleep-device-mode.md) §7).
The failure that prohibition prevents is specific: a device with a full battery
and bright sun "having margin to spare" and watering more freely than one on a
cloudy day, which would make irrigation a function of weather through the least
defensible possible route.

A future low-power PCB might use — and this is a block diagram, **not a design**:

```text
ESP32-C3 module
low-Iq regulator
load switch → RS485
load switch → soil sensor
MOSFET      → pump
```

Nothing is fabricated, and M14's exit criterion that `git diff` shows no
speculative implementation covers this in full: no schematic, no part number, no
solar field in any payload or table.

### Hardware targets — targets to verify, not specifications

These are the engineering targets for a later battery hardware version. **Every
one of them is unverified**, and none may be stated as a specification until
M10-012 has measured complete-system sleep current and wake-cycle energy on
assembled hardware.

| Target | Value | Status |
|---|---|---|
| Normal wake interval | ~15 min, configurable | design input |
| Battery-only endurance | **≥ 6 months** under the defined reference workload | **target — requires M10-012** |
| Outdoor/balcony solar | energy-neutral operation under documented assumptions | **target — requires M10-012 and M14-009** |

The reference workload must be stated alongside the figure or the figure means
nothing: wake interval, sensors sampled, warm-up duration, whether the node
waters, and how often.

**Energy-neutral** is the claim, and it is a bounded one:

```text
energy-neutral operation = measured production exceeds measured consumption
                           over a stated period, at a stated location and
                           season, with a stated reserve margin
```

**Indefinite or infinite autonomy is not a claim this project makes.** A panel
sized for July at 40° is roughly a factor of ten short of the same job in
December at 55°, so solar sizing is done for the worst realistic case — a run of
overcast winter days — with battery days-of-autonomy covering it.

The two currents that must never be conflated:

| | |
|---|---|
| **ESP32-C3 chip deep-sleep current** | a datasheet figure for the die alone |
| **complete board/system sleep current** | regulator quiescent current, load-switch and level-shifter leakage, the RS485 transceiver, the sensor rail, pull-ups, and any indicator LED |

The second determines battery life, is typically an order of magnitude or more
above the first, and is knowable only with a meter — and not a handheld one,
since sleep current is microamps and wake current is hundreds of milliamps within
the same second. **Complete-system sleep consumption is measured before any
autonomy claim is made** (M10-012, F-100-45…F-100-49).

**Payload size.** The envelope is already shaped so a binary encoding (CBOR or
`postcard`) can be substituted behind the same Rust types. A LoRaWAN profile
would drop `device_id` (implied by the LoRaWAN DevEUI) and `boot_id`, shorten
`message_id` to a 32-bit counter scoped by DevEUI, and use fixed-point integers.
This is a v2 protocol, delivered as a gateway translation so the Edge Controller
never learns about radios.

**Zones.** A `zone` entity between plant and device: a zone has a valve, a flow
target, and a set of measurement points. The V1 irrigation state machine ports
directly — the state machine does not care whether its actuator is a pump or a
valve, which is why `evaluate` returns `IssueDose { ml }` rather than
`PumpOn { seconds }`.

**Multi-depth.** `point` already exists. What is missing is a
root-zone aggregation: available water over a depth profile, weighted by root
density. That is a domain function, not a schema change.

**Weather.** An input to the recommendation engine, never to the safety gate.
Rain forecast may say "do not irrigate"; it may **never** say "irrigate despite
a leak". The gate stays local and physical.

**Security.** A field gateway on a public network needs TLS, per-device
certificates, signed firmware, and authenticated APIs. This is the largest
single gap between V1 and any field deployment, and it must not be treated as an
increment.

### Fertility — a deliberate limit

The progression EC → pH → fertilisation tracking → calibrated nitrate sensing →
laboratory correlation → crop nutrient models is a research programme, not a
feature list.

**EC is recorded and trended. No N/P/K value is ever derived from it.** Cheap
"NPK" probes compute their outputs from EC using an undisclosed formula with no
species or soil calibration; presenting those numbers as nutrient measurements
would be a false claim about a real field. This limit is permanent for V1 and is
stated in [PRD 100](100-real-soil-sensor.md) as well.

### Recommended V1 additions — deliberately almost none

| Addition | Recommendation |
|---|---|
| `point` column | **already present** |
| `edge_id` partitioning | **already present** |
| Trait-based adapters | **already present** |
| A `zone` entity | **no** — no consumer; add with valves |
| A weather client | **no** — add with the recommendation change that uses it |
| Binary payload encoding | **no** — add with the radio that needs it |
| A generic "actuator" abstraction over pump/valve | **no** — one implementation is not a pattern |

The consistent answer is no. Every abstraction added without a second consumer is
a guess about a requirement, and guesses accumulate as cost. The seams that *are*
present were chosen because they cost a column each.

## Interfaces

None delivered. Future interface sketches are in the design directions above.

## Data model

No changes. The reserved columns already described are sufficient for the
planning horizon.

## State model

No changes. The M6 state machine is expected to port to zones unchanged, which
is itself a finding worth recording: the abstraction chosen in
[ADR-006](../adr/006-irrigation-state-machine-ownership.md) — a pure function
over inputs returning a volume — happens to be actuator-agnostic.

## Failure modes

Field-specific failures, documented for future design rather than handled:

| Failure | Note |
|---|---|
| Radio link down for days | telemetry gaps are normal, not a fault; staleness thresholds must scale with the duty cycle |
| Device battery depleted | predicted from the voltage trend **where the trend supports one** — a LiFePO4 curve is flat across most of its range, so a projection is reported where defensible and absent where not, never fabricated (M13-016). It grants and refuses nothing: a depleted device stops reporting and its plants lock out on staleness. |
| Solar production below consumption for a season | the battery covers it or it does not; this is why days-of-autonomy sizing exists and why energy-neutrality is stated with a period, a location, and a reserve margin |
| Cold charging | LiFePO4 must not be charged below freezing without protection — a **charger requirement**, not an afterthought |
| Gateway offline | many devices vanish at once; a distinct condition from many devices failing |
| Duty-cycle exhaustion | the device cannot transmit even when it wants to |
| Extreme temperature | affects both sensors and battery chemistry |
| Physical damage or theft | a real field failure mode with no software mitigation |
| Valve stuck open | far worse than a stuck pump — a valve can drain a supply |

The valve row is the field equivalent of SAFETY-007 and will need its own
hardware-level bound, independent of firmware.

## Safety implications

No invariant changes in M14 because nothing is implemented. Two invariants are
identified as **needing rework** for radio-connected devices:

- **SAFETY-002** (expired commands) depends on a device with a synced clock
  evaluating an absolute expiry. A sleeping device with a drifting RTC and an
  hour-long wake interval cannot satisfy the V1 formulation. A v2 mechanism —
  wake-count TTL, or a device-verified sequence horizon — must be designed
  before any radio deployment.
- **SAFETY-005** (stale data) uses a threshold derived from the telemetry
  interval. That formula still works, but the intervals differ by two orders of
  magnitude, so the 15-minute floor becomes meaningless and the operator-facing
  meaning of "stale" changes.

Everything else — leak, tank, daily caps, hard limits, uncertainty defaults —
transfers unchanged, because they are about physical facts and local vetoes
rather than about connectivity.

## Observability

No changes. Noted for the future: a field deployment needs per-device battery
and link-quality metrics, and `devices_offline` becomes a normal nonzero number
rather than an alert condition.

## Testing strategy

M14 delivers documentation, so its verification is review-based:

- Every "already reserved" claim is checked against the actual schema and code,
  not against the ADR that proposed it.
- Every "breaks because" claim is specific enough to act on.
- The recommended-additions table is defended: each "no" states what would change
  the answer.

## Acceptance criteria

- [ ] This PRD and any supporting ADRs are complete and internally consistent.
- [ ] Every reserved expansion point is **verified present in the code**, not
      merely documented.
- [ ] The list of breaking assumptions is specific and actionable.
- [ ] Every recommended addition is either already present or justified as
      deferred.
- [ ] **No speculative implementation has been added.**
- [ ] The fertility limit is stated in both this PRD and
      [PRD 100](100-real-soil-sensor.md).
- [ ] The solar power chain is specified at the level of part **classes**, with
      LiFePO4 justified against named alternatives and cold-charging protection
      stated as a charger requirement.
- [ ] Seasonal energy arithmetic is worked for a **named** location and season,
      with every input labelled measured, cited, or assumed, and
      days-of-autonomy sizing covering a stated run of overcast days.
- [ ] Energy neutrality is used as a bounded, measured claim everywhere it
      appears; **no claim of indefinite or infinite autonomy exists anywhere.**
- [ ] Every autonomy and battery-life figure in the repository is audited: either
      labelled a target requiring measurement, or traceable to M10-012's results.
- [ ] The PCB block sketch is present and explicitly **not** a design — no
      schematic, no part number, no solar field in any payload or table.

## Dependencies

- M13 (a working, supportable multi-plant home system is the prerequisite for
  taking any of this seriously).

## Open questions

These are genuinely unresolved and are the substance of a future field project:

1. **Command delivery to a sleeping device.** Poll-on-wake versus a downlink
   window versus abandoning remote actuation for battery nodes entirely
   (monitoring-only field devices are a legitimate product).
2. **TTL semantics without a reliable clock.** The V1 answer — refuse if unsynced
   — would mean a battery node never waters. A different mechanism is required.
3. **Whether field irrigation should be edge-controlled at all**, or whether the
   field controller should own a local schedule with the platform as an advisor.
   The edge-first principle may resolve differently when the "edge" is a solar
   box in a field with an LTE modem.
4. **Regulatory constraints** on duty cycle and radio bands by region.
5. **Whether the same codebase should serve both** the houseplant and field
   cases, or whether they should share only the domain crate. The current
   architecture makes either possible; the answer depends on how far the
   protocol has to diverge.

## Future work

Everything in this PRD. It is a map, not a plan.
