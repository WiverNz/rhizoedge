# PRD 050 — Plant Model and Recommendations

**Milestone:** M5 · **Status:** DELIVERED · **Depends on:** M4

> **Revised 2026-08-26.** `PlantProfile` is demoted to a **template**; the
> authoritative per-plant configuration is now bindings plus per-measurement
> policies ([ADR-016](../adr/016-plant-binding-and-policy-model.md)). The
> milestone also authors and validates offline policies. Issues M5-013…M5-016
> were added; M5-001…M5-004 expanded.
>
> Three consequences worth stating plainly:
>
> - **Thresholds belong to the plant, not the sensor.** The same room sensor is
>   "fine" for one plant and "critical" for another, and the old model could not
>   express that.
> - **The actuator is optional.** A plant with no `ActuatorBinding` is a normal
>   monitoring plant — the common case in a real home — and gets telemetry,
>   history, thresholds, warnings, and critical alerts with no actuation path
>   (SAFETY-018).
> - **Warnings are not control conditions.** A critical temperature raises an
>   alert and never waters anything.
>
> **Additional acceptance criteria:** two plants hold different thresholds for
> one shared sensor; editing a profile does **not** rewrite existing plants;
> `POST /water` on an actuator-less plant returns **422**, distinguishable from a
> 409 safety refusal; no threshold crossing of any kind triggers actuation.

> **Extended 2026-08-28 — battery devices.**
> [ADR-018](../adr/018-battery-and-deep-sleep-device-mode.md) adds a device that
> sleeps between samples. M5 gains three issues, M5-019…M5-021, which are
> *device* work in a *plant* milestone and are here for a specific reason: they
> extend the M4-owned registry model, and **M4 is DONE and was not reopened**,
> exactly as M0 was not reopened by the 2026-08-26 pass. M5 is the first
> milestone still open, and every later milestone — M6's pending commands, M9's
> firmware, M12's presentation — depends on this landing first.
>
> - **M5-019** adds the wire surface once, in `rhizo-mqtt-contract`: two battery
>   measurement kinds, the optional `power` blocks, and the `sleeping` offline
>   reason. Entirely additive within v1; no version bump
>   ([versioning-policy.md](../protocol/versioning-policy.md) §1).
> - The **post-M4 battery correction** teaches the registry to tell an announced, bounded sleep from a
>   device that stopped waking — the new **SAFETY-021**.
> - **M5-021** gives the simulator a battery mode, so SCEN-110…SCEN-112 run with
>   no hardware and M9 has a specification to match rather than to invent.
>
> Nothing about the plant model changes. A sleeping device's samples are ordinary
> samples, its thresholds are ordinary thresholds, and **a monitoring-only
> battery plant is a first-class monitoring plant** — the same rule SAFETY-018
> already states, applied to a device class that makes it more common rather than
> less.
>
> **Additional acceptance criteria:** a battery device inside its window derives
> `sleeping`, not `offline`; a device past `overdue_at` derives `isolated` **from
> the timer, with no inbound message**; an announced wake time far in the future
> does not extend the edge's window; an always-on device's liveness behaviour is
> unchanged from M4's.

## Summary

Introduce plants, reusable plant profiles, moisture trends, manual-watering
detection, and an explainable rule-based recommendation engine. The system moves
from "here is telemetry" to "this plant needs water, and here is why" — while
still issuing no commands.

## Problem

Raw moisture is not actionable. 24 % is dry for a fern and fine for a succulent;
24 % falling steadily means something different from 24 % after a watering an
hour ago. Turning readings into a decision needs a profile, a trend, and a
history — and, critically, needs to explain itself, because an operator will not
enable automation they do not understand.

## Goals

1. Plant and plant-profile entities with validation.
2. Moisture trend computation robust to sensor noise.
3. Time-since-last-watering, including watering the system did not perform.
4. Manual-watering detection from moisture and weight step changes.
5. A rule-based recommendation engine with **structured** reasons.
6. Plant state derived and exposed.

## Non-goals

- Issuing any command. M5 recommends; M6 acts. This separation is deliberate:
  it lets the recommendation logic be validated against a real plant for a week
  before anything can pump.
