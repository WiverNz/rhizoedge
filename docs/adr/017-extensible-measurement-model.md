# ADR-017 — Extensible typed measurement model

## Status

Accepted — 2026-08-26. Contract in M1, storage in M3, thresholds in M5.

**Amends [ADR-002](002-mqtt-topic-versioning-and-qos.md)** (telemetry topics)
and **[ADR-004](004-sqlite-edge-persistence-model.md)** (the `measurements`
table shape).

## Context

The planned model hard-coded six measurements — moisture, soil temperature, EC,
pot weight, tank level, leak — as columns in a table and as four MQTT topics.
Adding ambient temperature meant a migration, a new topic, a new payload type, a
new ACL consideration, and a firmware topic-table entry.

The requirement set has grown to include ambient temperature and humidity,
illuminance (and PAR/PPFD later), pH, fertilisation events, and genuinely
measured nutrient values where real calibrated hardware exists. That is not a
list that will stop growing, and the current design charges a migration per
entry.

The obvious escape is a generic `{"name": ..., "value": ...}` bag. That is
explicitly rejected: it destroys compile-time semantics, gives the `no_std`
firmware nothing to validate against, and turns every range check into a runtime
string comparison.

The tension to resolve: **extensible without becoming untyped.**

## Decision

### A closed, typed `MeasurementKind` enum with a forward-compatible unknown

```rust
#[non_exhaustive]
pub enum MeasurementKind {
    SoilMoisture, SoilTemperature, SoilEc, SoilPh,
    AmbientTemperature, AmbientHumidity, Illuminance,
    PotWeight, TankLevel, LeakState,
    NitrateConcentration,          // only from a real calibrated sensor
    #[serde(other)] Unknown,       // forward compatibility
}
```

Each kind carries, as compile-time data in the contract crate:

```rust
pub struct KindSpec {
    pub unit: Unit,          // exactly one canonical unit per kind
    pub range: (f64, f64),   // physical plausibility bounds
    pub kind_class: Class,   // Scalar | Boolean
}
pub const fn spec(kind: MeasurementKind) -> KindSpec;
```

This is the whole trick: the *set* of kinds is extensible, but each kind is
strongly typed, single-unit, and range-checked by a `const fn` the firmware can
use. Adding ambient humidity costs one enum variant and one `KindSpec` — no
migration, no topic, no new payload type.

To be precise about what that does **not** cover: a device that must physically
*measure* the new kind still needs a firmware update for the driver and for its
capability declaration. The claim here is about the protocol and storage layers,
which are the ones that would otherwise force a schema change and a redeploy
across the whole system for every new sensor.

`Unknown` is what makes it forward-compatible: an older edge receiving a kind it
does not know **stores the sample** and treats it as advisory, rather than
rejecting the whole message. It never gates actuation
(SAFETY-012 — an unrecognised reading is not evidence).

### One canonical unit per kind, no unit negotiation

`SoilMoisture` is always `%VWC`. `AmbientTemperature` is always `°C`.
`Illuminance` is always lux. A device that measures in something else converts
before publishing.

Rejected alternative: a `unit` field the sender chooses. Unit mismatch is a
classic and expensive bug class, and "the receiver converts" means every
consumer needs a conversion table. One unit per kind makes a wrong unit a
*calibration* bug in one device rather than a systemic ambiguity.

The wire still carries `unit` — as a **check**, not a choice. A sample whose
declared unit disagrees with the kind's canonical unit is rejected.

### The sample envelope

```rust
pub struct MeasurementSample {
    pub kind: MeasurementKind,
    pub point: MeasurementPoint,   // "default", "depth_30cm", "ambient", …
    pub value: MeasurementValue,   // Scalar(f64) | Boolean(bool)
    pub unit: Unit,
    pub quality: Quality,          // Ok | Uncalibrated | Suspect | Fault
    pub sensor_id: Option<SensorId>,
    pub calibration_ref: Option<CalibrationRef>,
}
```

