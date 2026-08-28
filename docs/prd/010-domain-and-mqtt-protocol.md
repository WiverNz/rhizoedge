# PRD 010 — Domain Model and MQTT Protocol

**Milestone:** M1 · **Status:** IMPLEMENTED · **Depends on:** M0

> **Revised 2026-08-26, before implementation.** Scope grew to cover the typed
> `MeasurementKind` model and batched telemetry
> ([ADR-017](../adr/017-extensible-measurement-model.md)), device capability
> declaration ([ADR-016](../adr/016-plant-binding-and-policy-model.md)), the
> offline policy and event payloads, and the new `rhizo-policy` crate
> ([ADR-015](../adr/015-device-offline-autonomy.md)). Issues M1-014…M1-018 were
> added and M1-006/007/008/010 expanded. This is the milestone that freezes the
> contract, which is why the changes landed before it started.
>
> **Additional goals:** one canonical unit per measurement kind, enforced by a
> `const fn` the firmware can use; capabilities declared rather than assumed; an
> offline policy that is validated, versioned, and activated atomically; buffered
> events whose `event_id` is stable across replay.
>
> **Additional acceptance criteria:** an unrecognised measurement kind decodes,
> stores, and is treated as advisory rather than rejected; a policy omitting
> `enabled` defaults to `false`; `rhizo-policy` builds `no_std`; no `chrono` type
> appears in any offline payload.

## Summary

Define the shared wire contract (`rhizo-mqtt-contract`) and the pure domain
types (`rhizo-domain`). This is the contract that lets the simulator, the edge,
and the firmware be written independently and interoperate, and it is the crate
the firmware imports.

## Problem

Every later milestone depends on these types. Getting them wrong is expensive:
the protocol is the one thing that cannot be changed cheaply once devices exist
in pots. Two specific dangers:

1. A contract crate that accidentally requires `std` cannot be used by firmware,
   and the breakage is invisible to the default CI job.
2. A safety validator implemented once in the simulator and once in the firmware
   will diverge, and every simulator-based safety test becomes worthless.

## Goals

1. `rhizo-mqtt-contract`, `no_std` + `alloc`, implementing
   [docs/protocol/mqtt-v1.md](../protocol/mqtt-v1.md) exactly.
2. `validate_water_command` — the single shared actuation gate.
3. `rhizo-domain` skeleton: identifiers, measurement types, plant/irrigation
   state enums, `Clock` trait. (The state machine itself is M6.)
4. A protocol fixture corpus run by both workspaces.
5. Compile-time guards: `no_std` verification, and a clippy ban on direct clock
   access inside the domain.

## Non-goals

- The irrigation state machine and safety gate logic (M6).
- The recommendation engine (M5).
- Any MQTT client, storage, or I/O — this crate describes bytes, not transport.
- Protocol v2 or any binary encoding (M14).

## User/system flows

Developer-facing:

```text
write a message  → Envelope::new(kind, device_id, data).to_json()
read a message   → Topic::parse(topic) → Envelope::<T>::from_json(bytes)
                 → validate identity consistency → typed payload
guard a command  → validate_water_command(&cmd, &guard_state) → Verdict
```

## Functional requirements

### Contract crate

| ID | Requirement |
|---|---|
| F-010-01 | `#![no_std]` with `extern crate alloc`; a `std` feature adds only `std::error::Error` impls |
| F-010-02 | No `chrono` dependency; time is `UtcMillis(i64)` |
| F-010-03 | `DeviceId` newtype with the §2 grammar; no constructor bypasses validation |
| F-010-04 | `Topic` enum with `to_string` and `parse` covering all twelve topic forms |
| F-010-05 | `Envelope<T>` with all envelope fields per protocol §4 |
| F-010-06 | Payload types for all twelve message kinds |
| F-010-07 | Range constants and per-field validation returning which field failed |
| F-010-13 | `EdgeTime` payload (`edge_time_ms`) plus `TIME_SYNC_INTERVAL_SECONDS` and `TIME_SYNC_MAX_AGE_SECONDS` constants |
| F-010-08 | Inbound types use `#[serde(default)]`; unknown fields ignored |
| F-010-09 | Safety-relevant enums decode unknown values to a conservative variant |
| F-010-10 | Hard-limit constants (`FIRMWARE_MAX_*`) defined here |
| F-010-11 | `validate_water_command` implements protocol §5.8 steps 1–12 in order, allocation-free |
| F-010-12 | `NaN`/`Infinity` never serialised; treated as out-of-range on decode |

