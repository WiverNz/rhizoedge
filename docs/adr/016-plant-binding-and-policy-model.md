# ADR-016 — Plant, binding, and policy model

## Status

Accepted — 2026-08-26. Contract in M1, entities in M5, enforcement in M6.

**Supersedes the `PlantProfile`-centric model** sketched in
[ADR-011](011-configuration-and-secrets-model.md) layer L4 and
[PRD 050](../prd/050-plant-model-and-recommendations.md). Profiles survive as
*templates*; they are no longer the whole configuration.

## Context

The planned configuration model assumed one shape of plant: a pot with a
moisture probe and a pump, described by a single `PlantProfile` with a moisture
target and a dose. That is a demo, not a product.

Real deployments diverge immediately:

```text
Plant A   moisture 28–45 %, confirm 30 min, dose 35 ml, has a pump
Plant B   moisture 18–30 %, confirm 4 h,    dose 15 ml, has a pump
Plant C   monitoring only — alerts, no actuator, must never be "waiting for a pump"
Plant D   two probes at different depths on one device
Plant E   ambient temperature and light from a shared room sensor,
          soil moisture from its own probe
```

A single flat `PlantConfig` cannot express these without either a field per
sensor type forever, or one global rule applied to plants it does not fit. Worse,
binding business rules directly to a physical sensor id makes replacing a broken
probe a data-migration exercise.

The pressure to fix this now is [ADR-015](015-device-offline-autonomy.md): an
offline policy must say *which* measurements a *specific* plant requires. That is
impossible in a model where sensors are global.

## Decision

### Five separate concepts, never merged

```text
physical sensor   a channel on a device that produces readings
measurement       a typed value with a kind, unit, quality, point, time
plant             the thing being cared for
threshold policy  per-plant interpretation of a measurement kind
automation rule   what to do about it, connected or offline
actuator          the optional thing that can act
```

Merging any two of these is what produced the original design's limits. In
particular, **a threshold belongs to a plant, not to a sensor**: the same room
temperature sensor is "fine" for one plant and "critical" for another.

### The model

```text
Device
├── capabilities
│   ├── sensors[]    { sensor_id, kinds[], measurement_point }
│   └── actuators[]  { actuator_id, kind, limits }
└── (declared by the device, never assumed — see §Capability discovery)

Plant
├── plant_id, name, species, pot_volume_ml, soil_type
├── profile_id                      → template it was created from (optional)
├── auto_watering_enabled           connected-mode opt-in, default false
│
├── SensorBinding[]
│   ├── device_id, sensor_id, measurement_point
│   ├── kind                        which measurement this binding supplies
│   └── role                        Control | Required | Advisory
│
├── ActuatorBinding[0..1]           OPTIONAL — see §Optional actuator
│   ├── device_id, actuator_id
│   └── kind                        IrrigationPump today; extensible
│
├── MeasurementPolicy[]             one per bound kind — see ADR-017 §thresholds
│   ├── kind
│   ├── target_min / target_max     optional
│   ├── warning_low / warning_high  optional
│   ├── critical_low / critical_high optional
│   ├── stale_after                 required
│   ├── hysteresis                  optional
│   └── confirm_duration            optional
│
├── AlertPolicy                     which crossings raise events, and at what severity
└── AutomationPolicy
    ├── connected                   full Edge rules (dose, cooldown, caps, …)
    └── offline                     the restricted OfflinePolicy (ADR-015)
```

### Bindings, not sensor ids in rules

A rule names a **kind and a role**, and a binding maps that to hardware. Replacing
a failed probe is editing one binding; no threshold, policy, or history changes.

Three roles, and the distinction is safety-relevant:

| Role | Meaning | Missing/stale ⇒ |
|---|---|---|
| `Control` | drives the automation decision | **refuse to actuate** |
| `Required` | must be healthy for actuation to be safe (e.g. leak, tank) | **refuse to actuate** |
| `Advisory` | recorded, charted, may raise alerts | actuation unaffected |

Exactly one `Control` binding per automating plant. `Required` is how a plant
declares that it needs its tank sensor without pretending tank level is what
triggers watering. `Advisory` is how ambient temperature can raise a critical
alert without ever gating the pump (SAFETY-017, SAFETY-018).

### The actuator is optional

`ActuatorBinding` is `[0..1]`, and zero is a **first-class, fully supported
state** — not a degraded one. A plant without an actuator gets telemetry,
history, thresholds, warnings, critical alerts, recommendations, and UI
visibility. It simply has no actuation path.

