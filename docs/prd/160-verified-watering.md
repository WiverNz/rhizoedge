# PRD 160 — Verified Watering

**Milestone:** M16 · **Status:** PLANNED · **Depends on:** M11

> **Added 2026-09-01.** Governed by
> [ADR-020](../adr/020-verified-watering-and-delivery-evidence.md) and bounded by
> [SAFETY-023](../architecture/safety-invariants.md) and
> [SAFETY-024](../architecture/safety-invariants.md).
>
> It supersedes two deferrals in
> [PRD 110](110-real-pump-and-safety-hardware.md): §Future work sent
> "flow-meter verification" to M14, which is documentation-only and was never
> going to build it; and §Open questions 3 asked whether a flow meter is worth
> adding early. **The answer is no, and the reasoning in that question is why** —
> at the fraction of a litre per minute a dosing pump of this class delivers, a
> turbine meter is being read in its worst decade. The witness is a load cell
> under the reservoir instead (ADR-020 §4).
>
> **Depends on M11, not M13.** It needs a real pump and the HIL bench; it needs
> neither the UI nor multi-plant scale. It is independent of **M15** in both
> directions.

## Summary

Replace the system's central assumption — that a pump energised for a computed
duration delivered a computed volume — with an explicit model of what is
actually known about each actuation: what was requested, what was authorised,
what was commanded, what the firmware clamped it to, what calibration estimates
was pumped, and what a sensor measured leaving the reservoir.

Add a physical witness that measures delivered volume, a device-side execution
state machine that can stop a dose that is not delivering, an outcome vocabulary
in which **unknown is a first-class answer**, and an audit trail that lets an
operator answer "did my plant actually get water?" with something more precise
than yes.

## Problem

`pump_runtime × ml_per_second = delivered_ml` is the only evidence of delivery
this system has, and it is an assumption dressed as a measurement.

`ml_per_second` is honestly derived — five timed runs into a measuring cup, σ
under 5 %, dated (F-110-11…F-110-14). It is then used to assert a physical fact
about every subsequent dose, for months, through tubing that hardens, a pump
head that loses prime, and a line that may not be connected to anything.

Between the MOSFET and the root zone, nothing is observed. Each of these
produces a `completed` result carrying a `delivered_ml` that is credited to the
plant's rolling budget as though water arrived:

- the reservoir ran dry between the tank check and the dose
- the tube is blocked, kinked, perished, or has fallen out of the pot
- the pump lost its prime and is moving air
- the line is partially occluded and delivers 60 % of calibration
- the tubing has hardened over six months and delivers 70 %
- an anti-siphon valve is stuck shut
- the relay clicks and the pump does nothing

The existing mitigations are real and all *indirect and late*.
`no_delivery_detected` needs **two** unresponsive doses and a soil or weight
signal that may legitimately not move; it is a claim about the plant, hours
later, and it cannot fire until water has already been pumped twice at whatever
the tube is pointing at. The device gate checks tank, leak, and calibration
*before* actuation and nothing at all *during* it.

And one failure has no representation whatever: **water moving when nothing
authorised it** — a siphon through a tube left below the waterline, a valve
stuck open, a pump driven by a shorted MOSFET. It is not a leak, because the
tray may be dry. It is not a watering, because no command exists. Today it is
invisible until the reservoir is empty.

## Goals

1. An explicit `DeliveryOutcome` taxonomy with delivered, faulted, and **unknown**
   families, replacing a `completed` that means four different things.
2. Six separately recorded dose values, so no step in the chain silently
   overwrites another.
3. An ordered `EvidenceLevel`, so weaker evidence is never reported as stronger.
4. A `DeliveryWitness` abstraction, with a **reservoir load cell** as its V1
   implementation and an inline flow meter as a future one.
5. A device-side execution state machine that detects no-flow at startup, stops
   at target volume, and checks that flow stops after shutdown.
6. Unexpected and continued flow treated as a high-severity fault
   (**SAFETY-024**).
7. Unknown outcomes that are never resolved to zero delivery (**SAFETY-023**).
8. A durable per-attempt audit trail keyed by the existing `command_id`, adding
   no new identity and no new way to authorise an actuation.
