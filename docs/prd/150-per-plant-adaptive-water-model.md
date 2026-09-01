# PRD 150 — Per-Plant Adaptive Water Model

**Milestone:** M15 · **Status:** PLANNED · **Depends on:** M13

> **Added 2026-09-01.** The first milestone whose subject is *inference* rather
> than mechanism. It is deliberately placed after M13: the estimators need real
> probes (M10), real pumps that report delivered volume (M11), and months of a
> real deployment's history (M13) before any of their numbers mean anything.
> Nothing in it touches the wire, the firmware, or the cloud contract, and
> nothing in it is enabled by default.
>
> Governed by [ADR-019](../adr/019-per-plant-adaptive-water-model.md) and bounded
> by [SAFETY-022](../architecture/safety-invariants.md).

## Summary

Teach the Edge the water behaviour of one *specific physical setup* — this
plant, in this pot, in this substrate, with the probe at this depth, in this
room, this season — and use it to answer questions the static policy cannot:
when will this plant be dry, what does a millilitre actually do here, and is
today's behaviour normal for this plant.

The model is deterministic, statistical, replayable, explainable in a sentence,
and **subordinate to every existing safety mechanism**. Its only authoritative
effect in this milestone is to ask for a **smaller** dose than the operator
configured.

## Problem

`AutomationPolicy` holds one dose, one target, one cooldown, one absorption
window, for every plant that shares a profile or a species preset. Those numbers
are a starting point produced by a catalogue that has never seen the pot.

The consequences are concrete:

- A dose that is right for a 12 cm plastic pot floods a shallow bonsai tray and
  barely wets a 25 cm terracotta pot. The operator discovers this by watching
  the plant, and fixes it by editing a number.
- The system already measures the moisture rise a dose produces, reduces it to
  "responded / did not respond" through `recovery_delta_vwc`, and throws the
  magnitude away. Every dose is therefore the first dose.
- `trend::fit` computes a slope that nothing consumes, so "your fern will be dry
  on Thursday" is unanswerable even though the data supports it.
- "Faster than normal" has no per-plant definition, so a pot that has become
  root-bound, cracked, or been moved into direct sun looks exactly like a pot
  that is fine.

Everything needed to fix this is already recorded and already durable —
`measurements`, `watering_events`, `commands`, and the manual-watering detector.
What is missing is somewhere to keep the conclusion, and a rule for how much
authority a conclusion may have.

## Goals

1. A per-plant `HydrationModel`: a learned drying rate, a learned dose response,
   and a confidence, scoped to a **model epoch**.
2. Deterministic estimators in `rhizo-domain` — pure, clock-free, replayable,
   and explainable without statistics vocabulary.
3. A durable observation ledger that survives the 90-day raw-measurement prune,
   so the model does not depend on unbounded raw retention.
4. An explicit epoch model, so a repot, a substrate change, or a moved probe
   ends a model rather than quietly poisoning it.
5. Cold-start and low-confidence behaviour that is **exactly today's behaviour**.
6. A four-stage rollout — `disabled`, `shadow`, `advisory`, `adaptive` — with
   shadow mode recording what the model would have done against what the static
   policy did.
7. An explanation surface that answers "why 28 ml?" with numbers, not adjectives.
8. Hooks that a later anomaly-detection milestone can consume without redesign.

## Non-goals

- **Machine learning of any kind.** ROADMAP §7 keeps it on the not-doing list
  and this milestone does not move it. See ADR-019 §Alternatives.
- **Anomaly detection.** M15-014 lands the *hooks* and the recorded prediction
  error. Detecting, classifying, and alerting on anomalies is later work.
- **Raising a dose above `automation.dose_ml`.** Forbidden by SAFETY-022, in
  this milestone and until an ADR says otherwise.
- **Learning any other parameter.** Cooldown, absorption, `dry_confirm`,
  `max_daily_ml`, `target_min_vwc`, staleness thresholds, and every lockout rule
  stay authored. The model produces a dose proposal and advisory timing, and
  nothing else.
- **Learning on the device.** No protocol change, no firmware change, no learned
  value inside an `OfflinePolicy`.
- **A cloud representation.** The ADR-005 catalogue is not amended.
- **Weather, forecast, or external environmental inputs.** The model uses what
  the plant's own bindings report. Weather stays where
  [PRD 140](140-field-readiness.md) put it.