The API reflects this honestly: `POST /plants/{id}/water` on a plant with no
actuator returns **422** with `no_actuator_bound`, not 409 (which means "refused
by safety") and not 500. The UI renders no watering controls at all rather than
disabled ones (SAFETY-018).

Supported plant shapes, all first-class:

```text
monitoring only
monitoring + recommendation
monitoring + manual remote watering
monitoring + connected automatic watering
monitoring + offline autonomous watering
```

### Capability discovery, not assumption

A device **declares** its sensors and actuators in its retained status. The Edge
never assumes `device == pump controller`. A binding that names a capability the
device did not declare is rejected at write time with a specific error.

Future actuator kinds — `valve`, `grow_light`, `fan`, `heater`, `humidifier`,
`fertiliser_dosing_pump` — are representable in the enum as reserved variants
with **no implementation and no automation semantics**. This is a protocol
expansion point, not a feature: nothing in M1–M13 acts on them.

### Profiles become templates

`PlantProfile` remains, demoted to what it is actually good at: a named starting
point (`monstera_default`) that pre-populates `MeasurementPolicy[]` and
`AutomationPolicy` when a plant is created. After creation the plant owns its own
values. Editing a profile does **not** retroactively rewrite plants, because
silently changing the irrigation rules of twelve plants is not a feature.

### Validation belongs to the Edge

Every binding and policy is validated on write:

- the named device exists and declared that capability
- exactly one `Control` binding for an automating plant
- `target_min < target_max`, `warning` inside `critical`, hysteresis coherent
- `dose_ml ≤ FIRMWARE_MAX_ML_PER_RUN`, `dose × max_doses ≤ max_volume_per_window`
- an `AutomationPolicy` with no `ActuatorBinding` is rejected

**Reject, never clamp** ([ADR-011](011-configuration-and-secrets-model.md)).

## Alternatives considered

**Keep one flat `PlantConfig`.** Rejected: cannot express per-plant thresholds
over a shared sensor, cannot express monitoring-only cleanly, and needs a new
field for every measurement kind.

**Bind rules directly to `sensor_id`.** Rejected: replacing hardware would
rewrite every rule and orphan history. The binding indirection costs one table.

**Thresholds on the sensor rather than the plant.** Rejected: it makes a shared
room sensor unusable, since two plants legitimately disagree about what "too
cold" means.

**Make the actuator mandatory and model monitoring-only as "actuator disabled".**
Rejected: it forces every monitoring plant into a permanent lockout-shaped state,
and the UI would show watering controls for a plant that has no pump — which is
how an operator ends up believing water is possible when it is not.

**One `AutomationPolicy` used for both connected and offline.** Rejected: the
offline subset is deliberately narrower (ADR-015). Sharing one type would invite
the offline evaluator to grow toward the connected one, which is exactly the
drift to prevent.

**Implement the future actuator kinds now.** Rejected as speculative. Reserving
enum variants costs nothing; building a generic automation framework for
hardware nobody owns costs a milestone.

## Consequences

Positive:

- Every plant shape above is representable, including the two the old model
  could not express at all.
- Replacing a sensor is a binding edit.
- Offline policies can state their own required measurements, which is what
  makes SAFETY-017 enforceable.
- Monitoring-only plants stop being second-class, which matters because most
  plants in a real home will never have a pump.

Negative, accepted:

- **More tables and more joins** than a flat config. The safety-critical query
  ("latest control measurement for plant P") stays a single indexed lookup, which
  is the one that had to stay fast.
- **More validation surface**, and therefore more ways to reject an operator's
  input. Mitigated by specific error messages naming the violated rule.
- **The UI has materially more to render** (PRD 120 grows).
- Existing planning text that says "the plant profile" now has to say "the
  plant's measurement policy", and that wording churn is real.

## Risks

- **Binding sprawl** — a plant with eight advisory bindings whose thresholds
  nobody maintains. *Mitigation:* profiles seed a sensible minimum; the UI shows
  which bindings have no policy.
- **A `Control` binding silently removed**, leaving an automating plant with no
  trigger. *Mitigation:* validation rejects removing the last `Control` binding
  while automation is enabled; the gate independently refuses on a missing
  control measurement.
- **Role misuse** — marking the leak sensor `Advisory` and losing its veto.
  *Mitigation:* leak and tank are forced to `Required` for any plant with an
  actuator; validation rejects otherwise.

## Follow-up

- [ADR-017](017-extensible-measurement-model.md) — the measurement kinds these
  bindings and policies refer to.
- [ADR-015](015-device-offline-autonomy.md) — the offline half of `AutomationPolicy`.
- [configuration-model.md](../architecture/configuration-model.md) — updated layers.
- M1 carries capabilities and policy on the wire; M5 builds the entities and
  validation; M6 consumes them in the gate; M12 exposes them in the UI.