9. Explanations precise enough that "verified 38.7 ml", "actuation confirmed but
   no witness fitted", and "unknown — the device restarted mid-dose" are
   different answers in the domain model, not different sentences in a UI.

## Non-goals

- **A flow meter in V1.** ADR-020 §4. The abstraction supports one; the hardware
  recommendation does not include one.
- **Pump-current sensing.** Documented as an optional L1 corroborator; not V1,
  because a dry peristaltic head draws normal current and it is the signal most
  likely to be mistaken for delivery evidence.
- **Requiring verification to water.** A device with no witness keeps working
  exactly as it does today and reports `DeliveredUnverified`. No gate step reads
  a witness.
- **Any automatic retry of a failed delivery.** ADR-020 §9.
- **Loosening any budget, cap, cooldown, clamp, or firmware ceiling.** One rule
  is *tightened* (`max(estimated, measured)`); nothing is relaxed.
- **Soil-response confirmation as a precondition.** L4 corroborates and its
  absence downgrades nothing.
- **New hardware safety architecture.** The desirable production layering is
  documented in §Hardware hard stop; V1 adds no relay, valve, or pressure
  sensor.
- **Multi-plant shared reservoir attribution.** M13-003's shared reservoir makes
  the witness ambiguous; recorded as an open question, not solved here.
- **The adaptive model.** [PRD 150](150-per-plant-adaptive-water-model.md) is a
  consumer and is not a dependency in either direction.
- **A UI.** [PRD 120](120-rust-ui.md) builds the screen.

## User/system flows

**A verified dose.** The edge issues 30 ml. The device gate passes, the record
is persisted, the reservoir scale is read as the pre-dose baseline, and the pump
starts. Within `FLOW_START_TIMEOUT_MS` the reservoir mass begins falling; at the
target the pump stops; after `FLOW_SETTLE_MS` the mass is stable. The result
carries `status: "completed"` and a `delivery` object: `measured_ml: 28.4`,
`estimated_ml: 30.0`, `evidence: "flow_measured"`, `outcome:
"delivered_verified"`. The edge charges `max(30.0, 28.4) = 30.0` to the budget
and records both.

**A blocked tube.** The pump starts and the reservoir mass does not move. At
`FLOW_START_TIMEOUT_MS` the device stops the pump itself, publishes
`status: "failed"` with `outcome: "no_flow"` and `measured_ml: 0.0`, and latches
the actuator into a maintenance state. The edge charges the full `effective_ml`
conservatively, sets an explicit-clear `NoFlow` lockout, and **does not retry**.
The operator sees "no water moved — check the tube" on the first dose rather
than the third.

**A device lost mid-dose.** The pump is running when the device loses power. The
edge sees nothing. The command's TTL expires; the plant is held. On the next
boot the device finds its in-flight record, reports `interrupted`, and — if the
witness survived the reboot with a usable baseline — attaches whatever partial
`measured_ml` it can defend, or `outcome: "outcome_unknown"` if it cannot. The
edge charges the full `effective_ml`, records `OutcomeUnknown`, and never
converts it to zero.

**Continued flow.** The pump is commanded off and the reservoir keeps losing
mass. The device asserts the pump-off path, latches a fault, publishes a
high-severity event, and refuses further commands until reboot. The edge locks
the plant with an explicit-clear `UnexpectedFlow` lockout. **SAFETY-024.**

**Unauthorised flow.** No command is in flight and the reservoir mass falls
steadily — a siphon. The device publishes the same fault class; the edge locks
every plant bound to that actuator.

**No witness fitted.** Everything above is skipped. The result carries no
`delivery` object, the edge records `EvidenceLevel::Actuated` and
`DeliveredUnverified`, and the operator is told the delivery is unverified
rather than told it succeeded.

## Functional requirements

