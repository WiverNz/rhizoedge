# ADR-018 — Battery and deep-sleep device mode

## Status

Accepted — 2026-08-28. Contract and edge model delivered by the dated post-M4
battery-compatibility correction, durable command delivery
in M6, end-to-end scenarios in M8, firmware mechanics in M9, measurement in M10,
presentation in M12, fleet operations in M13, solar and field power in M14.
Planning only; no runtime code was written when this was accepted.

**Extends [ADR-007](007-esp32-rust-framework-and-toolchain.md)**, which scoped
the firmware to "a mains-powered indoor node" and deferred low power to M14.

It does **not** amend [ADR-013](013-clock-and-time-semantics.md): the wire
semantics of `edge.time` and of command TTL are unchanged, and §4 below explains
why they did not need to change. It does **not** amend
[ADR-015](015-device-offline-autonomy.md) or
[ADR-006](006-irrigation-state-machine-ownership.md) either. Sleeping introduces
no second watering authority — a sleeping device is a device that is sometimes
unreachable, and this project already has a complete answer for that.

## Context

The planned device is mains-powered, holds a TCP session open indefinitely, and
is "offline" exactly when something has gone wrong. That is a good model for a
node on a window sill next to a socket, and a useless one for a pot on a balcony
with no socket within reach.

A Wi-Fi ESP32 node running from a battery cannot hold a session open. It must
sleep for most of its life, wake on a timer, sample, transmit, and sleep again.
Almost everything else about it is unchanged: the same silicon, the same
protocol, the same JSON, the same broker, the same safety gate.

[PRD 140](../prd/140-field-readiness.md) already records this under "V1
assumptions that break", where it is bundled with LoRaWAN into one
high-severity group: *the V1 architecture assumes a device that is awake.* That
bundling is now wrong, and separating it is most of this decision.

**The battery Wi-Fi node breaks exactly one assumption of the five.** It is not
duty-cycled, its payloads are not size-constrained, it speaks TCP and MQTT and
JSON, and it can be given a fresh `edge.time` at every wake. It is only
sometimes absent. That single break is solvable inside v1; the remaining four
still require v2 and a radio, and stay in M14.

The competing pressure is honesty about power. "Six months on a battery" and
"solar means it runs forever" are the two claims this class of device attracts,
and both are usually made from a datasheet rather than from a meter. A chip's
deep-sleep current says almost nothing about a board's.

## Decision

### 1. Sleep is a declared state, never an inferred one

A device announces its intent to sleep before it disconnects, and the edge
records an **expected wake window** derived from its own clock. Absence inside
that window is `sleeping`. Absence outside it is `isolated` — the existing state
for a device that has gone quiet unexpectedly.

```text
Connected                          awake and reachable
Sleeping   { expected_wake_at }    announced, bounded, and expected back
Isolated                           absent without an announcement, or overdue
Reconciling                        replaying buffered history after an absence
```

These are the existing `connected | isolated | reconciling` values of the device
API with one variant added, not a parallel model. `Sleeping` is new; `Isolated`
already carried "offline unexpectedly" and keeps that meaning unchanged.

**A sleeping device that misses its wake window becomes `isolated`.** The
expected-sleep state is bounded by construction and cannot absorb a device that
has stopped waking — which is the whole failure mode a "quiet is fine now" state
would otherwise introduce. This is SAFETY-021.

Announcement is a clean shutdown: the retained `device.status` carries
`status: "offline"` with `reason: "sleeping"`. A device that drops its session
without announcing fires its Last Will with `reason: "connection_lost"` exactly
as before, and is `isolated`. An unrecognised reason is treated as
`connection_lost` — uncertainty resolves to *unexpectedly absent*, never to
*peacefully asleep* (SAFETY-012).

### 2. The edge's clock owns the wake window, not the device's

The device's announced `expected_wake_ms` is advisory, exactly as
`connectivity.mode` already is ([mqtt-v1.md](../protocol/mqtt-v1.md) §5.5). The
edge computes the authoritative window from its own `received_at` plus the
`wake_interval_seconds` it configured:

```text
expected_wake_at = received_at(sleep announcement) + wake_interval_seconds
overdue_at       = expected_wake_at + max(wake_interval_seconds, 300 s)
```

A device with a wrong clock therefore cannot make itself look punctual, for the
same reason it cannot make stale data look fresh (SAFETY-005).

### 3. Commands for a sleeping device are held as intents, not as messages

The current pipeline persists a command and publishes it immediately. For a
sleeping device that is not merely slow, it is wrong: the message would sit in
the broker for up to a wake interval and arrive carrying a TTL minted before the
device went to sleep.

The edge instead persists an **intent** — what the operator asked for — and mints
the actual command at the moment the device is awake:

```text
operator requests a dose
  → edge runs the safety gate against last-known state
  → device is battery-mode and not awake
      → persist a command intent          state: pending_for_device_wake
      → 202, carrying expected_delivery_after and intent_expires_at
  → device wakes and publishes status
      → edge sends edge.time  (unchanged, F-040-17)
      → edge RE-RUNS the full safety gate against current inputs
      → edge allocates ONE command_id, persists the command row, publishes
                                          state: sent
      → normal result handling; a delivery retry reuses that command_id
```

Three properties fall out of this, and they are why it is shaped this way:

- **An intent is not a command.** No `command_id` exists until delivery, so
  "persist before publish" and "a retry never generates a new `command_id`" are
  both untouched (SAFETY-001, SAFETY-010).
- **The gate runs against fresh data, not stale intent.** A leak that appeared
  while the device slept refuses the dose at delivery. This is strictly safer
  than the always-on path, where the gate runs once at request time.
- **The wire is unchanged.** No new topic, no new retention, no broker-side
  queue. Delivery happens while the device is connected, so it is an ordinary
  command in every respect.

**At most one open water intent per plant.** A second request while one is
pending returns 409 naming the pending intent. Without this, an impatient
operator could queue several doses that all deliver at one wake — the rolling cap
would still bound the total (SAFETY-006), but arriving at the cap by accident is
not a design.

An intent that is never delivered expires on the edge's clock at
`intent_expires_at` (default `2 × wake_interval_seconds`, floor 30 minutes) and
is recorded as `expired_before_wake`. Nothing is ever retained on MQTT to achieve
any of this.

### 4. Command TTL semantics are unchanged, and that is a finding

The 120-second TTL was chosen so that "a queued command is almost always stale by
the time a reconnecting device sees it, which is the intent"
([time-model.md](../architecture/time-model.md) §4). The obvious worry is that a
15-minute wake interval breaks it.

It does not, because the command is minted at delivery. The device receives a
command issued seconds ago, from an edge it has just synchronised with, and
evaluates it under exactly the rules it already had. **No change to TTL, to
`edge.time`, or to SAFETY-002 is required** — the latency lives in the intent,
which is an edge-side record with its own operator-visible expiry, and never on
the wire.

The retained sleep announcement carries `expected_wake_ms` as a diagnostic. It is
**not a time source**: a device MUST NOT apply any field of any retained message
to its clock. Only `edge.time`, which is never retained, sets a clock.

### 5. Power mode is configuration, and defaults to always-on

```text
PowerMode::AlwaysOn    hold the session; today's behaviour; the default
PowerMode::Battery     wake on a timer, sample, transmit, sleep
```

Delivered in the retained `device.config` alongside `wake_interval_seconds`,
`sensor_warmup_ms`, and `awake_budget_seconds`. An unrecognised mode decodes to
`AlwaysOn`, because sleeping is the branch that makes a device unreachable and
uncertainty must not choose it.

The wake cycle:

```text
deep sleep → timer wake → power the sensor rail and RS485 → warm-up delay
  → sample → Wi-Fi → MQTT → status, telemetry → receive time, config, policy,
  pending commands → [stay awake for a watering cycle if one is delivered]
  → announce sleep → peripherals off → deep sleep
```

**A device stays awake for the whole of an active watering cycle.** Sleeping
mid-dose is not a supported state: the run guard, the leak watch, and the tank
check all require the device to be running. The pump is de-energised and the
result durably published before sleep is entered, and an interrupted dose is
reported exactly as it already is.

### 6. Deep sleep may credit elapsed time only from a validated RTC monotonic source

[connectivity-modes.md](../architecture/connectivity-modes.md) §4 requires that a
device booting without a trustworthy wall clock **assumes no time has passed**:
the cooldown keeps running from its persisted remainder and the daily budget is
not replenished (SAFETY-015). Applied naively to a device that sleeps 96 times a
day, that rule would freeze the offline evaluator permanently.

It does not have to be weakened, because it is a rule about *clock uncertainty*,
not about reboots. The ESP32's RTC timer continues across deep sleep and is a
genuine monotonic source. So:

- **Timer wake, with the RTC-retained state's checksum valid** → the RTC
  counter's elapsed time is credited. This is a monotonic measurement, not a
  guess, so SAFETY-015 is satisfied as written.
- **Any other reset reason, or a failed checksum** → fall back to SAFETY-015's
  rule and assume no time has passed.

A deep-sleep wake is therefore not a reboot for accounting purposes, and a reboot
is still never a way to earn budget.

### 7. Solar is a power source, never a control input

```text
solar panel → LiFePO4-compatible charger/controller → battery
            → low-Iq regulation → load switches → ESP32 / sensors / pump
```

The battery is the buffer and the fallback; the panel only refills it. **No
watering or safety decision may read solar availability, battery voltage, or
state of charge as a permission.** Battery voltage is telemetry: it may raise an
alert, and it shortens nothing. A low battery is a maintenance condition, not a
watering rule.