- **Cross-plant or cross-population learning.** One model per plant per epoch.
  Borrowing a neighbour's coefficients is a later question.
- **A UI.** The API carries everything a screen would need;
  [PRD 120](120-rust-ui.md) builds the screen.

## User/system flows

**Cold start.** A plant is created, sensors are bound, `adaptive_mode` defaults
to `disabled`. Nothing changes; `GET /plants/{id}/hydration-model` answers
`learning_state: "cold_start"` with an empty model and an explicit
`observations_needed` count.

**Learning in shadow.** The operator sets `adaptive_mode: "shadow"`. Each control
tick, the refresh extracts any newly completed drying segments and dose
responses into the observation ledger and re-estimates. Watering decisions are
untouched; every tick that produced a static answer also records the adaptive
proposal beside it. The operator can compare the two at any time.

**Reaching confidence.** After enough clean drying segments and dose responses
in the current epoch, confidence reaches `Medium`. In `advisory` mode the
recommendation surface starts carrying the adaptive block —
`expected_dry_at`, `estimated_response_per_ml`, `proposed_ml` — while
`recommended_ml` remains the static dose.

**Adaptive watering.** The operator sets `adaptive_mode: "adaptive"`. The plant
crosses its target, the debounce elapses, and the control loop computes a
proposal of 31 ml, clamps it to `[min_effective_ml, dose_ml] = [5, 40]`, and
offers **31 ml** to the unchanged `evaluate`. The gate, the rolling cap, the
cooldown, the cycle-dose limit, the TTL, and the device's own firmware ceiling
all run exactly as they do today. The command is issued for 31 ml, or refused
for one of today's reasons.

**Repotting.** The operator repots and calls
`POST /plants/{id}/hydration-model/reset` with `reason: "repotted"`. A new epoch
opens, the model returns to `cold_start`, prior observations are marked
superseded and kept, and behaviour returns to the static policy until confidence
rebuilds.

**A sensor is moved.** The operator rebinds or replaces the control sensor. The
binding change opens a new epoch automatically — no operator action, because the
one thing worse than a reset nobody asked for is a model nobody knows is stale.

**Going away for the weekend.** The operator opens the plant and reads
"expected dry threshold: 2.6 days, confidence high" — the question the static
system could never answer.

## Functional requirements