| ID | Requirement |
|---|---|
| F-160-01 | `rhizo_domain::delivery` holds `DeliveryOutcome`, `EvidenceLevel`, `HydraulicEvidence`, `FlowObservation`, and `DeliveryRecord`; it is pure and reads no clock |
| F-160-02 | `EvidenceLevel` is ordered: `Commanded` < `Actuated` < `FlowObserved` < `FlowMeasured` < `ResponseCorroborated`, and a record carries the highest level actually reached |
| F-160-03 | A missing, failed, or implausible witness yields `Actuated`, never `FlowMeasured` with an inferred number |
| F-160-04 | Six dose values are stored separately: `requested_ml`, `authorized_ml`, `commanded_ml`, `effective_ml`, `estimated_ml`, `measured_ml` |
| F-160-05 | `DeliveryOutcome` has three families — delivered, faulted, and unknown — and no `success: bool` anywhere in the model |
| F-160-06 | `OutcomeUnknown` carries a typed reason and is never resolved to zero delivery by timeout, restart, reconciliation failure, or convenience (**SAFETY-023**) |
| F-160-07 | The `DeliveryWitness` trait exposes a monotonic cumulative delivered volume and a health state; `ReservoirScale` is its V1 implementation |
| F-160-08 | A witness observation may only refuse, stop, or annotate an actuation; it may never start a pump, extend a run, raise a clamp, or satisfy a device gate step |
| F-160-09 | The device records a pre-dose witness baseline **before** actuation and after the existing NVS in-flight write (step 13 stays first) |
| F-160-10 | No observed flow within `FLOW_START_TIMEOUT_MS` stops the pump and yields `NoFlow` |
| F-160-11 | Reaching the target measured volume stops the pump, independently of, and never later than, the calibrated run duration |
| F-160-12 | Continued flow beyond `FLOW_SETTLE_MS` after shutdown, or any flow with no authorised actuation, is a high-severity fault (**SAFETY-024**) |
| F-160-13 | Measured volume above `OVER_DELIVERY_FACTOR × effective_ml` yields `OverDelivery` and latches the actuator |
| F-160-14 | Measured volume below `PARTIAL_DELIVERY_FRACTION × effective_ml` yields `PartialDelivery` |
| F-160-15 | A witness reading that is non-finite, negative-cumulative, or beyond `MAX_PLAUSIBLE_FLOW_ML_S` yields `FlowSensorInvalid`, degrades the record to `Actuated`, and never produces a volume |
| F-160-16 | Witness calibration carries a version and a date; both are recorded on every `DeliveryRecord` |
| F-160-17 | Stale calibration beyond `CALIBRATION_MAX_AGE_DAYS` degrades the evidence level rather than invalidating the delivery |
| F-160-18 | Wire changes are additive within v1: two `MeasurementKind` variants, one optional `delivery` object on `command.result`, two optional `actuator.state` fields, three `RejectReason` variants, two device `EventKind` values. No new topic, no removed field, no retention or QoS change |
| F-160-19 | An edge that receives no `delivery` object behaves exactly as it does today |
| F-160-20 | A `DeliveryRecord` is keyed by `command_id`; a replayed result updates the same row and can never create a second attempt |
| F-160-21 | No message added by this milestone can cause an actuator to move; querying an attempt and authorising an actuation stay distinct operations |
| F-160-22 | `budget::credited_ml` keeps every existing rule, and charges `max(estimated_ml, measured_ml)` when both exist |
| F-160-23 | `OutcomeUnknown` charges the full `effective_ml` and holds the plant under the existing reconciliation rules |
| F-160-24 | `NoFlow`, `UnexpectedFlow`, `OverDelivery`, and a latched witness fault each set an explicit-clear lockout; none auto-clears |
| F-160-25 | No failed delivery is retried automatically, in any mode, including offline autonomous |
| F-160-26 | An actuator carries a health state — `Healthy`, `Degraded`, `NeedsInspection`, `Locked` — derived from its delivery history, reusing the existing device/plant state surfaces rather than a parallel status system |
| F-160-27 | Raw witness samples are retained for a bounded window; the derived `DeliveryRecord` is durable audit data and is never pruned by the retention worker |
| F-160-28 | `GET /api/v1/plants/{id}/waterings/{command_id}` returns the full attempt: six doses, evidence level, outcome, timings, calibration version, and reconciliation status |
| F-160-29 | Every outcome and fault is a typed value with a stable code; prose is produced only in the API layer |
| F-160-30 | An offline autonomous dose produces the same `DeliveryRecord` and the same outcomes, buffered and replayed through the existing event machinery |
| F-160-31 | Metrics cover attempts, verified and unverified deliveries, each fault class, unknown outcomes, requested-versus-measured error, witness failures, lockouts, and reconciliations |
| F-160-32 | The simulator implements a witness and the delivery fault set, so every scenario runs with no hardware |
| F-160-33 | HIL stages for blocked tube, disconnected tube, empty reservoir, restricted flow, and disconnect-mid-dose are **required gates**, not optional checks |

