# ADR-019 — Per-plant adaptive water model

## Status

**Proposed** — 2026-09-01. Contract-free: nothing on the wire changes, and no
firmware work is implied. Entities and estimators in **M15**
([PRD 150](../prd/150-per-plant-adaptive-water-model.md)); nothing is enforced
before M15-012, and M15-012 itself is opt-in per plant.

Amends the *reading* of one requirement without weakening it, and corrects a
citation:

- **F-050-23** — `recommended_ml` is "derived from profile `dose_ml`, never from
  an unbounded computation". A bounded computation whose ceiling **is**
  `dose_ml` satisfies the requirement as written; this ADR states that reading
  explicitly so it is not re-litigated.
- The doc comments on `AutomationPolicy::dose_ml` and
  `IrrigationDecision::IssueDose` say "never a computed volume" and cite
  **F-060-23**, which is the publish-retry requirement and has nothing to do
  with volumes. The requirement they mean is F-050-23. M15-012 corrects the
  citation and the sentence in the change that makes a bounded computed volume
  possible.

Does **not** amend [ADR-006](006-irrigation-state-machine-ownership.md),
[ADR-015](015-device-offline-autonomy.md), or
[ADR-016](016-plant-binding-and-policy-model.md). The edge still owns the
decision, the device still owns the veto, and per-plant configuration is still
authored by an operator.

## Context

Every watering rule in the system is a **static, species-level threshold applied
to an individual pot**. `AutomationPolicy` carries one `target_min_vwc`, one
`dose_ml`, one `cooldown`, one `dry_confirm`, one `absorption`, one
`recovery_delta_vwc`. Those numbers arrive from `PlantProfile` — a template — or
from the species preset catalogue, and they never change again unless a person
edits them.

That model has taken the project as far as it can go, and its limits are visible
in the code itself:

**The same species is not the same plant.** A 12 cm plastic pot of peat by a
south window and a 25 cm terracotta pot of bark-heavy mix in a hallway share a
species preset and behave nothing alike. One dries in two days, the other in
nine. The preset catalogue's own provenance notes say as much; it cannot say
more, because it knows nothing about the pot.

**The dose is a guess that never improves.** `IrrigationDecision::IssueDose`
carries `inputs.automation.dose_ml` verbatim. The system already measures what
that dose does — `pre_dose_soil`, `latest_soil`, and `recovery_delta_vwc` exist
precisely so `delivery_evidence` can ask "did anything happen?" — and then
discards the answer as a boolean. A year of watering events on one plant teaches
the system exactly nothing about how much water that plant needs.

**Timing is reactive only.** `trend::fit` produces a %VWC-per-hour slope over a
six-hour window, reported for operator intuition and consumed by nothing. The
system can say "this plant is dry now"; it cannot say "this plant will be dry on
Thursday", which is the question a person going away for the weekend actually
has.

**"Normal" has no per-plant meaning.** `no_delivery::no_delivery_detected` fires
when two consecutive doses move nothing, which catches a disconnected tube. It
cannot catch a pot that has started drying twice as fast as it did last month —
root-bound, cracked, or moved into the sun — because nothing anywhere records
what "as fast as it did last month" was.

Meanwhile the data needed to fix all four already exists and is already durable:
`measurements` holds 90 days of typed, timestamped, quality-tagged samples with
**edge** receipt times; `watering_events` is an append-only ledger of requested
and delivered volumes that retention never touches; `commands` carries mode and
settlement; `detect::detect_manual_watering` already separates water the system
delivered from water a person delivered, with an attribution window so the two
are never conflated.

What is missing is not data and not sensors. It is a **place to keep what the
data implies about one specific pot**, and a rule for how much authority that
inference is allowed to have.

## Decision

Introduce a **per-plant hydration model**: a deterministic, replayable,
statistical estimate of how one physical plant/pot/substrate/sensor setup loses
and absorbs water, owned by the Edge, stored in SQLite, and **subordinate to
every existing safety mechanism**.

### 1. It is derived state, not configuration

The codebase already separates the two, and the separation is load-bearing:
`measurement_policies` and `offline_policies` are authored; `plant_dry_state`,
`plant_state_current`, `sensor_stuck_state`, and `plant_threshold_state` are
derived and rebuildable. The hydration model belongs to the second family. An
operator never edits a learned coefficient — they reset the epoch, correct the
static policy, or turn the feature off.

### 2. Name: `HydrationModel`, in `rhizo_domain::hydration`

`hydration` names the physical property — how this pot takes on and loses water
— rather than the mechanism. Considered and rejected:

| Candidate | Rejected because |
|---|---|
| `PlantWaterModel` | "water model" reads as plumbing, and `plant_*` prefixes in this codebase name tables, not physics |
| `AdaptivePlantModel` | names the mechanism (adapting), not the subject; and it is not a model *of the plant*, it is a model of one pot's water behaviour |
| `WaterResponseModel` | one of the model's two halves; survives as `DoseResponse` |
| `DryingModel` | the other half; survives as `DryingRate` |
| `PlantWaterProfile` | collides catastrophically with `PlantProfile`, which is a template and the opposite of learned |
| `AIModel`, `MLModel` | there is no ML here, and naming it so would invite some |

### 3. Two estimators, one composed model, no ML

**`DryingRate`** — a weighted least-squares slope over *drying segments*: spans
of validated `soil_moisture` samples with no watering event, no lockout, and no
sampling gap inside them. Reported in %VWC per day with a residual spread.

**`DoseResponse`** — a through-origin weighted regression of observed moisture
rise against delivered millilitres, over completed watering events whose
pre-dose and post-dose readings are both present and fresh. Reported in %VWC per
millilitre.

Both are ordinary arithmetic over `f64`, both are pure functions in
`rhizo-domain`, both read no clock, and both are exactly as testable as
`trend::fit` and `budget::dose_fits` already are. The project's standing
position on machine learning does not change: ROADMAP §7 keeps "Machine
learning" on the not-doing list, and this is not it. A rule an operator can read
in a sentence — "your plant loses 1.8 points a day; 10 ml buys you 1.2" — is
worth more than a better-fitting model nobody can audit, on a system whose
failure mode is a dead plant or a flooded floor.

### 4. Confidence is a gate here, unlike in `recommend`

`Recommendation::confidence` is advisory and decides nothing, deliberately, and
`recommend.rs` carries a test that fails if anyone gates on it. That stays true.
`HydrationModel` carries its **own** `ModelConfidence`, a four-valued enum —
`ColdStart`, `Low`, `Medium`, `High` — and it **does** gate: below `Medium` the
model proposes nothing and the static policy answers unchanged. The two are
different quantities and are deliberately not merged; M15-007 names the
distinction in a test.

### 5. Authority: the model may only ask for **less**

This is the whole safety argument, and it is one sentence:

> An adaptive proposal is clamped into `[min_effective_ml, automation.dose_ml]`
> **before** it reaches `evaluate`, and every existing check then runs on the
> clamped value, unchanged.

Because the clamped value can never exceed `automation.dose_ml`, no gate, cap,
cooldown, lockout, or firmware limit can be reached by adaptive means that a
static dose could not already reach. The model cannot widen anything. This is
**SAFETY-022**.

The corollary is stated plainly because it is a real limitation: if the model
concludes the plant needs *more* than the configured dose, it says so in the
explanation and **the number does not change**. Raising `dose_ml` is an operator
decision and stays one.

### 6. Budget interaction: refusal semantics are not touched

`budget::dose_fits` **refuses** a dose that would cross the 24-hour cap; it does
not clamp to the remainder, and M15 does not teach it to. A clamped adaptive
dose is simply a smaller number offered to the same unchanged function. One
consequence is worth stating rather than discovering: a 12 ml adaptive dose can
fit under a cap that a 40 ml static dose would have crossed. That is not a
loosening — SAFETY-006 bounds the 24-hour **total**, and any dose fitting under
that total is permitted by the invariant's own definition — but it *is* an
observable behaviour change, and M15-012 tests it as such.

### 7. Epochs, not silent re-learning

Every observation belongs to a `ModelEpoch` — an integer, per plant,
monotonically increasing. A repot, substrate change, pot change, sensor move or
replacement, calibration change, plant replacement, device replacement, or a
long measurement outage **opens a new epoch**. Estimators read one epoch and
never mix. Superseded observations are retained and marked, not deleted, so the
history stays auditable and an epoch opened by mistake is diagnosable.

Silent re-learning was rejected explicitly: a decay window wide enough to absorb
a repot is wide enough to blur a real seasonal change, and a system that cannot
say *when* it stopped believing something cannot explain itself.

### 8. Rollout: four modes, per plant, starting at off

`disabled` → `shadow` → `advisory` → `adaptive`. Only `adaptive` changes a
volume, only from M15-012, and the default is `disabled`. Shadow mode records
what the model *would* have said next to what the static policy *did* say, which
is how the estimators get validated on real plants before they touch actuation.

### 9. Cloud gains nothing in V1

The [ADR-005](005-cloud-event-model-and-idempotency.md) catalogue is closed and
is not amended: no kind is added, renamed, or removed. The cloud already
receives `measurement.sample`, `watering.completed`, and `watering.detected`,
which is everything an offline analysis would need. Model state is edge-local,
an epoch change writes a `plant_events` row, and **the cloud remains incapable of
influencing a dose** — the property that would be most tempting to erode here,
since a learned model is exactly the kind of thing someone would want to compute
centrally.

