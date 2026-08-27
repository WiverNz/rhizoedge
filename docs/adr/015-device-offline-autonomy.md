# ADR-015 — Device offline autonomy

## Status

Accepted — 2026-08-26. Contract in M1, evaluator in M6, firmware in M9.

**Amends [ADR-003](003-edge-first-ownership-model.md) and
[ADR-006](006-irrigation-state-machine-ownership.md)**, both of which previously
stated that the device contains no irrigation intelligence. That claim is now
too strong and is corrected here.

## Context

The project's governing principle was expressed as "a plant must remain safely
monitored and controllable locally even when Internet/cloud connectivity is
unavailable". Every design decision honoured it — for the *cloud*.

It did not honour it for the case that actually kills houseplants: **the Wi-Fi
router reboots, or the edge host dies, while the owner is away for two weeks**.
In the architecture as planned, that plant simply went unwatered. The device had
sensors, water, a pump, and a rule it could have followed, and did nothing
because it had been designed to be incapable of deciding.

[connectivity-modes.md](../architecture/connectivity-modes.md) now separates
cloud offline (A), site offline (B), and device isolated (C). This ADR decides
what happens in C.

Two things must be preserved while fixing this:

1. The device must not improvise. A device that invents a threshold is more
   dangerous than a device that does nothing.
2. The firmware hard limits must remain the final, unreachable boundary
   (SAFETY-007).

## Decision

### A device may act autonomously only from a validated, persisted, versioned policy

The Edge authors the policy, validates it, and publishes it retained and
versioned. The device parses, re-validates against its own declared capabilities
and its compile-time hard limits, stages, activates atomically, and acknowledges
the applied version.

Absence of a policy, a policy that fails validation, a policy whose required
measurements are unavailable, or a corrupt store all resolve the same way:
**no actuation** (SAFETY-013). Absence is never permission.

`enabled` defaults to `false`. Offline autonomy is opted into per plant by a
human, the same posture as `auto_watering_enabled`.

### The offline evaluator is a deliberately restricted subset

Permitted: threshold, confirmation duration, hysteresis, cooldown, a fixed
policy-authored dose, bounded dose count, absorption wait, rolling volume cap,
required-measurement and staleness checks, and the full safety gate.

Forbidden: trend fitting, recommendation generation, confidence scoring,
manual-watering detection, cross-plant reasoning, reservoir arbitration, policy
authoring, and any computed dose size.

The full model is in
[offline-autonomy.md](../architecture/offline-autonomy.md).

### The evaluator is one pure function in a new shared crate

```text
crates/policy → rhizo-policy      no_std + alloc
```

`evaluate_offline(policy, state, inputs, elapsed) -> OfflineDecision` is pure,
allocation-frugal, and called from **exactly one place** in the firmware and one
place in the simulator — the same discipline that makes
`validate_water_command` trustworthy
([ADR-008](008-shared-code-simulator-and-firmware.md)).

`rhizo-domain` links the same crate. This is not incidental: it lets the Edge
**validate a policy before publishing it** and **predict what an isolated device
will do**, which is what makes reconciliation tractable. A policy the Edge cannot
evaluate is a policy it must not send.

Dependency direction: `mqtt-contract ← policy ← domain`.

### Offline autonomy runs on monotonic time

Every offline rule is a duration — confirmation, hysteresis dwell, cooldown,
absorption wait, staleness, budget window. Durations need a monotonic timer, not
a calendar.

Therefore an isolated device with an unsynced wall clock **may** act
autonomously, while still **refusing every edge command** it cannot TTL-validate
(SAFETY-002 unchanged).

Across reboot the monotonic clock resets, so the device persists the budget
accumulator and the cooldown as a *remaining duration*, and assumes no time
passed. A reboot can only delay watering, never grant more of it (SAFETY-015).

### Bounded event buffer with tiered retention

Audit events (autonomous doses, refusals, lockouts, policy activations, faults)
outrank telemetry samples and are never evicted by them. Eviction records an
explicit **gap marker** that is reported, stored, and shown in history
(SAFETY-020).

### Reconciliation is idempotent and blocks premature dosing