## Interfaces

**Domain (`rhizo-domain`), new module `delivery`:**

```text
delivery::EvidenceLevel      Commanded | Actuated | FlowObserved
                             | FlowMeasured | ResponseCorroborated
delivery::DeliveryOutcome    DeliveredVerified | DeliveredUnverified
                             | PartialDelivery | NoFlow | UnexpectedFlow
                             | OverDelivery | FlowSensorInvalid | TankEmpty
                             | LeakDuringDelivery | PumpTimeout
                             | OutcomeUnknown { reason: UnknownReason }
                             | SafetyRejected { reason: RejectReason }
delivery::UnknownReason      DeviceLostDuringActuation | DeviceRestarted
                             | WitnessBaselineLost | ResultNeverReceived
                             | ReconciliationIncomplete
delivery::HydraulicEvidence  { level, estimated_ml, measured_ml,
                               started_at, stopped_at, settle_ok,
                               calibration_version, witness_health }
delivery::FlowObservation    { at, cumulative_ml, rate_ml_s, valid }
delivery::DeliveryRecord     the six doses + evidence + outcome + timings
delivery::classify(&HydraulicEvidence, &DoseLadder) -> DeliveryOutcome
delivery::credited_ml(&DeliveryRecord) -> f32     (delegates to budget)
```

**Firmware (`firmware/esp32-node`):**

```text
trait DeliveryWitness {
    fn cumulative_ml(&mut self) -> Option<f32>;   // monotonic; None = unusable
    fn health(&self) -> WitnessHealth;
}
struct ReservoirScale<S: Scale>       // V1
struct NullWitness                    // no witness fitted; always L1
struct FlowMeter<P: PulseCounter>     // future, not V1
```

**MQTT (all additive within v1):**

```text
MeasurementKind::ReservoirWeight   gram,   0.0 – 100000.0
MeasurementKind::FlowRate          ml_s,   0.0 – 1000.0   (reserved; no V1 producer)

command.result.data.delivery  (optional object)
  { measured_ml, estimated_ml, evidence, outcome,
    started_at_ms, stopped_at_ms, settle_ok, calibration_version }

actuator.state.data           (optional fields)
  { witness_health, last_measured_ml }

RejectReason  += witness_faulted | unexpected_flow | actuator_maintenance
EventKind     += delivery.fault | flow.unexpected
```

**HTTP:**

```text
GET /api/v1/plants/{id}/waterings                 (gains outcome + evidence)
GET /api/v1/plants/{id}/waterings/{command_id}    (the full attempt)
GET /api/v1/devices/{id}/actuators                (health + witness state)
POST /api/v1/devices/{id}/actuators/{aid}/clear   (explicit maintenance clear)
```

The clear endpoint carries no override, force, or bypass semantics: it clears a
*latched fault record* after a person has inspected the hardware, and it cannot
cause an actuation.

## Data model

Three layers, deliberately separate.

**The request ledger — exists, unchanged.** `commands` (`command_id`,
`requested_ml`, `mode`, `issued_at`, `expires_at`, `status`) and
`command_results`. No columns added.

**The delivery attempt — new, durable, one row per actuation.**

```sql
watering_deliveries(
  command_id TEXT PRIMARY KEY REFERENCES commands(command_id),
  plant_id, device_id, actuator_id,
  requested_ml, authorized_ml, commanded_ml, effective_ml,
  estimated_ml, measured_ml,
  evidence_level TEXT NOT NULL,
  outcome TEXT NOT NULL,
  unknown_reason TEXT,
  actuator_started_at, actuator_stopped_at, duration_ms,
  settle_ok INTEGER,
  witness_health TEXT, calibration_version TEXT, calibrated_at INTEGER,
  firmware_version TEXT,
  reconciliation TEXT NOT NULL,     -- pending | complete | unresolvable
  credited_ml REAL NOT NULL,
  fault_json TEXT, created_at, updated_at)
```

