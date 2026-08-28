# Issue M5-019 — Add power mode, sleep announcement, and battery measurement kinds to the contract

**Status:** REDUCED before M5 started. The `device.status` half of this issue —
`PowerMode`, `PowerStatus`, the `sleeping` reason, the bounded sleep-announcement
rule, and their fixtures — was delivered by the 2026-08-28 post-M4 correction and
its dated review, both recorded in `docs/reports/M4.md`. **The Scope section below
is what is left**, and it is genuinely unimplemented. M5 has not started.

Delivered, and therefore *not* M5 work:

- `PowerMode` with `#[serde(other)] Unknown` and `PowerMode::effective`
- `device.status.data.power` (`mode`, `wake_interval_seconds`,
  `expected_wake_ms`, `wake_reason`, `battery_mv`, `awake_ms`)
- `reason: "sleeping"`, with anything unrecognised resolving to
  `connection_lost`
- `DeviceStatus::announces_sleep`, `announced_sleep_interval_seconds`,
  `declared_power_mode`, and `validate` — the single place the sleep rule lives
- fixtures `valid/status-sleeping.json` and the three under
  `invalid/status_sleep_wake_interval/`, with their `Expected` arm

`wake_reason` shipped as an `Option<String>` rather than the enum this issue
originally specified. Typing it stays in scope below: nothing reads it yet, so
the change is additive and costs one fixture.

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
- Config validation: `wake_interval_seconds` within the same
  `SLEEP_WAKE_INTERVAL_MIN_SECONDS..=SLEEP_WAKE_INTERVAL_MAX_SECONDS` range the
  status side already publishes, rejected rather than clamped
- `wake_reason` promoted from `Option<String>` to a typed
  `timer | cold_boot | external | watchdog | Unknown`, `Unknown` being the
  forward-compatible variant
- Once the edge publishes `device.config`, the registry's `power_mode` and
  `wake_interval_seconds` take that as their source instead of the device's own
  declaration, and PRD 040 F-040-20 and
  [http-api-boundaries.md](../../protocol/http-api-boundaries.md) §2.3 are
  updated in the same change
- Fixtures: valid `config_battery_mode`, valid `telemetry_battery_kinds`;
  invalid `config_wake_interval_out_of_range`

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
- [ ] An unrecognised `mode` string in `device.config` decodes to `AlwaysOn`.
- [ ] `device.config`'s `wake_interval_seconds` outside its documented range is
      **rejected** with a named typed error, not clamped, and shares its bounds
      with the status side rather than restating them.
- [ ] `wake_reason` is typed, and an unrecognised value decodes to `Unknown`
      rather than failing.
- [ ] Every new fixture behaves as its directory name states, and the delivered
      status fixtures stay green unchanged.
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

- `Unknown` conservative resolution for `PowerMode` in `device.config` and for
  the new `wake_reason` enum.
- Range rejection for `device.config`'s `wake_interval_seconds`.
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