This preserves the property that makes the rest of the architecture reviewable:
there is one set of inputs to the safety gate, and power is not among them.

### 8. Autonomy claims are measured, or they are not made

Two numbers are routinely confused, and are kept separate throughout this
project's documentation:

| | |
|---|---|
| **chip deep-sleep current** | an ESP32-C3 datasheet figure for the die alone |
| **complete-system sleep current** | what the assembled board actually draws: regulator quiescent current, load-switch and level-shifter leakage, the RS485 transceiver, the sensor rail, pull-ups, and the LED somebody left fitted |

The second is the one that determines battery life, is typically an order of
magnitude or more above the first, and is knowable only with a meter. **No
autonomy figure is stated as a specification until the complete-system figure has
been measured** (M10-012).

"Infinite autonomy" is not a claim this project makes about solar. The claim is
**energy-neutral operation**: measured production exceeds measured consumption
over a stated period, at a stated location and season, with a stated reserve
margin. Anything else is a datasheet multiplied by optimism.

## Alternatives considered

**Retain commands on the `commands/water` topic so the broker delivers them at
wake.** Rejected, and it is the trap this ADR exists to avoid. Retained commands
are already a protocol violation ([mqtt-v1.md](../protocol/mqtt-v1.md) §3)
precisely because the broker would redeliver them on every reconnect
indefinitely, causing repeated watering. A sleeping device reconnects around a
hundred times a day, which makes the worst case for retained commands roughly a
hundred times worse rather than better.

**Let the device poll a "pending work" topic on wake.** Rejected for v1: it adds
a request/response round trip and a new topic pair to save nothing, because the
edge already learns the device is awake from the retained status it publishes on
connect. This is the right answer for a duty-cycled radio device that cannot be
pushed to at all, and it remains the M14 design direction for v2.

**Treat a sleeping device as simply offline and accept the UI showing it as
failed.** Rejected. It trains the operator to ignore the offline indicator, which
is the indicator that a device has actually died. A status everybody ignores is
worse than no status.

**Widen the command TTL for battery devices.** Rejected, and it was the first
idea. It weakens SAFETY-002 for the class of device that is *hardest* to inspect,
in exchange for saving an edge-side table. Minting the command at delivery gets
the same behaviour with the safety property untouched.

**`no_std` firmware for lower idle power.** Not decided here. ADR-007 already
records it as reconsiderable for battery nodes, and the trait-based HAL keeps the
option open. M10-012's measurement is what should decide it — noting that
deep-sleep current is dominated by board hardware rather than by whether the
firmware links `std`, so this is likely to be the wrong lever.

**A dedicated battery-node firmware image.** Rejected. One image with a
configured power mode keeps the M9 conformance test meaningful and avoids two
divergent safety paths, for the same reason there is one
`validate_water_command`.

## Consequences

Positive:

- A pot with no socket nearby becomes a supported deployment rather than a future
  one.
- Four of PRD 140's five "high" connectivity breakages are correctly separated
  from the fifth, and the fifth is closed inside v1 with no protocol bump.
- Command delivery gains a re-run of the safety gate against fresh inputs, which
  is a safety improvement for battery devices over the always-on path.
- The operator gains an honest distinction between "asleep" and "dead" that the
  current model cannot express.

Negative, accepted:

- Manual watering on a battery device has bounded latency of up to one wake
  interval. This is inherent, is surfaced explicitly as `pending_for_device_wake`,
  and is not hidden behind a spinner.
- A new persisted intent table, a new liveness state, and a new firmware power
  path — three places where a bug can hide, in the milestone-spanning way that is
  hardest to test. Mitigated by SCEN-110…SCEN-117, which run against the
  simulator with no hardware.
- The energy budget is dominated by awake time rather than by sleep current
  (M10-012), so the six-month target is genuinely at risk and will be settled by
  measurement rather than by design.

## Risks

- **The sleep state becomes a place where dead devices hide.** *Mitigation:*
  SAFETY-021 and the bounded window — a device is `sleeping` only inside a window
  the edge computed, and `isolated` the moment it is overdue.
- **The measured complete-system sleep current makes the target unreachable.**
  *Mitigation:* M10-012 measures before anything is claimed, and the numbers in
  [PRD 140](../prd/140-field-readiness.md) are labelled as targets to verify, not
  as specifications.
- **Sleep and offline autonomy interact in the accounting.** A device that sleeps
  *and* is isolated exercises both mechanisms at once. *Mitigation:* §6's rule is
  narrow and testable, and SCEN-115 is exactly this combination.
- **Solar quietly becomes load-bearing** — a reviewer decides that a device with a
  full battery may water more freely. *Mitigation:* §7 states the prohibition in
  the same terms as the other gate inputs, and no power field is an input to
  `IrrigationInputs`.