- Machine learning of any kind. Explicitly out of scope for V1.
- Deriving N/P/K from EC. EC is recorded and trended; no nutrient claims are
  made ([PRD 140](140-field-readiness.md)).

## User/system flows

```text
operator creates a profile → creates a plant → attaches it to a device
        ↓
telemetry accumulates → trend computed → recommendation evaluated each tick
        ↓
GET /api/v1/plants/{id}/recommendation
   { "recommendation": "water", "recommended_ml": 40,
     "reasons": [ moisture_below_target, dry_for, last_watering ] }
```

Manual watering detection:

```text
operator waters by hand
   → moisture jumps +20 pp within one interval
   → (and/or) pot weight jumps +350 g
   → watering_event(mode='detected', delivered_ml≈350) recorded
   → time-since-last-watering resets
```

## Functional requirements

### Entities

| ID | Requirement |
|---|---|
| F-050-01 | CRUD for plants; `auto_watering_enabled` defaults to **false** |
| F-050-02 | CRUD for profiles; profiles are reusable across plants |
| F-050-03 | Profile validation **rejects** rather than clamps: `target_min >= target_max`, `dose_ml × max_doses > max_daily_ml`, non-positive intervals, `dose_ml > FIRMWARE_MAX_ML_PER_RUN` |
| F-050-04 | A plant consumes `SensorBinding[]` from one or more devices; one device may serve several plants; an `ActuatorBinding` is optional |
| F-050-05 | Deleting a plant preserves its `watering_events` history |

### Analysis

| ID | Requirement |
|---|---|
| F-050-10 | Moisture trend as a least-squares slope (%VWC/hour) over a configurable window (default 6 h), requiring ≥ 5 valid samples |
| F-050-11 | Trend returns `None` when there is insufficient or too-sparse data — never a slope computed from two points |
| F-050-12 | Dry-duration tracked as continuous time below `target_min`, reset by any valid sample at or above it |
| F-050-13 | Time-since-last-watering considers all modes including `detected` |
| F-050-14 | Manual watering detected on a moisture rise ≥ `detect_moisture_delta` (default 8 pp) between consecutive samples not attributable to a command |
| F-050-15 | Where a scale exists, weight rise ≥ `detect_weight_delta` (default 100 g) also triggers detection and gives a better volume estimate |
| F-050-16 | A detected event is attributed to a command if one completed within the absorption window, preventing double-counting |
| F-050-17 | Stuck-sensor detection: `stuck_sample_count` (default 20) bit-identical readings marks the sensor unhealthy |
| F-050-18 | EC recorded and trended; a rise beyond `ec.warning_high_us_cm` raises a warning event and no more |

### Recommendation

| ID | Requirement |
|---|---|
| F-050-20 | Evaluated per plant on the control tick; result persisted with reasons |
| F-050-21 | `recommendation ∈ water \| no_water \| blocked` |
| F-050-22 | Reasons are typed enum values, never prose strings |
| F-050-23 | `recommended_ml` derived from profile `dose_ml`, never from an unbounded computation |
| F-050-24 | `confidence` reported, and reduced when inputs are sparse, noisy, or partially missing |
| F-050-25 | When the safety gate would block, `recommendation = blocked` with `blocked_by` naming the lockout |
| F-050-26 | Plant state derived and exposed: `Healthy`, `Drying`, `WaterRecommended`, `SensorFault`, `WateringLocked` |

### Rule

```text
recommend water WHEN
      latest sample valid AND fresh
  AND moisture < profile.target_min
  AND dry_duration >= profile.dry_confirm_minutes
  AND time_since_last_watering >= profile.cooldown_hours
  AND safety gate passes
```

Every conjunct that fails contributes a reason, so a `no_water` answer is as
explainable as a `water` one.

## Interfaces

```text
GET    /api/v1/plants
POST   /api/v1/plants
GET    /api/v1/plants/{plant_id}
PATCH  /api/v1/plants/{plant_id}
DELETE /api/v1/plants/{plant_id}
GET    /api/v1/plants/{plant_id}/measurements?from=&to=&resolution=
GET    /api/v1/plants/{plant_id}/recommendation
GET    /api/v1/plants/{plant_id}/watering-events
GET    /api/v1/profiles      POST /api/v1/profiles
GET    /api/v1/profiles/{id} PUT  /api/v1/profiles/{id}
```

