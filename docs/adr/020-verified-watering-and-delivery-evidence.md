# ADR-020 — Verified watering and delivery evidence

## Status

**Proposed** — 2026-09-01. Entities, evidence model, and enforcement in **M16**
([PRD 160](../prd/160-verified-watering.md)), which depends on **M11** — real
pump hardware — and not on M12, M13, M14, or M15.

Additive within MQTT v1 throughout
([versioning-policy.md](../protocol/versioning-policy.md) §1): two
`MeasurementKind` variants, one optional `delivery` object on `command.result`,
two optional fields on `actuator.state`, three `RejectReason` variants, and two
device `EventKind` values. No topic is added, no field is removed or retyped, no
retention or QoS rule changes, and the device subscription set stays at eight
exact topics.

Amends one deferral and one open question, both in
[PRD 110](../prd/110-real-pump-and-safety-hardware.md):

- §Future work deferred "flow-meter verification" to **M14**. M14 is
  documentation-only field-readiness planning and was never going to implement
  it. The work moves to M16 and keeps its own milestone.
- §Open questions 3 asked "whether a flow meter is worth adding early", noting
  that inexpensive flow meters are unreliable at these flow rates. **That
  concern is correct and this ADR answers it by not using one** — see §4.

Does **not** amend [ADR-006](006-irrigation-state-machine-ownership.md),
[ADR-014](014-failure-and-retry-policy.md), or
[ADR-015](015-device-offline-autonomy.md). The edge still decides, the device
still vetoes, an isolated device still waters from its own policy, and no retry
rule is loosened.

## Context

The system's entire claim that a plant was watered rests on one number in one
message: `command.result.delivered_ml`. Follow it back and it is not a
measurement.

```text
requested_ml ──► effective_ml ──► run_ms = effective_ml / ml_per_second
                                      │
                                      ▼
                              the pump is energised for run_ms
                                      │
                                      ▼
                       delivered_ml := effective_ml   ← an assumption
```

`ml_per_second` comes from M11-004: five timed runs into a measuring cup, mean
and standard deviation, recorded with a date. It is a good calibration. It is
also a statement about how the pump behaved on a bench, on one day, with one
piece of tubing, into open air — and it is then used to assert a physical fact
about every dose thereafter.

Everything between the relay and the root zone is unobserved:

```text
   command       relay/MOSFET      pump head      tube      pot
      │               │                │            │         │
      └─ known ───────┴─ known ────────┘            │         │
                       (GPIO state, run duration)   │         │
                                                    └── unobserved ──┘
```

An empty reservoir, a blocked or kinked tube, a tube that fell out of the pot, a
perished peristaltic tube pumping air, a pump head that lost its prime, a
partially occluded line, tubing that hardened over six months and now delivers
70 % of its calibration, an anti-siphon valve stuck shut, or a relay that clicks
while the pump does nothing — every one of these produces a *successful*
`completed` result carrying a `delivered_ml` the system then credits to the
plant's rolling budget as though water had arrived.

The existing mitigations are real, and they are all *indirect and late*:

- **`no_delivery::no_delivery_detected`** notices after **two** unresponsive
  doses that soil moisture and pot weight both failed to move. It is the right
  check and it is the wrong instrument for this question: it is a claim about
  the plant, minutes to hours later, and it cannot fire at all until water has
  already been pumped twice at whatever the tube is actually pointing at.
- **`detect::detect_manual_watering`** infers water the system did not deliver,
  from soil and weight steps.
- **The device gate** (protocol §5.8) refuses on tank, leak, and calibration, all
  *before* actuation. Nothing checks anything *during* it.
- **`credited_ml`** is conservative for `interrupted` and `failed` — it charges
  the full request — which correctly bounds the budget and says nothing about
  what actually happened.

So the system can already answer "did the plant respond?" and "was the command
accepted?". It cannot answer the question in between, which is the one an
operator actually asks: **did water physically leave the reservoir and go down
the tube, and how much?**