| ID | Requirement |
|---|---|
| F-150-01 | `HydrationModel` lives in `rhizo_domain::hydration`, is pure, reads no clock, and performs no I/O |
| F-150-02 | Every observation, estimate, and model row is scoped to a `(plant_id, epoch)` pair; no estimator ever reads two epochs |
| F-150-03 | A drying segment is a run of validated `soil_moisture` samples containing no watering event of any mode, no active lockout, no sampling gap wider than the plant's `max_sample_age`, and at least `MIN_SEGMENT_SAMPLES` samples spanning at least `MIN_SEGMENT_HOURS` |
| F-150-04 | `DryingRate` is a weighted least-squares slope over segment observations, reported in %VWC/day, with a residual spread and the count and age of the observations used |
| F-150-05 | A dose-response observation requires a `completed` watering event, a validated pre-dose reading within `max_sample_age` before it, and a validated post-dose peak inside the plant's `absorption` window |
| F-150-06 | `DoseResponse` is a through-origin weighted regression of moisture rise on **delivered** millilitres, reported in %VWC/ml, with a residual spread and observation count |
| F-150-07 | Where `delivered_ml` is absent, `requested_ml` is used and the observation is flagged `unverified`, weighted lower, and shown as such in the explanation |
| F-150-08 | Observations are weighted by recency with a configured half-life; weights are deterministic functions of observation age, never of wall-clock time at estimation |
| F-150-09 | Outliers are rejected by a median-absolute-deviation rule against the current epoch's observations, and a rejected observation is retained and marked, never deleted |
| F-150-10 | Every estimator answers `None` rather than a fabricated value when its minimum observation count, minimum span, or maximum residual spread is not met |
| F-150-11 | No estimator, projection, or proposal may produce `NaN` or an infinity; a non-finite intermediate answers `None` |
| F-150-12 | `ModelConfidence` is `ColdStart`, `Low`, `Medium`, or `High`, computed from observation count, residual spread, observation age, epoch age, and sensor stability |
| F-150-13 | Confidence below `Medium` produces no proposal and no `expected_dry_at`; the static policy answers unchanged |
| F-150-14 | `expected_dry_at` projects the current validated reading forward at the learned drying rate to `target_min_vwc`, and is absent when the drying rate is absent, non-negative, or the reading is stale |
| F-150-15 | A dose proposal is the volume that returns the plant from its current reading to a configured `target_recovery_vwc`, at the learned response, and is `None` when either estimate is absent |
| F-150-16 | The proposal is clamped into `[min_effective_ml, automation.dose_ml]` **before** it reaches `evaluate`; the clamp is applied in exactly one place (**SAFETY-022**) |
| F-150-17 | `evaluate`, `safety_gate`, `budget::dose_fits`, `credited_ml`, and every lockout rule are unchanged and run on the clamped value |
| F-150-18 | `adaptive_mode` is per plant, defaults to `disabled`, and only `adaptive` may change a volume |
| F-150-19 | In `shadow` and `advisory` mode the model computes and persists a proposal that no watering path reads |
| F-150-20 | Every tick that evaluates a plant with a model records the static answer, the adaptive proposal, the clamped value, and which one was used |
| F-150-21 | An epoch change is recorded with its trigger, its instant, and the operator or subsystem that caused it, and emits a `plant_events` row |
| F-150-22 | A binding change to a `control`-role sensor, an actuator rebinding, a device replacement on a bound sensor, or a calibration-reference change opens a new epoch automatically |
| F-150-23 | Repot, substrate change, pot change, and plant replacement open a new epoch through the explicit reset endpoint |
| F-150-24 | A measurement outage longer than `EPOCH_STALE_DAYS` for the control sensor opens a new epoch on the next sample |
| F-150-25 | Observations from a superseded epoch are retained, marked, and never read by an estimator |
| F-150-26 | The model is rebuildable from the observation ledger alone; rebuilding a plant's model from its persisted observations reproduces the stored estimates exactly |
| F-150-27 | Restart never changes a recommendation: the model is loaded from storage, not recomputed from a different window |
| F-150-28 | The observation ledger is bounded per `(plant, epoch)` by row count, and pruning drops the oldest and lowest-weighted observations first; ledger tables in the retention sense (`watering_events`, `commands`, `device_events`) are untouched |
| F-150-29 | `GET /api/v1/plants/{id}/hydration-model` returns the model, its epoch, its confidence, its inputs' counts and ages, and its learning state |
| F-150-30 | The recommendation response carries an `adaptive` block when a model exists, including the proposal before and after the clamp and the reason for any difference |
| F-150-31 | Every adaptive contribution to a decision is expressible as typed `Reason` values; prose is produced only in the API layer |
| F-150-32 | Predicted and observed dose response are both recorded per watering event, so prediction error is a queryable series |
| F-150-33 | Metrics cover enabled plants by mode, confidence distribution, model refreshes, epoch changes, proposals made, proposals clamped, proposals refused by the gate, and static-policy fallbacks |
| F-150-34 | Nothing in this milestone changes `rhizo-mqtt-contract`, `rhizo-policy`, the MQTT protocol, or the ADR-005 event catalogue |

## Interfaces

**Domain (`rhizo-domain`), new module `hydration`:**

```text
hydration::DryingSegment          one accepted drying observation
hydration::DoseResponseObservation one accepted watering observation
hydration::DryingRate             { vwc_per_day, spread, n, newest_age }
hydration::DoseResponse           { vwc_per_ml, spread, n, unverified_n }
hydration::HydrationModel         { epoch, drying, response, confidence, updated_at }
hydration::ModelConfidence        ColdStart | Low | Medium | High
hydration::EpochReason            Repotted | SubstrateChanged | PotChanged
                                  | PlantReplaced | SensorChanged | DeviceChanged
                                  | CalibrationChanged | MeasurementGap | OperatorReset
hydration::estimate_drying_rate(&[DryingSegment], &EstimatorConfig) -> Option<DryingRate>
hydration::estimate_dose_response(&[DoseResponseObservation], &EstimatorConfig) -> Option<DoseResponse>
hydration::confidence(&HydrationInputs) -> ModelConfidence
hydration::expected_dry_at(now, vwc, target_min, &DryingRate) -> Option<DateTime<Utc>>
hydration::propose_dose(vwc, target_recovery, &DoseResponse) -> Option<f32>
hydration::clamp_proposal(proposal, min_effective_ml, static_dose_ml) -> ClampedDose
```