```rust
// rhizo-domain
pub fn recommend(inputs: &RecommendationInputs) -> Recommendation;

pub struct Recommendation {
    pub decision: Decision,          // Water | NoWater | Blocked
    pub recommended_ml: Option<f32>,
    pub confidence: f32,
    pub reasons: Vec<Reason>,
    pub blocked_by: Option<LockoutReason>,
}

pub fn moisture_trend(samples: &[SoilSample], window: Duration) -> Option<TrendVwcPerHour>;
pub fn detect_manual_watering(prev: &Sample, cur: &Sample, cfg: &DetectCfg)
    -> Option<DetectedWatering>;
```

### Preset endpoints (M5-017, M5-018)

```text
GET  /presets                      list and search the embedded catalogue
GET  /presets/{preset_id}          one entry, with provenance per value
POST /plants                       optional `preset_id` prefills configuration
POST /plants/{id}/apply-preset     apply to an existing plant; `overwrite`
                                   required if it already has policies, and the
                                   response names every changed field
```

A preset value that violates a profile hard limit is **rejected with 422**, not
clamped: a curated catalogue is an input, not a trusted one. Creating a plant
without `preset_id` behaves exactly as it did before presets existed — the
manual path is not a fallback, it is the same first-class path it always was.

## Data model

Uses `plants`, `plant_profiles`, and `watering_events` from
[ADR-004](../adr/004-sqlite-edge-persistence-model.md), plus `applied_preset_id`
and `applied_catalogue_version` on `plants` (M5-018), which are **provenance
columns only** — nothing reads them to decide anything.

### Plant presets

A **plant preset** is a reusable starting configuration for a species, so that
creating a plant does not begin with an operator inventing a moisture band. It
is a template in the same sense `PlantProfile` is, and it is subject to the same
rule: it is never authoritative runtime state. Applying one writes ordinary
per-plant `MeasurementPolicy` rows through the existing binding and policy model
([ADR-016](../adr/016-plant-binding-and-policy-model.md)) and then stops
mattering. Every value is editable afterwards, and no edit is ever re-derived.

Three constraints define the shape:

- **A preset is not a schedule.** It holds preferences and conditions — a
  soil-moisture band, a light preference, temperature and humidity ranges, pH, a
  suggested dose and cooldown class — and never an interval such as "water every
  2 days". Watering remains a function of measurements and, from M6, the safety
  gate. A timer would be a second actuation authority that no sensor reading and
  no lockout could contradict.
- **Source facts and Rhizo-derived defaults are stored separately.** A figure a
  cited source stated, in that source's own units, is a `SourceFact`. A starting
  value Rhizo interpreted from it is a `RhizoDefault`. An external
  `soil_humidity = 6` on some vendor's 1-10 scale converted to a volumetric
  water content is an interpretation with a guess inside it, and presenting it
  as a measured fact gives a plausible number authority it has not earned.
- **The catalogue is embedded and versioned.** It is compiled into the binary,
  carries a `catalogue_version`, and requires no network and no database. Making
  plant creation the one operation that needs the internet would contradict the
  offline-first premise in the README.

Two further properties are invariants rather than conveniences:

- **Materialisation happens exactly once, and the provenance columns are inert.**
  A preset is applied at one moment and never re-derived — not on restart, not on
  a catalogue upgrade, not on a tick. `applied_preset_id` is **not read by
  recommendation, by the safety gate, by irrigation control, or by offline-policy
  evaluation**; those four see a preset-configured plant and a hand-configured
  plant as identical, because they consume the same `MeasurementPolicy` rows,
  bindings, and measurements. Anything else gives the plant two owners, and the
  operator's edit is the one that loses.
- **A preset names a `MeasurementKind`, never a sensor.** The catalogue holds no
  `device_id`, `sensor_id`, `point`, or capability identity. Which probe supplies
  a kind for a given plant is a `SensorBinding` and stays one, so applying a
  preset resolves against the bindings the plant already has and creates,
  selects, or edits none. A catalogue cannot know which probe is in which pot.

Applying a preset to a **monitoring-only plant succeeds**: measurement policies
are created normally, and any dose or cooldown default is recorded as an inert
starting value that neither creates nor requires an `ActuatorBinding`. SAFETY-018
holds by construction, since nothing on this path writes to `actuator_bindings`.
Presets are most useful for exactly these plants.

