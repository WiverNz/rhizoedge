# Issue M5-019 — Add power mode, sleep announcement, and battery measurement kinds to the contract

**Status:** PARTIALLY SUPERSEDED before M5 started. The minimal status
power/sleep contract needed by registry liveness was delivered as the 2026-08-28
post-M4 correction recorded in `docs/reports/M4.md`. Battery measurement kinds
and desired-config power fields remain M5 scope; M5 has not started.

**Milestone:** M5 · **PRD:** [PRD 050](../../prd/050-plant-model-and-recommendations.md) · **Depends on:** M1-019

## Context

[ADR-018](../../adr/018-battery-and-deep-sleep-device-mode.md) adds a battery
device that sleeps between samples. Every later piece of that work — the edge's
liveness model (M5-020), the simulator (M5-021), pending commands (M6-022), the
firmware (M9-019…M9-021), the UI (M12-018) — needs the same handful of wire
fields, so they land once, here, before anything consumes them.

**This is entirely additive within v1 and forces no version bump.**
[versioning-policy.md](../../protocol/versioning-policy.md) §1 permits new
optional `data` fields, new `MeasurementKind` variants (the designed extension
point of [ADR-017](../../adr/017-extensible-measurement-model.md)), and new enum
variants provided `Unknown` takes the conservative branch. All three apply.
No new topic, no retention change, no QoS change.

## Goal

Define the battery-mode wire surface in `rhizo-mqtt-contract`, with fixtures, so
that every consumer decodes the same thing.

## Scope

- `MeasurementKind::BatteryVoltage` and `MeasurementKind::BatteryPercent` with
  their `const fn spec()` entries (units `V` and `%`, valid ranges, advisory)
- `device.config.data.power`: `mode` (`always_on` | `battery`),
  `wake_interval_seconds`, `sensor_warmup_ms`, `awake_budget_seconds` — all
  optional, with an absent block meaning `always_on`
- `PowerMode` with `#[serde(other)] Unknown`, and the documented rule that
  `Unknown` and absent both resolve to `AlwaysOn`
- `device.status.data.power`: `mode`, `wake_interval_seconds`,
  `expected_wake_ms`, `wake_reason` (`timer` | `cold_boot` | `external` |
  `watchdog` | `Unknown`), `battery_mv`, `awake_ms` — all optional, all advisory
- `reason: "sleeping"` as an additional variant of the offline-status reason,
  with `Unknown` resolving to `connection_lost`
- Config validation: `wake_interval_seconds` within a documented range, rejected
  rather than clamped
- Fixtures: valid `status_sleeping`, valid `config_battery_mode`, valid
  `telemetry_battery_kinds`; invalid `config_wake_interval_out_of_range`,
  invalid `status_sleeping_without_wake_interval`

## Non-goals

- Any edge or device behaviour — this issue defines types and validation only.
- A new topic or a change to retention, QoS, or the subscription set. The device
  subscription list stays at seven exact topics.
- `MeasurementKind` variants for solar production or charge state. Nothing
  produces them, and [ADR-018](../../adr/018-battery-and-deep-sleep-device-mode.md)
  §7 forbids any decision from consuming them.

## Dependencies

- M1-019

## Implementation notes

The conservative-`Unknown` direction is opposite for the two enums, and both
directions matter:

```text
PowerMode::Unknown        → treated as AlwaysOn      (never start sleeping)
OfflineReason::Unknown    → treated as ConnectionLost (never assume asleep)
```

Both resolve towards *stays reachable* and *is reported absent*, which is the
SAFETY-012 direction for a field whose whole purpose is to explain silence.

`expected_wake_ms` is a **diagnostic**, and the type-level comment must say so.
It is carried on a retained message, and a retained timestamp is the exact shape
of bug the `time` topic's retention prohibition exists to prevent. Nothing may
apply it to a clock; only `edge.time` sets a clock
([time-model.md](../../architecture/time-model.md) §3b).

Add the two battery kinds as ordinary advisory measurement kinds. They must not
acquire a special case anywhere: a battery reading is telemetry, chartable and
alertable like any other, and never an input to an irrigation decision.

The fixture corpus is append-only ([versioning-policy.md](../../protocol/versioning-policy.md)
§6). Valid fixtures decode as their **concrete payload type**, never as
`serde_json::Value`, and each invalid directory name is the expected typed
failure and needs its match arm in `crates/mqtt-contract/tests/fixtures.rs`.

## Acceptance criteria

- [ ] The two battery `MeasurementKind` variants exist with correct specs and
      round-trip through the telemetry batch.
- [ ] An absent `power` block in `device.config` decodes to `AlwaysOn`.
- [ ] An unrecognised `mode` string decodes to `AlwaysOn`, not to an error.
- [ ] An unrecognised offline `reason` decodes to `connection_lost`.
- [ ] `wake_interval_seconds` outside its documented range is **rejected** with a
      named typed error, not clamped.
- [ ] Every new fixture behaves as its directory name states.
- [ ] `cargo build -p rhizo-mqtt-contract --no-default-features --target
      thumbv7em-none-eabi` still succeeds.
- [ ] The device subscription set is still exactly seven topics.

## Verification

```bash
cargo test -p rhizo-mqtt-contract --test fixtures
cargo test -p rhizo-mqtt-contract power_mode
cargo build -p rhizo-mqtt-contract --no-default-features --target thumbv7em-none-eabi
cargo run -p rhizo-docscheck
```

## Tests required

- `Unknown` conservative resolution for both enums.
- Range rejection for `wake_interval_seconds`.
- Round-trip of both battery measurement kinds inside a batch.
- The full fixture corpus, including the new files.

## Documentation impact

- [mqtt-v1.md](../../protocol/mqtt-v1.md) §5.1, §5.5, §5.6, §5.7.
- [versioning-policy.md](../../protocol/versioning-policy.md) — the additive
  change recorded.

## Files likely affected

```text
crates/mqtt-contract/src/measurement.rs
crates/mqtt-contract/src/power.rs
crates/mqtt-contract/src/status.rs
crates/mqtt-contract/src/config.rs
crates/mqtt-contract/tests/fixtures.rs
test/fixtures/protocol/valid/
test/fixtures/protocol/invalid/
```
