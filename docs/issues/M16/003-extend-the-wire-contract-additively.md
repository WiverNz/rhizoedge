# Issue M16-003 — Extend the wire contract additively

**Milestone:** M16 · **PRD:** [PRD 160](../../prd/160-verified-watering.md) · **Depends on:** M16-002

## Context

Every device this project will ever deploy is in a pot, on a shelf, and
re-flashing one is a hardware task with a ladder. The wire changes for Verified
Watering must therefore be additive within v1 in the strict sense the versioning
policy means: an edge that ignores every new field behaves exactly as it does
today, and a device that never sends them keeps working forever.

## Goal

The `rhizo-mqtt-contract` half of Verified Watering, entirely additive, with
fixtures proving both directions.

## Scope

- `MeasurementKind::ReservoirWeight` (`gram`, 0.0–100000.0) and
  `MeasurementKind::FlowRate` (`ml_s`, 0.0–1000.0, reserved with no V1 producer).
- An optional `delivery` object on `command.result.data`: `measured_ml`,
  `estimated_ml`, `evidence`, `outcome`, `started_at_ms`, `stopped_at_ms`,
  `settle_ok`, `calibration_version`.
- Optional `witness_health` and `last_measured_ml` on `actuator.state.data`.
- `RejectReason` gains `WitnessFaulted`, `UnexpectedFlow`, `ActuatorMaintenance`.
- Device `EventKind` gains `delivery.fault` and `flow.unexpected`, with typed
  `EventDetail` variants.
- Protocol §5.1, §5.3, §5.10, §9 change log, and the versioning policy's
  worked-examples list updated in the same change.
- Fixtures: valid and invalid, under `test/fixtures/protocol/`.

## Non-goals

- **A new `CommandStatus` variant.** ADR-020 §Alternatives: an older edge decodes
  an unknown status to `Unknown`, which charges the full request and creates *no
  watering event* — so a newer device's successful doses would silently stop
  producing watering events. An optional sub-object is ignored instead, which is
  what the policy actually promises.
- A new topic. The device subscription set stays at eight exact topics.
- Any change to QoS, retention, dedup key, or command TTL.

## Dependencies

- M16-002

## Implementation notes

`rhizo-mqtt-contract` is `no_std` and firmware-facing. Nothing added here may
pull in a `std`-only dependency, and both bare-metal targets are part of this
issue's verification, not an afterthought.

The `delivery` object is `Option<Delivery>` with
`#[serde(default, skip_serializing_if = "Option::is_none")]`, so a device that
omits it produces a byte-identical `command.result` to today's. Assert that with
a fixture: the pre-M16 `command.result` fixture must still decode **and** still
re-encode without materialising a `delivery` key. That round-trip is what makes
"additive" a checked claim rather than an intention.

Every new enum takes `#[serde(other)] Unknown`, and each `Unknown` must resolve
to the conservative branch — an unrecognised outcome is not a success, and an
unrecognised `witness_health` is not healthy (SAFETY-012).

A fixture directory name **is** its expected typed failure. Add one invalid
fixture per new failure class and one match arm in
`crates/mqtt-contract/tests/fixtures.rs`; a directory the harness does not
recognise fails the suite rather than being skipped.

`FlowRate` is reserved deliberately with no producer: reserving the kind now
costs one table row and means a future inline flow meter is a driver, not a
protocol change.

## Acceptance criteria

- [ ] Every change is additive; no field removed, retyped, or made required.
- [ ] The pre-M16 `command.result` fixture decodes and re-encodes with no
      `delivery` key.
- [ ] The pre-M16 `actuator.state` and `telemetry.batch` fixtures still decode.
- [ ] Unknown enum values decode to `Unknown` and take the conservative branch.
- [ ] Protocol §5.1, §5.3, §5.10, and §9 are updated in this change.
- [ ] The versioning policy records this as an additive v1 change with its
      reasoning.
- [ ] The device subscription set is unchanged at eight exact topics.
- [ ] Both bare-metal targets build.

## Verification

```bash
cargo test -p rhizo-mqtt-contract --test fixtures
cargo test -p rhizo-mqtt-contract
cargo build -p rhizo-mqtt-contract --no-default-features --target thumbv7em-none-eabi
cargo build -p rhizo-policy --no-default-features --target thumbv7em-none-eabi
cargo run -p rhizo-docscheck
```

## Tests required

- Round-trip of every new payload as its **concrete type**, never
  `serde_json::Value`.
- Backward-compatibility fixtures in both directions.
- One invalid fixture per new failure class, with its match arm.
- Unknown-variant conservatism for each new enum.

## Documentation impact

- `docs/protocol/mqtt-v1.md` §5.1, §5.3, §5.10, §9 change log.
- `docs/protocol/versioning-policy.md` — a third worked example of an additive
  v1 change.
- `docs/adr/017-extensible-measurement-model.md` — two kinds added at the
  designed extension point.

## Files likely affected

```text
crates/mqtt-contract/src/payload/telemetry.rs
crates/mqtt-contract/src/payload/command.rs
crates/mqtt-contract/src/payload/status.rs
crates/mqtt-contract/src/payload/events.rs
crates/mqtt-contract/tests/fixtures.rs
test/fixtures/protocol/
docs/protocol/mqtt-v1.md
docs/protocol/versioning-policy.md
```