Each entry carries `source`, `source_ref`, `license`, and `retrieved_at`.
External catalogues such as Trefle or Perenual may be used as **import and
research inputs** for building the curated data offline, with human review; they
are never a runtime dependency, and their licences must be verified to permit
redistribution before any of their data is committed (see Open questions).

Delivered by M5-017 (catalogue) and M5-018 (application).

A `plant_recommendations` row is written per evaluation only when the decision
or reason set **changes**, not on every tick — otherwise a 30-second tick would
write 2 880 rows per plant per day to record that nothing happened.

`watering_events` rows created by detection have `command_id = NULL` and
`mode = 'detected'`, which is how the daily-cap query
([time-model.md](../architecture/time-model.md) §6) correctly excludes them from
the automatic budget while still resetting the cooldown.

## State model

```text
        ┌──────────┐  sample invalid / absent
        │ Healthy  │──────────────────────────► SensorFault
        └────┬─────┘
             │ moisture < target_min
             ▼
        ┌──────────┐  moisture recovers
        │  Drying  │──────────────────────────► Healthy
        └────┬─────┘
             │ dry_duration >= dry_confirm_minutes
             ▼
   ┌────────────────────┐  watering detected / performed
   │ WaterRecommended   │──────────────────────► Healthy
   └────────────────────┘
             │ safety gate blocks
             ▼
     ┌────────────────┐
     │ WateringLocked │
     └────────────────┘
```

This is the **plant** state, which is descriptive and operator-facing. The
**irrigation** state machine that acts is separate and arrives in M6
([ADR-006](../adr/006-irrigation-state-machine-ownership.md)). Keeping them
distinct means the UI can show "needs water" without implying "is about to
water".

## Failure modes

| Failure | Behaviour |
|---|---|
| Insufficient samples for a trend | trend `None`; confidence reduced; recommendation still possible from the latest reading alone |
| All samples invalid | plant state `SensorFault`; recommendation `blocked` |
| Profile references a deleted device | plant surfaced with an error state; no crash |
| Weight sensor absent | detection falls back to moisture only, with lower confidence |
| Moisture jump caused by a command | attributed to the command, not double-counted as detected |
| Shared or cross-device sensors | each plant resolves its own bindings and measurement policies independently |
| Clock step | trend window shifts; handled by M6's clock-step lockout |

## Safety implications

M5 issues no commands, so it can violate no invariant. It does, however, build
the inputs M6's gate consumes, and two requirements are safety-load-bearing:

- **F-050-11** — a trend must be `None` rather than a fabricated slope from two
  noisy points. A confident wrong trend is worse than an absent one
  (SAFETY-012).
- **F-050-16** — attributing a detected rise to a completed command prevents
  double-counting a watering, which would corrupt both the cooldown and the
  rolling daily total that SAFETY-006 depends on.

- **F-050-03** — profile validation rejects a `dose_ml` above the firmware hard
  limit rather than clamping, so the operator learns the real limit while
  editing rather than during an incident
  ([ADR-011](../adr/011-configuration-and-secrets-model.md)).

- **F-050-01** — `auto_watering_enabled` defaults to false. A plant created and
  forgotten does nothing.

## Observability

Metrics:

```text
plants_total                     gauge
plant_state{state}               gauge
recommendations_total{decision}
manual_watering_detected_total
```

Events: `manual_watering_detected`, `ec_high`, `sensor_stuck`.

Logging: INFO when a recommendation *changes*, not on every evaluation. A tick
that reaches the same conclusion as the previous tick is not news.

## Testing strategy

- Unit: trend slope against known series including noise; `None` on sparse data;
  dry-duration accumulation and reset; detection thresholds at boundaries;
  command attribution window; profile validation rejections one rule at a time.
- Unit: the recommendation rule with each conjunct failing in isolation,
  asserting the exact reason set.
- Integration: SCEN-003 (recommendation without automation — **zero commands
  published**), SCEN-024 (stuck sensor).
- Integration: simulator dries the soil; assert the plant reaches
  `WaterRecommended` with reasons `moisture_below_target` and `dry_for`.
- Integration: inject a moisture step with no command; assert a `detected`
  watering event and a cooldown reset.