**HTTP (`http-api-boundaries.md`):**

```text
GET  /api/v1/plants/{id}/hydration-model
GET  /api/v1/plants/{id}/hydration-model/observations?epoch=&limit=
POST /api/v1/plants/{id}/hydration-model/reset      { reason, note? }
PUT  /api/v1/plants/{id}/adaptive-mode              { mode }
GET  /api/v1/plants/{id}/recommendation             (gains an `adaptive` block)
```

`POST .../reset` is not a watering path and carries no override, force, or bypass
semantics: it discards inference, never a limit.

**MQTT:** unchanged. **Cloud:** unchanged.

## Data model

Three layers, kept separate on purpose.

**Raw observations — already exist, nothing added.** `measurements`,
`watering_events`, `commands`, `plant_events`. Raw measurements are pruned at 90
days and the model must not depend on them beyond that; that is what the derived
layer is for.

**Derived observations — new, durable, bounded.**

```sql
plant_hydration_epochs(
  plant_id, epoch, opened_at, reason, note, opened_by, closed_at,
  PRIMARY KEY(plant_id, epoch))

plant_drying_segments(
  segment_id, plant_id, epoch, started_at, ended_at, start_vwc, end_vwc,
  sample_count, slope_vwc_per_day, residual, mean_ambient_c, mean_illuminance,
  status,            -- accepted | rejected_outlier | superseded
  created_at)

plant_dose_responses(
  observation_id, plant_id, epoch, watering_event_id, dosed_at,
  requested_ml, delivered_ml, verified, pre_vwc, peak_vwc, peak_at,
  rise_vwc, vwc_per_ml, predicted_rise_vwc, mean_ambient_c,
  status, created_at)
```

**Learned state — one row per plant, rebuildable from the layer above.**

```sql
plant_hydration_model(
  plant_id PRIMARY KEY, epoch, model_version,
  drying_vwc_per_day, drying_spread, drying_n,
  response_vwc_per_ml, response_spread, response_n, response_unverified_n,
  confidence, updated_at, updated_from_observation_id)

plant_adaptive_decisions(
  id, plant_id, epoch, evaluated_at, static_ml, proposed_ml, clamped_ml,
  applied TEXT,        -- static | adaptive
  confidence, reason_json)

plants ... ADD COLUMN adaptive_mode TEXT NOT NULL DEFAULT 'disabled'
```

`model_version` is the estimator's own schema version. A change to estimator
semantics raises it and forces a recompute from the observation ledger rather
than reinterpreting numbers produced by different arithmetic.

**Environmental context** is carried on the observation, not on the model:
`mean_ambient_c` and `mean_illuminance` are recorded per segment where the plant
has those bindings, and are **not** used by the V1 estimators. Recording them now
costs two columns; reconstructing them later is impossible once raw measurements
are pruned.

**Migration.** `migrations/edge/0002_adaptive_model.sql`. This is the first
migration after the canonical baseline, and it is the trigger
`canonical_baseline_contains_the_final_schema` was written to produce (see
ADR-019 §Consequences and M15-002).

## First deterministic model

The V1 arithmetic in full. It is deliberately small enough to state here, which
is the point: an operator can be told what it does, and a reviewer can check it.

**Weighting.** Every observation carries a weight
`w = 0.5 ^ (age_days / OBSERVATION_HALF_LIFE_DAYS)`, multiplied for a drying
segment by its sample count and for a dose response by `1.0` when `delivered_ml`
was reported and `0.5` when only `requested_ml` was. `age_days` is measured from
the `now` the caller passes, never from a clock read inside the estimator.

**Outlier rejection.** Over the current epoch's accepted observations, take the
median of the per-observation values (`slope_vwc_per_day`, or `vwc_per_ml`), the
median absolute deviation from it, and reject anything further than
`MAD_OUTLIER_K * MAD`. Median-based rather than mean-based, because the single
observation this rule exists to survive — a repot that escaped the epoch
machinery — is exactly the one that drags a mean.

**Drying rate.** Weighted least squares of `end_vwc - start_vwc` against segment
duration in days, through the origin, over the surviving observations. Answers
`None` below `MIN_SEGMENTS` observations, above the residual-spread ceiling, or
on any non-finite intermediate.