Keyed by `command_id`, which is why a replay updates rather than inserts
(F-160-20). It is written for **every** actuation attempt including `NoFlow`,
which is why it is not merged into `watering_events` — that table means *water
reached the plant*, `creates_watering_event` is what decides it, and a no-flow
attempt must not create one.

**Raw observations — bounded, prunable.**

```sql
delivery_observations(
  id, command_id, at, cumulative_ml, rate_ml_s, valid)
```

High-frequency and diagnostic. Retention keeps a bounded window per attempt and
a bounded total; the `DeliveryRecord` above is what survives.

**Actuator health — new, one row per actuator.**

```sql
actuator_health(
  device_id, actuator_id, state TEXT NOT NULL,   -- healthy | degraded
                                                 -- | needs_inspection | locked
  since, consecutive_no_flow, last_outcome, last_fault_at,
  cleared_by, cleared_at, PRIMARY KEY(device_id, actuator_id))
```

**Migration** `migrations/edge/0003_verified_watering.sql`, forward from
`0002`. The single-baseline regime already ended with M15-002; if M16 is
executed before M15 it ends here instead, and the issue says so.

## State model

**Delivery execution, owned by the device:**

```text
                   ┌──────────► SafetyRejected (gate step 1–12)
   Requested ──► Accepted ──► BaselineTaken ──► Actuating
                                                   │
                        ┌──────────────────────────┼───────────────┐
                        ▼                          ▼               ▼
                  no flow by                  target reached   leak / tank /
                  FLOW_START_TIMEOUT               │           guard / timeout
                        │                          ▼               │
                        ▼                       Stopping ◄──────────┘
                     NoFlow                        │
                                                   ▼
                                                Settling
                                          ┌────────┴────────┐
                                          ▼                 ▼
                                   flow stopped      flow continues
                                          │                 │
                                          ▼                 ▼
                                      Delivered       UnexpectedFlow
                                          │             (latched)
                                          ▼
                            classify → DeliveredVerified
                                      | DeliveredUnverified
                                      | PartialDelivery | OverDelivery
```

Any transition interrupted by power loss, reset, or an unusable baseline exits
to `OutcomeUnknown` with a typed reason — never to `NoFlow`, which is a
*measured* condition.

**Reconciliation, owned by the edge:**

```text
pending ──► complete        a result arrived and committed
        └─► unresolvable    TTL expired, device replayed, and no result exists
                            for this command_id — records OutcomeUnknown and
                            keeps the conservative charge
```

**Actuator health:**

```text
Healthy ──► Degraded ──► NeedsInspection ──► Locked
   ▲                                            │
   └────────── explicit operator clear ─────────┘
```

`Degraded` on one `PartialDelivery` or a stale calibration; `NeedsInspection` on
repeated partials or one `NoFlow`; `Locked` on `UnexpectedFlow`,
`OverDelivery`, or a latched witness or pump fault. Nothing auto-clears.

## Failure modes

| Failure | Immediate action | Outcome | Budget |
|---|---|---|---|
| No flow at startup | device stops the pump | `NoFlow` | full `effective_ml` |
| Partial delivery | dose completes | `PartialDelivery` | `max(estimated, measured)` |
| Over-delivery | device stops at the ceiling and latches | `OverDelivery` | `max(estimated, measured)` |
| Flow continues after stop | assert pump-off, latch, high-severity event | `UnexpectedFlow` | full `effective_ml` |
| Flow with no command | latch, high-severity event, lock every bound plant | `UnexpectedFlow` | n/a; no attempt exists |
| Witness absent | none; normal operation | `DeliveredUnverified` at L1 | today's rule |
| Witness invalid or implausible | degrade to L1, do not stop the dose | `FlowSensorInvalid` | today's rule |
| Calibration stale | degrade the evidence level | unchanged | unchanged |
| Tank empties mid-dose | existing device tank path | `TankEmpty` | full `effective_ml` |
| Leak asserts mid-dose | existing 1 s stop (F-110-33) | `LeakDuringDelivery` | full `effective_ml` |
| Run guard fires | existing independent guard | `PumpTimeout` | full `effective_ml` |
| Device disconnects mid-dose | device continues under its own limits | `OutcomeUnknown` | full `effective_ml`, plant held |
| Device restarts mid-dose | existing in-flight NVS record | `OutcomeUnknown` | full `effective_ml` |
| Edge restarts mid-dose | none; the device is autonomous | resolved on reconnect | unchanged |
| Result never arrives | TTL expiry, then reconciliation | `OutcomeUnknown` | full `effective_ml` |
| Duplicate result replayed | update the same row | unchanged | charged once |