Replay deduplicates on device-generated `event_id` through the existing
`processed_messages` mechanism. Autonomous doses become `watering_events` with
`origin = offline_autonomous` and count toward the **same** rolling budget as
commanded doses — one budget per plant, not one per control path.

The Edge holds a reconnecting plant in `Uncertain` until replay completes, so it
cannot issue a dose on top of an autonomous dose delivered ninety seconds ago
(SAFETY-016).

## Alternatives considered

**Leave mode C unhandled** (the previous design). Rejected: it fails the
project's own stated principle in the case that most often matters, and the
hardware is already capable.

**Full Edge Controller in firmware.** Rejected outright. It would mean two
implementations of the recommendation engine, an ESP32 running `sqlx`-shaped
logic, and a safety surface that could not be property-tested cheaply. The
restricted subset exists precisely so the firmware stays auditable.

**Device caches the last Edge decision and repeats it.** Rejected as unsafe: a
decision computed against six-hour-old soil is not evidence about current soil,
and the mechanism has no way to stop.

**A timer-based fallback** — "water 30 ml every 48 h if isolated". Rejected: this
is open-loop irrigation. It ignores the sensor the device is holding, and it
keeps watering a pot that is already saturated. It is the single most common way
hobby irrigation projects drown plants.

**Wall-clock-based offline rules with an RTC.** Rejected as unnecessary: a
battery-backed RTC adds a part and still drifts, when every offline rule only
needs elapsed time. Revisit only if a future rule genuinely needs the date (a
day/night schedule would).

**Put the evaluator in `rhizo-mqtt-contract`.** Rejected: that crate describes
bytes on the wire. Mixing a decision engine into it would blur the one boundary
the firmware most needs to be able to trust, and would make the contract crate
grow every time a policy rule changes.

## Consequences

Positive:

- A plant survives a router outage, which is the failure the owner actually
  experiences.
- The offline safety surface is a pure function, so SAFETY-013…020 are
  property-testable without hardware — the same economics that made SAFETY-006
  affordable.
- The Edge can validate and predict, because it links the same crate.
- Simulator-first still holds: M2 delivers the device mechanics, M6 installs the
  shared evaluator and simulator call site, and M8 tests full autonomy end to
  end — all before an ESP32 is involved.

Negative, accepted:

- **Two evaluators exist** and can disagree about whether a plant needs water.
  Bounded by sharing the offline rules, not eliminated.
- **Firmware complexity grows** in the hardest place to debug: NVS state, an
  event ring, atomic activation, monotonic accounting.
- **A divergence window exists at reconnection**, bounded by the reconciliation
  rules rather than removed.
- **History has gaps** when the buffer overflows. Made visible rather than
  hidden.
- The device now holds per-plant configuration, so provisioning has more state
  to get right.

## Risks

- **Policy drift.** A device runs an old policy for weeks while isolated.
  *Mitigation:* `applied_policy_version` is reported on every reconnect and drift
  is surfaced in the UI; the policy carries no expiry because expiring it would
  mean disabling watering for an absent owner, which is the wrong direction.
- **Budget divergence.** The device's local accumulator and the Edge's row-derived
  budget disagree after a long isolation. *Mitigation:* on reconciliation the
  Edge's row-derived value is authoritative and is pushed back as the device's
  new baseline; the device's accumulator is conservative in the interim.
- **A future contributor adds a "sensible default" to the offline path** to make
  a test pass. *Mitigation:* SAFETY-013's test asserts that a device with no
  policy never actuates, and the gate has no catch-all arm.
- **Silent buffer overflow** hiding an autonomous dose. *Mitigation:* audit tier
  never evicted by telemetry; gap markers are first-class events; SAFETY-020.

## Follow-up

- [offline-autonomy.md](../architecture/offline-autonomy.md) — normative model.
- [connectivity-modes.md](../architecture/connectivity-modes.md) — mode definitions.
- [ADR-016](016-plant-binding-and-policy-model.md) — where the policy comes from.
- SAFETY-013…020 in [safety-invariants.md](../architecture/safety-invariants.md).
- M1 adds the policy payload and capability contract; M6 the evaluator and its
  property tests; M9 the firmware side; M8 the isolation scenarios.