There is also a failure it cannot represent at all. Water moving when nothing
authorised it — a siphon through a tube left below the reservoir waterline, a
valve stuck open, a pump driven by a shorted MOSFET — is not a leak (the tray
may be dry) and not a watering (no command exists). Today it is invisible until
the reservoir is empty and the pot is drowned.

## Decision

Introduce **Verified Watering**: an explicit, typed model of what is known about
each actuation, backed by a physical witness that measures volume leaving the
reservoir, and an outcome vocabulary in which "we do not know" is a first-class
answer.

### 1. The product capability is *Verified Watering*; the domain says *delivery*

`Verified Watering` names the capability in the README, the roadmap, and the UI.
The domain module is `rhizo_domain::delivery`, and its types are
`DeliveryOutcome`, `EvidenceLevel`, `HydraulicEvidence`, `FlowObservation`, and
`DeliveryRecord`. "Proof of watering" is rejected as a name: *proof* overstates
what a sensor establishes, and the phrase invites a reading this project has no
interest in.

**`DeliveryEvidence` is already taken**, by `irrigation::no_delivery`, where it
means *soil and pot-weight* response — the biological half. It is not renamed.
The two are genuinely different evidence about different questions, the module
path already disambiguates them, and renaming a type in the safety-critical
irrigation machine to make room for a new feature is the kind of change that is
free to write and expensive to review. The new type is `HydraulicEvidence`, and
each carries a doc comment naming the other.

### 2. Six doses, not one

The single most useful thing this ADR does is refuse to let one number mean six
things. Each is recorded, and each has a different author:

| Value | Author | Meaning |
|---|---|---|
| `requested_ml` | policy (static `dose_ml`, or M15's clamped proposal) | what the plant is thought to need |
| `authorized_ml` | edge safety gate and rolling cap | what the edge is willing to command |
| `commanded_ml` | the wire | what `command.water.requested_ml` carried |
| `effective_ml` | device gate steps 10 and 12 | what survived the firmware clamps; `clamped` already marks this |
| `estimated_ml` | `run_ms × ml_per_second` | what calibration says was pumped — today's `delivered_ml` |
| `measured_ml` | the delivery witness | what a sensor observed leaving the reservoir |

Today the system collapses all six into `delivered_ml` and loses the audit trail
at every step. `authorized_ml` and `commanded_ml` are equal in V1 and are still
stored separately, because the moment they can differ is the moment nobody will
remember they could.

### 3. Evidence levels, ordered and never confused

```text
L0 Commanded          a command was issued and accepted        (today)
L1 Actuated           the device confirms the actuator ran     (today, implicitly)
L2 FlowObserved       a witness saw water move
L3 FlowMeasured       a witness measured how much              ← V1 target
L4 ResponseCorroborated  soil or pot weight later agreed       (today, late)
```

`EvidenceLevel` is an ordered enum and every `DeliveryRecord` carries the highest
level it actually reached. The rule that makes it worth having: **a lower level
is never reported as a higher one, and a missing witness produces L1, never L3
with a guessed number.** A device with no witness keeps working exactly as it
does today and says so — `DeliveredUnverified`, not `DeliveredVerified`.

L4 is deliberately *not* required for a delivery to be `Verified`. Hydraulic
delivery and biological response are different claims on different timescales,
and blocking the first on the second would make every dose provisional for an
hour. L4 corroborates, and its absence downgrades nothing.

### 4. The V1 witness is a reservoir scale, not a flow meter

This is the load-bearing hardware decision, and it is a direct answer to
PRD 110's open question.

A peristaltic dosing pump of the class this project specifies moves on the order
of **0.5 L/min** — the simulator's default `ml_per_second` of 8.2 works out at
0.49 L/min, and the real figure is TBD until M11-004 measures it, which is
exactly the point: it is a fraction of a litre per minute either way.
Inexpensive Hall-effect turbine flow meters are specified from
1 L/min upward; the small-bore variants start around 0.3 L/min and are least
accurate at the bottom of their range, which is exactly where every dose this
system delivers lives. A turbine meter would be measuring in its worst decade,
in a line that also has to start and stop for eight seconds at a time, where the
startup and shutdown transients are a substantial fraction of the whole event.

A **load cell under the reservoir** measures the same physical quantity by
subtraction, and does so better here:

- 1 g of water is 1 ml, so it measures **volume directly** rather than
  integrating a rate over a duration.
- It has **no minimum flow rate**. Its resolution limit is grams, not litres per
  minute, and a 40 ml dose is a 40 g step.
- It is **the hardware this project already has**. `MeasurementKind::PotWeight`
  exists, an HX711-class load cell is already in the hardware guide and already
  has a `tare` command (protocol §5.9), and the firmware already has a scale
  driver class. The new sensor is the same part in a different place.
- It sits **outside the wetted path**, so it cannot itself block, clog, or leak
  — which a turbine meter plumbed inline can.

What it can prove, and what it cannot, stated plainly:

| Signal | Proves | Does not prove |
|---|---|---|
| Reservoir scale | a measured volume left the reservoir | that it reached the pot |
| Pot scale (exists) | a measured mass arrived in the pot | which line it came from |
| Both together | volume left *and* arrived — the strongest evidence available | that it reached the root zone rather than running down the inside of the pot |
| Tank level (M11-005) | the reservoir is not empty | any delivery; it is a precondition |
| Leak sensor (M11-006) | water is where it should not be | anything about the intended path |
| Pump current | the motor drew current, so it is not open-circuit | that any liquid moved — a dry peristaltic head draws normal current |
| Soil response (L4) | the plant's substrate got wetter, eventually | anything in time to stop a running pump |

**V1 requires exactly one new part: one load cell and one amplifier, under the
reservoir.** The pot scale is optional and already supported. Pump-current
sensing is documented as a Level-1 corroborator and is explicitly not V1 — it
is the signal most likely to be mistaken for delivery evidence, and a dry
peristaltic head is the case it gets wrong.

An **inline flow meter remains a first-class future witness**, which is why the
abstraction is `DeliveryWitness` with `ReservoirScale` as its first
implementation rather than a scale-shaped API with a flow meter bolted on later.
Larger deployments with real flow rates — the greenhouse topology in
[deployment-model.md](../architecture/deployment-model.md) §6 — are where a
turbine meter starts being the right instrument.

### 5. Unknown is an outcome, not a gap to be filled

The taxonomy has three families, and the third is the point of the whole design:

```text
delivered:  DeliveredVerified   DeliveredUnverified   PartialDelivery
faulted:    NoFlow  UnexpectedFlow  OverDelivery  FlowSensorInvalid
            TankEmpty  LeakDuringDelivery  PumpTimeout
unknown:    OutcomeUnknown { reason }
```

`OutcomeUnknown` is what a device losing power mid-dose produces, what a network
partition during actuation produces, and what an unreconcilable replay produces.
It is never resolved to `NoFlow` by timeout and never to zero delivery by
convenience. **This is SAFETY-023.**

### 6. Budget arithmetic is not loosened. One rule is tightened.

`budget::credited_ml` keeps every rule it has: `completed` charges what was
delivered or, failing that, the request; `rejected` charges nothing;
`interrupted`, `failed`, and an unrecognised status charge the full request.

One change, and it only ever charges **more**:

> When both an `estimated_ml` and a `measured_ml` exist for the same actuation,
> the budget is charged `max(estimated_ml, measured_ml)`.

A witness that reports *less* than calibration therefore buys no extra budget,
which closes the obvious attack on the feature: a broken or drifting sensor
reading low must not become a licence to water more. A witness reporting *more*
— a valve that stayed open, an over-delivery — charges the larger, real number.

`OutcomeUnknown` charges the full `effective_ml` and holds the plant, which is
what `interrupted` already does and what SAFETY-016's reconciliation hold
already does. **Nothing about the rolling cap, the cooldown, the cycle dose
limit, or any firmware ceiling changes.**

### 7. Immediate physical safety stays in firmware

Latency decides ownership. A witness observation that must stop a pump within a
second cannot make a round trip to the edge, so:

- **The device** owns the delivery execution state machine, the no-flow startup
  timeout, the target-volume stop, the residual-flow check after shutdown, and
  the unexpected-flow detector. It already owns the independent run guard
  (M11-002), the hardware watchdog, and the latched pump fault (M11-003); this
  extends that layer rather than adding one beside it.
- **The edge** owns durable attempt records, reconciliation, budget accounting,
  lockouts, maintenance state, the audit trail, and every explanation.

A device with no witness runs today's path unchanged. A witness is an additional
veto, never a new permission: no witness observation can start a pump, extend a
run, raise a clamp, or satisfy a gate step.

### 8. Idempotency is unchanged, because `command_id` already carries it

`command_id` is the dedup key on the wire (protocol §6), the primary key of
`commands`, and the unique key of `command_results` and of `watering_events`.
The device's dedup ring re-publishes the **stored** result for a repeat
`command_id` and MUST NOT actuate (§5.8 step 1). `command.result.ack` retires a
result only after the edge commits.

Verified Watering adds no new identity. A `DeliveryRecord` is keyed by
`command_id`, one row per actuation attempt, so a replayed result updates the
same row and cannot create a second attempt. **Querying an attempt and
authorising an actuation are different operations on different topics**, and
this ADR adds nothing that blurs them: there is no "re-run", no "verify again",
and no edge→device message that could cause a pump to move.

### 9. Failure does not retry

An actuation that ends in `NoFlow` does not retry. A blocked tube, a lost prime,
and an empty line all look identical from outside, and the naive response —
dose again, harder — is how a reservoir ends up on the floor. `NoFlow` stops the
cycle, sets an explicit-clear lockout, and moves the actuator to a maintenance
state that needs a person. This is the same reasoning M11-003 already applies to
a latched pump fault, and the same reasoning `no_delivery_detected` applies two
doses later; Verified Watering simply reaches the conclusion on the first dose
instead of the third.

### 10. The adaptive model is a consumer, not a partner

M15's `DoseResponseObservation` already carries `verified: bool` and weights an
unverified observation at half, precisely so this milestone can arrive later and
improve it without a redesign. When M16 exists, `measured_ml` is what a
dose-response observation regresses against, and `verified` becomes true.

**Neither milestone depends on the other.** M15 works with `estimated_ml` and
says so; M16 is worth building for a system that never gains an adaptive model,
because "did my plant get water?" is a question every operator asks and no
amount of learning answers.

## Alternatives considered

**Inline turbine flow meter as the V1 witness.** The obvious choice and the
wrong one at this scale, for the reasons in §4. It stays supported as a future
`DeliveryWitness` implementation for higher-flow deployments.

**Pump-current sensing as the V1 witness.** Cheap, non-invasive, and already
half-present in any driver design. Rejected as the *primary* witness because it
measures the motor, not the water: a peristaltic head running dry, a
disconnected outlet tube, and a normal dose are indistinguishable by current.
It is genuinely useful for the failure it does detect — an open-circuit or
stalled motor — and is documented as an optional L1 corroborator.

**Rely on the pot scale alone.** Attractive: no new hardware at all, and it
measures the end that matters. Rejected as the sole witness because a pot scale
is not present on most plants, is disturbed by anything touching the pot, cannot
distinguish delivered water from a watering can, and — decisively — cannot
observe water that left the reservoir and went somewhere else, which is the
failure with the largest downside. It is an excellent *second* witness and is
used as one.

**Infer delivery from tank level.** The tank sensor already exists, so this is
free. Rejected: a float switch is binary (F-110-24), and even an ultrasonic
level is far too coarse to resolve a 40 ml step in a several-litre reservoir. It
remains a precondition check, not evidence.

**Make `no_delivery_detected` stricter — fire after one dose.** Free, no
hardware. Rejected: it would trade the current false-negative for a large
false-positive rate, because soil near field capacity legitimately shows no rise
after one dose, which is the exact case the two-signal rule was built to
tolerate. It also still cannot fire until after water has been pumped.

**Add a `CommandStatus` variant such as `unverified`.** Rejected in favour of an
optional `delivery` object. A new status is additive only because receivers
decode unknown values to `Unknown`, and `Unknown` currently charges the full
request and creates no watering event — so an older edge would *silently stop
creating watering events* for successful doses from a newer device. An optional
sub-object reaches an older edge as an ignored field, which is the behaviour the
versioning policy actually promises.

**Put verification in the cloud.** Rejected without argument on
[ADR-003](003-edge-first-ownership-model.md) grounds. A verification that needs
the internet is a verification that stops working during exactly the outage an
operator most wants it during.

## Consequences

- The system can distinguish "commanded", "actuated", "measured", and "unknown",
  and can say which one it means. Today it says "completed" for all four.
- One new part per node: a load cell and amplifier under the reservoir. The pot
  scale becomes materially more useful because it is now the second half of a
  pair.
- A no-flow condition is caught on the **first** dose instead of after the
  second, before a second dose is pumped at whatever the tube is pointing at.
- Two new safety invariants, **SAFETY-023** and **SAFETY-024**, and a new
  explicit-clear lockout for each.
- `watering_deliveries` and a bounded `delivery_observations` table; migration
  `0003`. `watering_events` keeps its meaning — a claim that water reached the
  plant — and gains no columns.
- The firmware grows a delivery execution state machine and a second scale
  driver instance. Both sit inside the layer M11 already established.
- HIL gains a stage. Blocking a tube and disconnecting a tube become **required
  gates** rather than things that happen to a deployed system.
- A device without a witness is fully supported, forever, and reports
  `DeliveredUnverified` rather than a silent claim.

## Risks

**A trusted-but-wrong witness.** A drifting or badly tared reservoir scale
produces confident wrong volumes. Bounded structurally rather than
statistically: the budget charges `max(estimated, measured)` so a low-reading
witness grants nothing, a witness disagreeing with calibration beyond a
threshold degrades to `FlowSensorInvalid` and L1 rather than asserting a number,
and calibration carries a version and an age that the outcome records.

**A reservoir scale is disturbed by handling.** Refilling the reservoir, leaning
on the shelf, or a cat is a step change that is not a dose. Mitigated by only
attributing steps inside an authorised actuation window, by requiring the step's
sign and rough magnitude to be plausible, and by treating an implausible step as
`FlowSensorInvalid`. It is not eliminated, and a shared reservoir across several
plants (M13-003) makes it harder — which is why M16 depends on M11 and not on
M13, and why the multi-plant reservoir case is stated as an open question rather
than solved here.

**Verification becoming a precondition for watering.** The tempting next step is
"refuse to water without a witness", which would break every existing
deployment and every monitoring-only plant. The design forbids it: absence of a
witness is L1 and normal, and no gate step reads a witness.

**Scope creep into hardware redesign.** One load cell. The temptation is a
pressure sensor, a normally-closed solenoid, a second cutoff relay, and a valve
position switch. Those are documented in PRD 160 as the **desirable production
architecture** for a later milestone, and deliberately kept out of V1, where the
existing hardware fail-off (gate pull-down, independent cutoff, run guard,
watchdog) is already the layer that stops a pump.

**Two features arriving as one.** Verified Watering and the adaptive model share
a vocabulary and would be easy to merge into a single subsystem nobody can
review. They are separate milestones, with separate PRDs, separate invariants,
and no dependency in either direction — and M15-005's non-goals already say so.