## Safety implications

Two new invariants, and a strict ordering that does not move.

**SAFETY-023 — an unknown delivery outcome is never credited as zero.** The
tempting simplification is that a dose with no result delivered nothing, because
it makes the state machine tidy. It is also the single most dangerous assumption
available: a device that pumped 40 ml and then lost power would free 40 ml of
budget and water again immediately.

**SAFETY-024 — water movement that no command authorised is a fault, not an
observation.** Distinct from SAFETY-003: a leak is water where it should not be,
detected by the leak sensor in the tray. Unexpected flow is water moving through
the intended path with no authorisation, and the tray can be perfectly dry while
the reservoir empties into the pot.

Ordering, unchanged from today except where marked:

```text
policy dose (static, or M15's clamped proposal)
  → edge safety_gate  (SAFETY-003/004/005/012/016/017/018)
  → machine::evaluate (cooldown, cycle limit, SAFETY-006 rolling cap)
  → persist before publish (SAFETY-001/010)
  → device gate steps 1–12 (SAFETY-002/007/012)
  → NVS in-flight write, step 13 (SAFETY-011)
  → witness baseline                                    ← new, after step 13
  → actuate, step 14
  → execution state machine: startup, target, settle    ← new, veto only
  → independent run guard + watchdog (SAFETY-007)       ← unchanged, still last
  → pump
```

The witness enters **only** as an additional veto after the gate has already
passed. It reads no gate step, satisfies no gate step, and cannot lengthen a
run. The independent run guard and the hardware watchdog remain the last word,
exactly as M11-002 established, and F-160-11's target-volume stop can only stop
a pump **earlier** than calibration would have.

Specifically:

- **SAFETY-006** — the cap is still derived from rows. The one arithmetic change
  charges `max(estimated, measured)`, which can only increase a charge. A
  low-reading witness therefore buys no budget.
- **SAFETY-007** — every firmware ceiling and clamp is untouched; the witness
  bounds are additional and stricter.
- **SAFETY-012** — a missing, failed, or implausible witness degrades evidence
  and never grants anything. No `unwrap_or_default` on a witness reading, no
  catch-all arm on an outcome match.
- **SAFETY-016** — reconciliation still holds the plant across an unreconciled
  seam; `OutcomeUnknown` is one more way to be unreconciled, not a way out.
- **SAFETY-001/-010** — identity is still `command_id`; no new identity, and no
  new message that can cause a pump to move.
- **SAFETY-013/-014** — an offline autonomous dose gets the same execution state
  machine and the same outcomes, and the device's own budget still bounds it.

There is **no override, force, or bypass parameter** anywhere in this milestone.
The maintenance-clear endpoint clears a latched fault record after inspection and
cannot actuate.

## Observability

```text
rhizo_watering_attempts_total          counter, labelled by mode
rhizo_watering_outcomes_total          counter, labelled by outcome
rhizo_watering_evidence_level          counter, labelled by level
rhizo_delivery_error_ml                histogram, effective minus measured
rhizo_delivery_unknown_total           counter, labelled by unknown reason
rhizo_witness_health                   gauge, labelled by health
rhizo_witness_failures_total           counter, labelled by cause
rhizo_unexpected_flow_total            counter
rhizo_actuator_health                  gauge, labelled by state
rhizo_delivery_reconciliations_total   counter, labelled by resolution
rhizo_calibration_age_days             gauge
rhizo_delivery_verify_latency          histogram, stop to classification
```