**Dose response.** Weighted least squares of `rise_vwc` against `delivered_ml`,
**through the origin** — zero millilitres produce zero rise, and fitting an
intercept invites the model to claim a plant gets wetter from being asked
politely. Answers `None` below `MIN_RESPONSES` observations, on the same two
other conditions.

**Time to dry.**
`expected_dry_at = now + (vwc - target_min_vwc) / vwc_per_day` days, and `None`
whenever `vwc_per_day >= 0`, the reading is stale or invalid, or the result is
not finite.

**Dose proposal.**
`proposed_ml = (target_recovery_vwc - vwc) / vwc_per_ml`, then
`clamped_ml = clamp(proposed_ml, min_effective_ml, automation.dose_ml)` —
SAFETY-022, applied in one place.

**Environmental adjustment.** None in V1. `mean_ambient_c` and
`mean_illuminance` are recorded per observation so a later version can use them;
using them now would add two coefficients to a fit that does not yet have enough
observations for one.

**Fallback.** Any `None` at any step means the static policy answers, unchanged
and unannotated apart from the explanation saying which estimate was missing.

**Worked example**, and the numbers PRD 150's tests reproduce. Six clean drying
segments over three weeks averaging −1.8 %VWC/day with a spread of 0.3; four
dose responses averaging 0.12 %VWC/ml with a spread of 0.02; current reading
24.0 %VWC; `target_min_vwc` 22.0; `target_recovery_vwc` 30.0; `dose_ml` 40;
`min_effective_ml` 5.

```text
expected_dry_at  = now + (24.0 - 22.0) / 1.8  = now + 1.11 days
proposed_ml      =       (30.0 - 24.0) / 0.12 = 50.0 ml
clamped_ml       = clamp(50.0, 5, 40)         = 40.0 ml   ← clamped at the static dose
confidence       = Medium (4 responses, spread 0.02, newest 3 days old)
```

The clamp firing in the *worked* example is not an accident. It is the common
case for a plant whose configured dose is conservative, and it is what "the
model may only ask for less" means in practice.

## State model

**Learning state**, derived and reported, never stored as a separate truth:

```text
cold_start  ──(enough observations)──▶ low ──▶ medium ──▶ high
     ▲                                  │        │         │
     └──────────(epoch change)──────────┴────────┴─────────┘
```

**Adaptive mode**, per plant, operator-set, defaulting to `disabled`:

```text
disabled ──▶ shadow ──▶ advisory ──▶ adaptive
   ▲            │           │            │
   └────────────┴───────────┴────────────┘   (any mode may return to disabled)
```

Mode and learning state are independent. A plant in `adaptive` mode with
`cold_start` confidence behaves exactly like a plant in `disabled` mode — that
is F-150-13, and it is the cold-start safety property.

## Failure modes

| Failure | Behaviour |
|---|---|
| No observations yet | `cold_start`; static policy answers; no proposal |
| Too few observations | Confidence `Low`; no proposal |
| Residual spread too wide | Estimator answers `None`; no proposal |
| Non-finite intermediate | Estimator answers `None`; a metric counts it |
| Latest reading stale or invalid | No `expected_dry_at`, no proposal; the gate refuses automatic watering as it does today |
| Manual watering inside a segment | Segment discarded, not fitted |
| Undetected small manual watering | Biases the drying rate slower — the conservative direction |
| Sensor replaced or moved | Epoch opens automatically; model returns to `cold_start` |
| Sensor stuck | `sensor_stuck_state` already suppresses the samples; segments starve and confidence decays |
| Long measurement outage | Epoch opens on the next sample after `EPOCH_STALE_DAYS` |
| Model row missing or undecodable | Treated as `cold_start`; a warning is logged; the static policy answers |
| Observation ledger at its cap | Oldest, lowest-weighted observations pruned; estimates continue |
| Proposal exceeds the static dose | Clamped down to `dose_ml`, difference recorded and explained |
| Proposal below `min_effective_ml` | Clamped up to `min_effective_ml`, which is itself `<= dose_ml` |
| Proposal would cross the rolling cap | Refused by `budget::dose_fits`, exactly as a static dose is |
| Edge restart mid-learning | Model loaded from storage; recommendations identical across the restart |

## Safety implications

**SAFETY-022 is the new invariant** and the reason the rest of this section is
short: an adaptive value may only ever narrow, never widen. The clamp is applied
in one place, before `evaluate`, and the ordering is