### Domain crate

| ID | Requirement |
|---|---|
| F-010-20 | `PlantId`, `CommandId`, `EventId` newtypes |
| F-010-21 | `PlantState`, `IrrigationState`, `LockoutReason`, `WateringMode` enums |
| F-010-22 | `Clock` trait with `SystemClock`; `TestClock` lives in testkit |
| F-010-23 | `PlantProfile` type with validation (rejects, never clamps) |
| F-010-24 | `SoilSample` with `is_valid()` and `is_stale(now, max_age)` |
| F-010-25 | No I/O and no direct clock access anywhere in the crate |

### Guards

| ID | Requirement |
|---|---|
| F-010-30 | CI builds the contract crate `--no-default-features` for a bare-metal target |
| F-010-31 | `clippy.toml` disallows `Utc::now` and `SystemTime::now` in `rhizo-domain` |
| F-010-32 | Fixture corpus in `test/fixtures/protocol/{valid,invalid}/` |

## Interfaces

```rust
pub const PROTOCOL_VERSION: u16 = 1;
pub const FIRMWARE_MAX_RUN_SECONDS: u32 = 20;
pub const FIRMWARE_MAX_ML_PER_RUN: f32 = 80.0;
pub const FIRMWARE_MAX_DAILY_ML: f32 = 500.0;
pub const MAX_CLOCK_SKEW_SECONDS: i64 = 5;
pub const COMMAND_DEDUP_RING: usize = 16;

pub struct DeviceId(/* private */);
impl DeviceId { pub fn parse(s: &str) -> Result<Self, DeviceIdError>; }

pub struct UtcMillis(pub i64);

pub enum MessageKind { TelemetryBatch, ActuatorState, DeviceEvents,
                       DeviceStatus, DeviceConfig, DevicePolicy, EdgeTime,
                       CommandWater, CommandTare, CommandCalibrate,
                       CommandResult, EventAck }

pub struct Envelope<T> { /* protocol §4 fields */ }
impl<T: Serialize + DeserializeOwned> Envelope<T> {
    pub fn to_json(&self) -> Result<String, EncodeError>;
    pub fn from_json(bytes: &[u8]) -> Result<Self, DecodeError>;
    pub fn check_identity(&self, topic_device: &DeviceId) -> Result<(), DecodeError>;
}

pub enum Topic { /* twelve variants; see mqtt-v1.md §3 for the exhaustive list */ }
impl Topic {
    pub fn to_string(&self) -> alloc::string::String;
    pub fn parse(topic: &str) -> Result<Topic, TopicError>;
    pub fn device_id(&self) -> &DeviceId;
}

pub fn validate_water_command(cmd: &WaterCommand, state: &DeviceGuardState)
    -> CommandVerdict;
```

## Data model

No persistence. The types here are wire and in-memory representations only.
Their mapping to SQLite columns is PRD 030's concern.

## State model

The state *enums* are defined here; the *transitions* are M6. Defining the enums
early lets M3/M4 store and report state without waiting for the state machine.

```rust
pub enum PlantState { Healthy, Drying, WaterRecommended, WaitingForResponse,
                      Recovering, SensorFault, WateringLocked }

pub enum IrrigationState { Normal, Drying, DryConfirmed, DoseIssued,
                           WaitForAbsorption, Recheck, Locked(LockoutReason) }

pub enum LockoutReason { Leak, TankLow, StaleData, SensorFault, DailyLimit,
                         MaxDosesReached, NoDeliveryDetected, Uncertain,
                         ClockUnsynced, PumpFault }
```