`quality` is the field that keeps the model honest. An uncalibrated soil probe
publishes `Uncalibrated`, and the safety gate treats anything other than `Ok` as
unusable for control while still storing it. That is how
[PRD 100](../prd/100-real-soil-sensor.md)'s "an uncalibrated sensor publishes
null" requirement generalises: it now publishes a *value with a quality*, which
is strictly more useful and equally safe.

### One batched telemetry topic replaces four

```text
before:  telemetry/soil  telemetry/weight  telemetry/tank  telemetry/pump
after:   telemetry                      (batch of MeasurementSample)
         actuator                       (actuator state — not a measurement)
```

One message per sampling cycle instead of four. Consequences that matter:

- **One envelope, one `message_id`, one dedup key** — the sample set from one
  cycle is atomic. Previously moisture could be stored while the tank reading
  from the same instant was lost to a redelivery edge case.
- Adding a kind needs no topic, no ACL change, no firmware topic table entry.
- Fewer, slightly larger messages: better for MQTT and much better for the
  future LoRaWAN path ([PRD 140](../prd/140-field-readiness.md)).

Actuator state moves to its own `actuator` topic because it is state, not a
measurement, and conflating them was a modelling error in the original design.

**This is a v1 change made before v1 exists.** M1 has not started, so nothing is
deployed and no compatibility is owed. See §Protocol version below.

### Storage: a narrow typed-kind table

`measurements` becomes:

```sql
CREATE TABLE measurements (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    device_id      TEXT NOT NULL REFERENCES devices(device_id),
    sensor_id      TEXT,
    point          TEXT NOT NULL DEFAULT 'default',
    kind           TEXT NOT NULL,          -- MeasurementKind, snake_case
    value_num      REAL,                   -- Scalar
    value_bool     INTEGER,                -- Boolean
    unit           TEXT NOT NULL,
    quality        TEXT NOT NULL,
    calibration_ref TEXT,
    received_at    INTEGER NOT NULL,       -- edge clock, AUTHORITATIVE
    device_time_ms INTEGER,                -- advisory
    boot_id        TEXT,
    sequence       INTEGER,
    batch_id       TEXT NOT NULL           -- groups one sampling cycle
);
CREATE INDEX idx_meas_lookup ON measurements(device_id, point, kind, received_at DESC);
CREATE INDEX idx_meas_time   ON measurements(received_at);
CREATE INDEX idx_meas_batch  ON measurements(batch_id);
```

The safety-critical query stays a single index seek:

```sql
SELECT value_num, quality, received_at FROM measurements
WHERE device_id=?1 AND point=?2 AND kind=?3
ORDER BY received_at DESC LIMIT 1;
```

`batch_id` preserves what the wide table gave for free — which readings came from
the same instant — which charts and the manual-watering detector both need.

Cost, stated plainly: **six rows per cycle instead of one**. At a 300 s interval
that is ~1 700 rows/device/day, ~630 k/device/year, tens of megabytes with
indexes. SQLite is entirely comfortable there, and M13's hourly downsampling
bounds it further.

Cloud PostgreSQL mirrors the shape, partitioned by `edge_id` as before
([ADR-005](005-cloud-event-model-and-idempotency.md)).

### Fertilisation: events are not measurements

A deliberate separation, because conflating them is how a system starts claiming
nutrient values it never measured:

```text
FertilisationEvent          an action a human or machine performed
  fertiliser_type, amount, unit, occurred_at, optional concentration
  → stored in plant_events, correlated with EC in charts

MeasurementSample           a value a sensor produced
  kind = NitrateConcentration, unit = mg_l, quality, calibration_ref
  → stored in measurements, ONLY when real hardware measured it
```