```text
observations → estimators → confidence gate → proposal
   → clamp to [min_effective_ml, automation.dose_ml]     ← SAFETY-022
   → safety_gate (SAFETY-003/004/005/012/016/017/018)
   → machine::evaluate (cooldown, cycle dose limit, SAFETY-006 rolling cap)
   → command issue (persist before publish, TTL, SAFETY-001/002/010)
   → device-side validate_water_command (SAFETY-007/014 firmware ceilings)
   → pump
```

Every existing invariant keeps its existing enforcement point. Specifically:

- **SAFETY-005** — the model never relaxes staleness, never supplies a reading,
  and never substitutes a projection for a measurement. A projection is not an
  observation, and `expected_dry_at` is never an input to the gate.
- **SAFETY-006** — the cap is still derived from rows and still *refuses* rather
  than clamping. A smaller adaptive dose may fit where a larger static one did
  not; the 24-hour total remains bounded, which is what the invariant states.
- **SAFETY-012** — every estimator answers `Option`, and `None` means the static
  policy answers. There is no `unwrap_or_default` on a learned value and no
  catch-all arm on a confidence match.
- **SAFETY-013/014** — the device's offline behaviour is untouched; no learned
  number reaches an `OfflinePolicy`.
- **SAFETY-018** — a monitoring-only plant can hold a full hydration model and
  still has no actuation path. `expected_dry_at` on a plant with no pump is one
  of the feature's better uses.
- **SAFETY-007** — the firmware ceiling is unchanged and unreachable from here;
  the clamp's upper bound is already below it.

There is **no override, force, or bypass parameter** anywhere in this milestone,
and the reset endpoint discards inference rather than a limit.

## Observability

Metrics, following the existing naming and bounded-cardinality conventions:

```text
rhizo_adaptive_plants                  gauge, labelled by mode
rhizo_adaptive_confidence_plants       gauge, labelled by confidence
rhizo_adaptive_model_refreshes_total   counter, labelled by outcome
rhizo_adaptive_observations_total      counter, labelled by kind and status
rhizo_adaptive_epoch_changes_total     counter, labelled by reason
rhizo_adaptive_proposals_total         counter, labelled by outcome
                                       (none | clamped_low | clamped_high | used | shadow)
rhizo_adaptive_static_fallback_total   counter, labelled by cause
rhizo_adaptive_prediction_error_vwc    histogram, predicted minus observed rise
rhizo_adaptive_refresh_duration        histogram
```

Logs: one structured event per epoch change (reason, trigger, prior estimates),
one per model refresh at `debug`, one at `warn` when an estimator refuses for a
non-finite intermediate. Traces follow the existing control-tick span.

Plant events: an epoch change writes a `plant_events` row so it appears in the
plant's own history next to lockouts and threshold crossings.

## Testing strategy

Deterministic first, and the `TestClock` makes every case reproducible.

**Unit, in `rhizo-domain`** — cold start, insufficient observations, a clean
linear drying history, a noisy one, injected outliers, repeated watering events,
recency weighting, non-finite inputs, an absent estimate, confidence transitions
in both directions, and the clamp in all four regions (below the floor, inside,
above the ceiling, and a ceiling below the floor).

**Property tests**, following `budget`'s example:

- A clamped proposal is always in `[min_effective_ml, dose_ml]` and always
  finite, for arbitrary observation histories including adversarial ones.
- No estimator output is `NaN` or infinite for any input, including empty,
  single-point, zero-span, and extreme-value histories.
- Replaying the same observation sequence in the same order always produces the
  same model, and a model rebuilt from the ledger equals the incrementally
  maintained one.
- An adaptive decision never issues a volume a static decision with the same
  inputs would not have been permitted to issue.

**Integration, in `edge-controller`** — restart mid-learning changes no
recommendation; a binding change opens an epoch; an epoch change makes prior
observations unreadable to the estimators; shadow mode issues nothing; a
detected manual watering discards the segment containing it; a stale reading
suppresses the proposal; the gate refuses an adaptive dose exactly as it refuses
a static one.

**Safety tests**, named `safety_022_*` per the convention, covering: a proposal
above `dose_ml` is clamped; a proposal cannot cross the rolling cap; a proposal
cannot shorten a cooldown; a model cannot clear a lockout; and a corrupt or
missing model falls back to the static policy.