### 10. The device learns nothing

No protocol change, no new topic, no new payload field, no firmware work.
`rhizo-mqtt-contract` and `rhizo-policy` are untouched, so both bare-metal
targets are unaffected. An isolated device keeps evaluating
`rhizo_policy::evaluate_offline` against its statically provisioned
`OfflinePolicy`, exactly as ADR-015 specifies. Pushing a learned dose into an
offline policy is a possible later extension and is deliberately not V1: it
would put a number no human authored inside the one path that runs with no
supervision at all.

## Alternatives considered

**Do nothing; keep static thresholds.** Honest and cheap, and it is the right
answer until real hardware has produced real history — which is why this is M15
and not M6. It stops being the right answer once the ledger exists and the
system is still ignoring it.

**Learn by adjusting `AutomationPolicy` in place.** Tempting: no new tables, and
every consumer benefits at once. Rejected — it destroys the operator's authored
configuration, makes "why is this 47 ml?" unanswerable, gives a learned number
the same authority as a human one, and would leak into `OfflinePolicy` and onto
the device.

**A single blended "smart dose" number.** Rejected: the two estimators answer
different questions, fail independently, and carry different confidence. A plant
with a clean drying history and no watering history should get good *timing* and
no dose change; one number cannot express that.

**Machine learning (regression trees, small nets, online learners).** Rejected
for V1 on four grounds: explainability (a hard requirement, not a nicety),
replayability (the same history must produce the same state), the data volume
available from one pot, and dependency weight in a workspace that must
cross-compile parts of itself to `thumbv7em-none-eabi`. The estimators are
deliberately shaped so a better fit could replace their internals without
touching their signatures.

**Physical soil-water models (van Genuchten, FAO-56 evapotranspiration).**
Rejected: they need substrate parameters, calibrated sensors, and reference
evapotranspiration the system does not have. A per-pot empirical slope needs
none of it and is what an experienced grower actually reasons with.

**Compute the model in the cloud.** Rejected on
[ADR-003](003-edge-first-ownership-model.md) grounds without further argument. A
decision input that lives in the cloud is a decision the cloud can break by being
absent.

## Consequences

- The system can answer "when will this plant be dry?" and "did that dose do
  what it usually does?" — neither of which it can answer today.
- Over-watering shrinks first: the model's only authoritative effect in V1 is to
  ask for *less* than the static dose.
- One new migration (`0002_*.sql`) and the end of the single-baseline schema
  regime. `canonical_baseline_contains_the_final_schema` asserts exactly one
  migration and fails the moment a second appears — deliberately, as the prompt
  to ask whether the first release has happened. By M15 it has (M13-013 ships
  release CI), so M15-002 converts that test into a forward-migration assertion.
  This is the intended trigger, not a surprise.
- A per-plant `adaptive_mode` column and four new derived tables. Retention gains
  bounded, per-epoch row caps.
- Explanation becomes a first-class output. `Reason` gains adaptive variants and
  stays a typed enum; prose stays in the API layer, as today.
- The recommendation surface grows a block, and the UI gains a screen worth
  building (M12 is not blocked and is not reopened).

## Risks

**A confident wrong model.** The mitigation is structural rather than
statistical: confidence gates participation, the clamp bounds the damage to "a
smaller dose than the operator configured", and shadow mode runs first. The
worst realistic outcome of a badly wrong model in `adaptive` mode is
under-watering, which is visible, recoverable, and already surfaced by
`NoDeliveryDetected` and the plant-state API.

**Sensor drift indistinguishable from plant change.** A capacitive probe drifting
looks like a pot drying faster. Partly mitigated by epoch changes on sensor
events and by `sensor_stuck_state`; not eliminated. M10-010's gravimetric check
is the only real answer, and it is why M15 sits after real sensors rather than
before them.

**Manual watering contaminating drying segments.** Already mitigated, and this is
the strongest existing foundation the design leans on: `detect` produces
`detected` watering events with an attribution window, and any segment containing
one is discarded rather than fitted. What remains is manual watering too small to
detect, which biases the drying rate toward "slower" — the conservative
direction, since a slower estimated drying rate delays watering.

**Scope creep into an ML platform.** Mitigated by the milestone boundary, by
ROADMAP §7 keeping ML on the not-doing list, and by M15-014 stopping at *hooks*
for anomaly detection rather than at anomaly detection.

**The learned model becoming load-bearing.** The real long-term risk: once the
model is good, someone will want it to extend a cooldown, widen a budget, or
seed an offline policy. SAFETY-022 exists so that change has to be argued for in
an ADR rather than merged as a refinement.