## Acceptance criteria

- [ ] A plant created from a preset has ordinary `MeasurementPolicy` rows,
      indistinguishable from hand-configured ones, and every value stays
      editable afterwards.
- [ ] No catalogue entry contains an interval, frequency, or schedule field.
- [ ] Every preset value is labelled as either a source fact or a Rhizo-derived
      default; there is no unlabelled third case.
- [ ] The catalogue is queryable with no network and no external service.
- [ ] `auto_watering_enabled` is still `false` on a plant created from a preset.
- [ ] Recommendation and threshold evaluation give identical results for a
      preset-configured and a hand-configured plant carrying the same values,
      and no decision path reads `applied_preset_id`.
- [ ] Applying a preset to a plant with no `ActuatorBinding` succeeds, creates
      its measurement policies, and leaves `POST /water` returning 422
      `no_actuator_bound` (SAFETY-018).
- [ ] Applying a preset creates, selects, or edits no `SensorBinding`.

- [ ] The simulator drying past the threshold produces `WaterRecommended` with a
      non-empty structured reason list.
- [ ] **No MQTT command is published in any M5 scenario.**
- [ ] A profile with `dose_ml = 200` is rejected with 422 naming
      `FIRMWARE_MAX_ML_PER_RUN`.
- [ ] A manual moisture step creates a `detected` watering event and resets
      time-since-last-watering.
- [ ] A moisture step following a completed command creates **no** second event.
- [ ] Trend is `None` with fewer than 5 valid samples in the window.
- [ ] A new plant has `auto_watering_enabled = false`.
- [ ] `GET /plants/{id}/recommendation` returns reasons for `no_water` as well as
      for `water`.

## Dependencies

- M4 (devices, staleness, sensor health, API scaffolding).

## Open questions

1. **Confidence as a single scalar** is a simplification — it conflates data
   sparsity, noise, and missing sensors. It is reported for operator intuition
   and is **not** used in any safety decision, so the simplification is
   contained. Revisit if it proves misleading in M7 field use.
2. **Detection thresholds** (8 pp, 100 g) are plausible starting values; they
   will need tuning against a real plant in M10. Nothing safety-critical depends
   on them — a missed detection means a conservative cooldown, not a risk.
3. **`recommended_ml` is currently just `profile.dose_ml`.** A volume derived
   from the moisture deficit and pot volume is tempting and is deliberately
   deferred: multi-dose feedback (M6) achieves the same convergence with a
   bounded worst case.

**Preset catalogue licensing — resolved in M5-017 on 2026-08-29.** The question
was whether Trefle or Perenual data could be redistributed inside the Rhizo
binary. It was not answered in Rhizo's favour and, more to the point, it was not
answerable from inside this repository: neither service's terms could be read and
verified here, and "free to query" was never evidence of a redistribution
licence. **The second branch was therefore taken.** No third-party row is
committed and no external API is contacted at build time or at run time. The
shipped catalogue is Rhizo-authored from general horticultural guidance, with a
`source`, `source_ref`, `license`, and `retrieved_at` on every entry; `license`
reads `rhizo-authored` precisely because there is no third-party licence to
honour.

That constrains what the entries may claim, which is why the `Provenance`
discriminator matters more than it first appears. Temperature and pH ranges are
`source_fact` values in their source's own units, citing the reference they came
from. Every soil-moisture band is a `rhizo_default` carrying its
`derived_from`, because converting horticultural advice such as "let the top
third dry" into a volumetric water content is an interpretation with a guess
inside it. Should a redistributable source be verified later, importing it is an
additive change to the same shape rather than a rewrite.

**How many species does the first catalogue need?** The working assumption held:
`presets.v1.json` ships **twenty-two** curated entries — houseplants, herbs, and
container edibles — rather than a scraped list. A small catalogue kept the
provenance discipline visible while it was still cheap to establish, and an
operator with an unlisted species still has the manual path, which M5-018's tests
check is unchanged by presets existing. Revisit once real usage shows what people
actually plant.


## Future work

- Evapotranspiration estimation from weight trend (M9+).
- Species-specific default profiles shipped with the system (M13).
- Seasonal adjustment (post-V1).
- Weather-informed recommendations ([PRD 140](140-field-readiness.md)).
