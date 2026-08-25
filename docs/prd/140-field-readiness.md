# PRD 140 — Field Readiness Architecture

**Milestone:** M14 · **Status:** PLANNED (architecture only) · **Depends on:** M13

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
no multi-depth ingestion. M14 produces documentation and, at most, two or three
small reserved seams.

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
| Multiple measurement points per device | `measurements.measurement_point` column, defaulted | [ADR-004](../adr/004-sqlite-edge-persistence-model.md) |
| Multiple plants per device | `plants.device_id` many-to-one | ADR-004 |
| Multiple edge instances | `edge_id` partitioning throughout the cloud schema | [ADR-005](../adr/005-cloud-event-model-and-idempotency.md) |
| Transport independence | MQTT contract carries no transport concern | [ADR-002](../adr/002-mqtt-topic-versioning-and-qos.md) |
| Hardware independence | trait-based sensor and pump adapters | [PRD 090](090-esp32-rust-firmware.md) |
| Protocol evolution | `rhizo/v2/` namespace, coexistence supported | [versioning-policy.md](../protocol/versioning-policy.md) |

These cost one column and one default each, and they are the reason the field
version is an extension rather than a rewrite.

### V1 assumptions that break — the honest list

| Assumption | Breaks because | Severity |
|---|---|---|
| Devices are always connected | LoRaWAN devices sleep; Last Will and online/offline become meaningless | **high** |
| JSON payloads (~300 B) | LoRaWAN payloads are ~50 B and duty-cycled | **high** |
| Telemetry every 300 s | duty-cycle limits allow a few messages per hour | **high** |
| Command TTL of 120 s | a sleeping device may not wake for an hour | **high** |
| Mains power | battery devices must sleep and cannot hold a TCP session | **high** |
| SNTP-synced clocks | a sleeping device syncs rarely; SAFETY-002 depends on this | **high** |
| One pump per device | zones use shared pumps and per-zone valves | medium |
| A plant is the unit of irrigation | a field irrigates zones, not individuals | medium |
| Soil moisture is one number | a root-zone profile is a curve over depth | medium |
| Weather is irrelevant | rain forecast dominates irrigation decisions | medium |
| The LAN is the security boundary | a field gateway is on a public network | **high** |

The five "high" rows in the connectivity group are one problem wearing five
hats: **the V1 architecture assumes a device that is awake.** That assumption is
load-bearing for Last Will, for command TTL, and therefore for SAFETY-002.

### Design directions

**Sleeping, radio-connected devices.** A v2 protocol would need to invert
command delivery: rather than the edge pushing a short-lived command, the device
polls for pending commands on wake, and the edge holds them with an expiry it
knows the device will evaluate. TTL becomes a wake-count or an absolute time the
device can verify with a slow-drift RTC. Last Will is replaced by an expected
next-contact time — a device is "offline" when it misses its window, not when a
TCP session drops.

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

**Multi-depth.** `measurement_point` already exists. What is missing is a
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
| `measurement_point` column | **already present** |
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
| Device battery depleted | must be predicted from voltage trend, not discovered |
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