## Failure modes

| Failure | Behaviour |
|---|---|
| Malformed JSON | `DecodeError::Json` — caller quarantines |
| `v` != 1 | `DecodeError::UnsupportedVersion` |
| topic/payload `device_id` mismatch | `DecodeError::DeviceMismatch` |
| invalid `device_id` grammar | `TopicError::InvalidDeviceId` |
| field out of range | not a decode error — decodes with the field `None` plus a `ValidationReport` listing what was clamped, so the caller can store the good fields |
| unknown enum value | decodes to the conservative variant |

The out-of-range behaviour is the important one: a message with one bad field
must remain usable (protocol §10).

## Safety implications

This PRD delivers the mechanism for three invariants, though enforcement is
tested in M6:

- **SAFETY-002** — `validate_water_command` steps 2 and 3 (clock unsynced,
  expired).
- **SAFETY-007** — steps 10–12 (clamping against compile-time hard limits).
- **SAFETY-012** — conservative decoding of unknown enum values, and `Option`
  rather than defaults for absent safety inputs.

The single most consequential requirement here is F-010-11: **one validator,
called by both the simulator and the firmware.** Without it, every safety test
in M6 tests a simulator that hardware does not resemble
([ADR-008](../adr/008-shared-code-simulator-and-firmware.md)).

## Observability

The contract crate has no logging — it is `no_std` and must not assume a
subscriber. Errors are typed and carry enough detail (which field, which
expectation) for the caller to log usefully.

## Testing strategy

- Encode/decode round trip for every message kind.
- Topic build/parse round trip; malformed topics rejected.
- `DeviceId` grammar: valid cases plus `x/#`, `+`, `#`, uppercase, too short,
  too long, leading/trailing hyphen.
- Range validation at exact boundaries: 0.0, 100.0, 100.1, −0.1, NaN, Infinity.
- `validate_water_command`: **one test per ordered check**, each asserting the
  exact `RejectReason`, plus tests proving the order (e.g. an expired *and*
  oversized command rejects as `expired`, not as clamped).
- Unknown fields ignored; unknown enum values conservative.
- Fixture corpus: every `valid/` file decodes and re-encodes equivalently; every
  `invalid/` file fails with the documented variant.
- `cargo build --no-default-features --target thumbv7em-none-eabi` succeeds.

## Acceptance criteria

- [x] All twelve topic forms build and parse round-trip.
- [x] All twelve message kinds encode and decode round-trip.
- [x] `DeviceId::parse("x/#")` and `DeviceId::parse("Plant-01")` both fail.
- [x] Every ordered check in protocol §5.8 has a test asserting its reason.
- [x] `validate_water_command(requested_ml = 10000)` never returns an `Accept`
      whose `effective_ml` exceeds `FIRMWARE_MAX_ML_PER_RUN`.
- [x] The contract crate compiles for a `no_std` target with default features off.
- [x] `Utc::now()` inside `rhizo-domain` fails clippy.
- [x] Every fixture in `test/fixtures/protocol/` behaves as documented.

## Dependencies

- M0 (workspace, toolchain, CI).

## Open questions

1. **`heapless` vs `alloc` for the dedup ring.** The ring is fixed-size, so
   `heapless` would avoid allocation entirely. Decided in M1-009; `alloc` is
   acceptable since ESP-IDF provides an allocator, and the ring is 16 UUIDs.
   Not blocking.
2. **Whether `ValidationReport` should be returned by value or via an out
   parameter** in the `no_std` path. Cosmetic; resolved in implementation.

## Future work

- Binary encoding (CBOR/postcard) behind a feature flag for LoRaWAN (M14).
- Multi-depth measurement points beyond the reserved `point` field (M14).
- Protocol v2 process is specified in
  [versioning-policy.md](../protocol/versioning-policy.md) but not exercised.