**Scenario suite** — M15-014 registers the end-to-end cases in
`docs/testing/failure-scenarios.md` and allocates their identifiers at that
point, since the registry's numbering is contiguous and allocating early would
leave holes.

## Acceptance criteria

- [ ] A plant with no history behaves **identically** to today, byte for byte in
      the recommendation response apart from an explicit `cold_start` block.
- [ ] A plant with a clean synthetic drying history reports a drying rate within
      tolerance of the injected one, and an `expected_dry_at` consistent with it.
- [ ] A plant with clean synthetic dose responses reports a response within
      tolerance of the injected one.
- [ ] Injected outliers do not move either estimate beyond tolerance.
- [ ] Confidence rises with clean observations and falls with age, spread, and an
      epoch change.
- [ ] A proposal above `automation.dose_ml` is clamped to it, and the difference
      is visible in the explanation.
- [ ] A proposal that would cross the rolling cap is refused with today's reason
      and today's status code.
- [ ] Shadow mode records proposals and issues no command; `no_commands_in_shadow`
      passes.
- [ ] Restarting the edge mid-learning produces the same recommendation before
      and after.
- [ ] Rebuilding a model from its persisted observations reproduces the stored
      estimates exactly.
- [ ] An epoch change returns the plant to `cold_start` and no estimator reads a
      superseded observation.
- [ ] `cargo test safety_` passes, including the new `safety_022_*` tests.
- [ ] `rhizo-mqtt-contract`, `rhizo-policy`, the MQTT contract documents, and the
      ADR-005 catalogue are unchanged, and both bare-metal targets still build.
- [ ] `cargo run -p rhizo-docscheck` is clean.

## Dependencies

- **M13** — a real, multi-plant, hardware-verified deployment producing the
  history the estimators need. M15-001 depends on M13-017.
- **M11** — a real pump. `delivered_ml` from real hardware is what makes a dose
  response an observation rather than an assumption.
- **M10** — real probes, and M10-010's gravimetric check, which is the only thing
  that distinguishes probe drift from plant change.
- **M12** — not a dependency. The API is complete without a screen, and the UI
  work to consume it is a later M12 addition rather than a reopening.

## Open questions

1. **Environmental normalisation.** Ambient temperature and illuminance are
   recorded per observation and unused by V1. Whether the second version
   normalises the drying rate against them, or segments the model by season, is
   open — and cannot be answered before a year of one plant's data exists.
2. **`target_recovery_vwc`.** The proposal aims at a recovery target that V1
   derives from `target_min_vwc` plus `recovery_delta_vwc`. Whether that deserves
   to be its own authored field is open; adding a field is easy and removing one
   is not.
3. **Half-life and minimum counts.** `OBSERVATION_HALF_LIFE_DAYS`,
   `MIN_SEGMENTS`, and `MIN_RESPONSES` are starting values, not measurements.
   They need a real deployment to settle, and M15-014 is where they get revisited.
4. **Whether `advisory` and `shadow` should be one mode.** They differ only in
   whether the block is shown to the operator. Keeping them separate lets the
   estimators be validated without an operator acting on numbers nobody has
   checked; merging them later is easy.
5. **Cross-plant priors.** Two identical pots of the same species by the same
   window will learn the same thing twice. Sharing a prior is attractive and is
   deliberately not V1, because the first thing a shared prior does is make one
   plant's fault another plant's dose.
6. **Whether a learned dose should ever reach an `OfflinePolicy`.** Currently no,
   on ADR-015 grounds. This is the single most likely place for the safety
   argument to be re-opened, and it should be re-opened in an ADR or not at all.

## Future work

- Anomaly detection consuming the recorded prediction error (drying much faster
  than baseline, a dose producing no response, a step change in sensor values).
- Seasonal segmentation of the model, once a year of data exists.
- Learned absorption time — already recorded as post-V1 in
  [PRD 060](060-irrigation-control-and-safety.md) and a natural third estimator.
- Consuming verified delivery (flow sensing) as a first-class observation weight.
- A UI screen for the model, its history, and its explanation
  ([PRD 120](120-rust-ui.md)).
- Evapotranspiration from pot-weight trend, already listed in
  [PRD 050](050-plant-model-and-recommendations.md) and a fourth estimator that
  would fit this frame without changing it.