No label carries a plant, device, sensor, or actuator identifier, following the
existing catalogue. `Metrics::new()` is a process-wide `OnceLock` singleton, so
tests take `api::health::gauge_lock()` or assert on deltas.

Logs: one structured `warn` per fault outcome carrying the six doses and the
evidence level; one `error` per `UnexpectedFlow`; one `info` per reconciliation
resolution. Device events `delivery.fault` and `flow.unexpected` carry the same
detail through the buffered-event path so an isolated device's faults survive
and replay.

These are also the raw material for the product-facing reliability figures — the
share of operations physically verified, median requested-versus-delivered
error, no-flow incidents, uncertain outcomes, automatic-watering availability.
None of those is computed in this milestone; the requirement is only that every
one of them is derivable from what is stored, without a schema change.

## Testing strategy

**Unit, in `rhizo-domain`** — `classify` over every evidence shape; the ordering
of `EvidenceLevel`; the six-dose ladder; `credited_ml` for each outcome; the
`max(estimated, measured)` rule in both directions; every non-finite, negative,
and implausible witness value; and every `OutcomeUnknown` reason.

**Property tests**, following `budget`'s example:

- No witness input, however adversarial, produces `NaN`, an infinity, or a
  credited volume below what the same result would be charged today without a
  witness.
- `credited_ml` is monotonic in measured volume and never below the conservative
  floor.
- Replaying any sequence of results for one `command_id`, in any order and with
  arbitrary duplication, yields one `DeliveryRecord` and one charge.
- No sequence of witness observations can extend an actuation beyond the
  calibrated run duration or the firmware run ceiling.

**Firmware and simulator** — the full state machine: verified delivery,
under-delivery within and beyond tolerance, over-delivery, no flow, delayed
startup inside and outside the timeout, clean settle, continued flow,
unauthorised flow, tank empty mid-dose, leak mid-dose, witness absent, witness
invalid, disconnect before actuation, disconnect during actuation, restart
during actuation, and baseline lost across a reboot.

**Edge integration** — reconciliation after reconnect; a duplicate MQTT result;
a duplicate `command_id`; an edge restart mid-dose; a TTL expiry with no result;
budget totals after each uncertain outcome; actuator health transitions; and the
maintenance clear.

**Safety tests**, named per the convention:

- `safety_023_unknown_outcome_is_never_credited_as_zero`
- `safety_023_a_missing_result_never_becomes_a_zero_delivery`
- `safety_023_reconciliation_failure_keeps_the_conservative_charge`
- `safety_024_continued_flow_after_stop_latches_and_locks_out`
- `safety_024_flow_with_no_authorised_actuation_locks_every_bound_plant`
- `safety_024_unexpected_flow_does_not_auto_clear`

## Acceptance criteria

- [ ] A verified dose reports six distinct doses, an evidence level of
      `flow_measured`, and `delivered_verified`.
- [ ] A device with no witness reports `delivered_unverified` at `actuated`, and
      its behaviour is byte-identical to pre-M16 for every existing test.
- [ ] A blocked tube produces `no_flow` on the **first** dose, stops the pump,
      and is not retried.
- [ ] Continued flow after shutdown latches the actuator and locks the plant with
      an explicit-clear lockout.
- [ ] Flow with no authorised actuation locks every plant bound to the actuator.
- [ ] A device lost mid-actuation produces `outcome_unknown`, charges the full
      `effective_ml`, and is never resolved to zero.
- [ ] A duplicate or replayed result updates one row and charges once.
- [ ] No witness input produces `NaN`, an infinity, or a charge below today's.
- [ ] Over-delivery charges the measured volume, not the commanded one.
- [ ] An invalid witness degrades to `actuated` and never asserts a volume.
- [ ] `cargo test safety_` passes, including every `safety_023_*` and
      `safety_024_*` test.
- [ ] The wire changes are additive: pre-M16 fixtures still decode, and an edge
      ignoring the `delivery` object behaves as it does today.
- [ ] Both bare-metal targets still build.
- [ ] The HIL stages for blocked tube, disconnected tube, empty reservoir,
      restricted flow, and disconnect-mid-dose all pass and are recorded.