**N/P/K is never inferred from EC.** This limitation is preserved verbatim from
[PRD 100](../prd/100-real-soil-sensor.md) and
[PRD 140](../prd/140-field-readiness.md): cheap "NPK" probes derive their output
from EC by an undisclosed formula, and presenting that as a nutrient measurement
would be a false claim about a real field. There is no `MeasurementKind` for
nitrogen, phosphorus, or potassium, and adding one requires a calibrated sensor
plus a `calibration_ref` — the type system makes the honest path the only path.

### Protocol version stays v1

No v2. [versioning-policy.md](../protocol/versioning-policy.md) triggers v2 on
*incompatible change to a deployed contract*. v1 has never been implemented or
deployed; M1 writes it for the first time. Bumping to v2 before v1 exists would
leave a version number nobody ever spoke.

The versioning policy is amended with one sentence making that explicit, so a
future reader does not conclude the rules were bent.

## Alternatives considered

**Keep wide columns, add one per kind.** Rejected: a migration per sensor type
forever, a very sparse table, and the firmware payload type grows in lockstep.

**Generic `{name, value}` bag.** Rejected, and named as an anti-goal in the
requirements. No compile-time semantics, no `const` range checks for `no_std`,
runtime string matching in the safety path.

**JSON blob column per sample.** Rejected: unqueryable for charts without
extraction, no index on kind, and it hides unit and quality where no validator
sees them.

**Sender-chosen units.** Rejected — see above.

**One topic per measurement kind.** Rejected: topic proliferation, an ACL and a
firmware table entry per kind, and it loses the atomic sampling batch.

**Separate table per kind.** Rejected: schema churn per kind, and cross-kind
queries (charts, the batch view) become unions.

## Consequences

Positive:

- New measurement kinds cost one enum variant and one `KindSpec` **at the
  protocol and storage layers** — no migration, no topic, no payload type. A
  device that physically measures the new kind still needs a driver and a
  capability-declaration change, i.e. a firmware release.
- The sampling batch is atomic — a redelivery cannot split it.
- `quality` and `calibration_ref` make uncalibrated and suspect data
  representable instead of being forced into null-or-nothing.
- The `const fn spec()` range table is usable by `no_std` firmware, so device and
  edge validate against literally the same bounds.
- The honest-nutrients constraint is enforced by the type system, not by
  documentation alone.

Negative, accepted:

- **Row count grows ~6×.** Bounded by retention and downsampling; irrelevant at
  V1 volumes but real at fleet scale.
- **Charts must pivot** rather than reading columns. One query helper, written
  once in `rhizo-storage`.
- **`#[non_exhaustive]` + `Unknown` means exhaustive matching is impossible**, so
  every consumer must handle an unrecognised kind. That is the point, and the
  conservative branch is mandated, but it is more code at each call site.
- Existing planning text referring to `measurements.moisture_vwc` is now wrong
  and had to be rewritten across ADR-004, PRD 030, PRD 050, and the M3 issues.

## Risks

- **`Unknown` treated as permissive** by a careless consumer. *Mitigation:* the
  gate's exhaustive match has no catch-all; `Unknown` maps explicitly to
  advisory-only; SAFETY-012's property test generates unknown kinds.
- **Kind proliferation** — thirty kinds nobody has hardware for. *Mitigation:* a
  variant is added only when a device can actually produce it, or when a reserved
  expansion point is justified in an ADR.
- **`quality` ignored** in a query that then drives control. *Mitigation:* the
  repository's "latest control sample" method filters `quality = 'ok'` internally
  rather than leaving it to callers.
- **Batch atomicity lost** if a device publishes kinds across several messages.
  *Mitigation:* the protocol requires one batch per sampling cycle; the
  conformance test asserts it.

## Follow-up

- [docs/protocol/mqtt-v1.md](../protocol/mqtt-v1.md) §5 — the normative payloads.
- [ADR-004](004-sqlite-edge-persistence-model.md) — schema revision.
- [ADR-016](016-plant-binding-and-policy-model.md) — how plants consume kinds.
- M1 implements the kinds, specs, and batch payload; M3 the narrow table; M5 the
  per-kind thresholds; M10 real sensors producing new kinds.