- [ ] `cargo run -p rhizo-docscheck` is clean.

## Dependencies

- **M11** — a real pump, the independent run guard, the latched pump fault, the
  tank and leak adapters, and the HIL bench. M16-001 depends on M11-014.
- **M9** — the firmware workspace, the hardware trait pattern, the in-flight NVS
  record, and the buffered event ring.
- **M2** — the simulator, which gets the witness and the delivery faults so the
  scenarios run with no hardware.
- **M6** — the command lifecycle, the safety gate, and reconciliation.
- **M15** — *not* a dependency, in either direction. M15's
  `DoseResponseObservation` already carries `verified` and weights unverified
  observations lower, so M16 improves it on arrival with no change to either
  design.
- **M13** — not a dependency. A shared reservoir (M13-003) makes the witness
  ambiguous and is an open question, not a prerequisite.

## Hardware hard stop

The existing layering is already correct and V1 does not change it:

```text
edge decision → device gate → firmware execution → independent run guard
              → hardware watchdog → gate pull-down → independent pump cutoff → pump
```

The gate is pulled down in hardware so an undriven pin is pump-off (F-110-03),
the run guard is on a separate task from MQTT (F-110-05), the watchdog leaves the
pump off (F-110-06), and a physical cutoff is required during all testing
(F-110-42). Nothing in M16 sits between the guard and the pump.

The **desirable production architecture**, documented here and deliberately not
built in V1: a normally-closed solenoid downstream of the pump, so a stuck pump
cannot siphon; an independent hardware one-shot timer that cuts pump power
regardless of firmware state; and valve position feedback. Each is a real
improvement and each is a hardware milestone of its own. V1's answer to a stuck
pump is the latched fault, the cutoff, and a person — which is honest, and is
what the current bill of materials supports.

## Open questions

1. **Reservoir scale disturbance.** Refilling, leaning on the shelf, or an
   animal produces a mass step that is not a dose. Attribution windows and
   plausibility bounds mitigate it; whether a settling filter or a second
   baseline read is needed is a bench question, not a design one.
2. **Shared reservoirs.** M13-003 introduces one reservoir for several plants. A
   single witness cannot attribute concurrent doses, and the likely answer is
   that concurrent actuation on a shared reservoir is refused — but that is a
   M13 decision, and M16 must not pre-empt it.
3. **The `FLOW_START_TIMEOUT_MS` value.** Long enough for a peristaltic head to
   prime and a compliant tube to pressurise; short enough that a blocked line
   does not run for seconds. Measured on the bench in M16's HIL stage, not
   guessed here.
4. **Whether `estimated_ml` should decay in trust as calibration ages.**
   F-160-17 degrades the evidence level; whether it should also widen the
   partial-delivery tolerance is open.
5. **Battery cost of a second load cell.** An HX711 is not free and a battery
   node samples it during every dose. ADR-018's energy budget is measured in
   M10-012; whether a battery node should ship a witness at all is answered
   there, not here.
6. **Whether a verified zero should ever reduce the conservative charge.** Today
   `NoFlow` charges the full `effective_ml` even though the witness says nothing
   moved. That is deliberately wasteful of budget and deliberately safe; the case
   for trusting a verified zero is real and is exactly the kind of loosening that
   needs its own ADR.

## Future work

- Inline flow meter as a second `DeliveryWitness`, for greenhouse-scale flows
  where a turbine meter is in its accurate range
  ([PRD 140](140-field-readiness.md)).
- Pump-current sensing as an L1 corroborator, distinguishing an open-circuit
  motor from a dry head.
- Normally-closed solenoid and an independent hardware one-shot cutoff.
- Distinguishing "delivered but the plant did not respond" — water reached the
  pot but not the root zone, or the probe is outside the wetting front — from
  today's `NoDeliveryDetected`, which collapses both.
- Drift detection: calibrated `ml_per_second` against measured delivery over
  months, which is how hardening peristaltic tubing announces itself.
- Product reliability reporting built on §Observability's counters.
- Feeding `measured_ml` into [PRD 150](150-per-plant-adaptive-water-model.md)'s
  dose-response estimator as a verified observation.
