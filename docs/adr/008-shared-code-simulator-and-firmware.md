# ADR-008 — Shared code between simulator and firmware

## Status

Accepted — 2026-08-25. Contract/mechanics in M1/M2, evaluator and simulator
integration in M6, firmware integration exercised in M9.

**Extended 2026-08-26.** A **second** crate is now shared with the firmware:
`rhizo-policy`, holding the offline evaluator
([ADR-015](015-device-offline-autonomy.md)). The reasoning is identical to the
reasoning for the shared command validator, and so is the rule: one
implementation, one call site per consumer, no second copy of the decision.

## Context

The project's central bet is that the Device Simulator can stand in for the
ESP32 for milestones M0–M8. That bet only pays off if the two are genuinely
interchangeable. If they diverge, then every test run against the simulator is
testing a system that does not exist, and the divergence is discovered at the
worst possible moment — with real water and a real plant.

The specific danger: the simulator being **more permissive** than the firmware.
A test suite that passes against a lenient simulator gives false confidence
about SAFETY-002 and SAFETY-007.

They live in separate Cargo workspaces ([ADR-001](001-rust-workspace-and-crate-boundaries.md)),
so sharing needs a deliberate mechanism.

## Decision

### Two crates are shared: `rhizo-mqtt-contract` and `rhizo-policy`

The firmware depends on it by **path**, with default features off:

```toml
# firmware/esp32-node/Cargo.toml
[dependencies]
rhizo-mqtt-contract = { path = "../../crates/mqtt-contract", default-features = false }
```

```toml
rhizo-policy = { path = "../../crates/policy", default-features = false }
```

A path dependency, not a version dependency: there is exactly one copy of the
source on disk, so the firmware and the host workspace cannot drift to different
versions. The cost is that the firmware workspace is not independently
publishable, which nothing requires.

**`rhizo-policy` is shared for exactly the same reason as the validator.** If the
firmware had its own offline evaluator, every offline safety test in M6 and every
isolation scenario in M8 would be exercising rules the hardware does not follow —
the identical failure this ADR exists to prevent, one layer up. The Edge links it
too, which additionally lets it reject a policy it cannot evaluate and predict
what an isolated device will do.

Milestone sequencing is deliberate: M2 persists policies and evaluator state,
models isolation/reconnection, and buffers/replays history, but makes no offline
watering decision. M6-019 implements `rhizo-policy::evaluate_offline` once and
adds the simulator's sole call site. M9 adds the firmware's sole call site. At no
point may the simulator or firmware contain its own copy of the rules.

### What is shared

| Shared | Not shared |
|---|---|
| Envelope and payload types | MQTT client (rumqttc vs esp-idf-svc) |
| Topic grammar (build + parse) | Sensor drivers |
| `DeviceId` validation | Pump drivers |
| Protocol version and compatibility rules | Storage |
| Physical range constants | Irrigation logic (edge only) |
| **`validate_water_command`** | Networking, Wi-Fi, NVS |
| **Firmware hard-limit constants** | The physical model (simulator only) |

The two bolded rows are the ones that make the interchangeability claim true.

### The shared validator

```rust
// crates/mqtt-contract/src/safety.rs   —  no_std, no allocation
pub const FIRMWARE_MAX_RUN_SECONDS: u32 = 20;
pub const FIRMWARE_MAX_ML_PER_RUN: f32 = 80.0;
pub const FIRMWARE_MAX_DAILY_ML: f32 = 500.0;
pub const MAX_CLOCK_SKEW_SECONDS: i64 = 5;

pub struct DeviceGuardState {
    pub clock_synced: bool,
    pub now_ms: i64,
    pub delivered_today_ml: f32,
    pub leak: bool,
    pub tank_percent: Option<f32>,
    pub tank_min_percent: f32,
    pub pump_faulted: bool,
    pub recent_command_ids: [Option<Uuid>; COMMAND_DEDUP_RING],
}

pub enum CommandVerdict {
    Accept { effective_ml: f32, run_ms: u32, clamped: bool },
    Reject(RejectReason),
    AlreadyExecuted { previous: CommandOutcome },
}

pub enum RejectReason {
    ClockUnsynced,      // SAFETY-002 + SAFETY-012
    Expired,            // SAFETY-002
    OverSingleMax,      // SAFETY-007
    OverDailyMax,       // SAFETY-007
    LeakDetected,       // SAFETY-003
    TankLow,            // SAFETY-004
    TankUnknown,        // SAFETY-012
    PumpFaulted,
    MalformedCommand,
}

pub fn validate_water_command(
    cmd: &WaterCommand,
    state: &DeviceGuardState,
) -> CommandVerdict;
```

**The simulator and the firmware both call this function and nothing else** to
decide whether to actuate. Neither has its own copy of the rules. The simulator
cannot be more permissive than the firmware because there is no second
implementation to be permissive in.

It is `no_std` and allocation-free so the firmware can call it in a constrained
context, and it takes an explicit `DeviceGuardState` rather than reading
hardware, so it is trivially unit-testable on the host.

### Shared protocol fixtures

`test/fixtures/protocol/` holds canonical JSON files — one per message kind,
plus a set of deliberately malformed ones:

```text
test/fixtures/protocol/
├── valid/
│   ├── telemetry_soil_full.json
│   ├── telemetry_soil_partial.json      # optional fields absent
│   ├── status_online.json
│   ├── status_lwt_offline.json
│   ├── config_v7.json
│   ├── command_water.json
│   └── command_result_completed.json
└── invalid/
    ├── unknown_version.json
    ├── device_id_mismatch.json
    ├── moisture_out_of_range.json
    ├── missing_message_id.json
    └── topic_injection_device_id.json
```

Both workspaces run the same fixture tests:

- the host workspace via `crates/mqtt-contract/tests/fixtures.rs`
- the firmware workspace via `firmware/esp32-node/tests/fixtures.rs`, which
  runs **on the host** (no board needed) against the same files

A change to the contract that breaks decoding fails in both places. Fixtures in
`valid/` must decode and re-encode to an equivalent value; fixtures in
`invalid/` must be rejected with the documented error variant.

### Firmware logic that cannot be shared is still host-tested

`firmware/esp32-node/src/app/` contains the orchestration — command handling,
dedup ring management, NVS state transitions, telemetry assembly — and imports
no `esp_idf_*` symbols. It talks to hardware only through traits:

```rust
pub trait Pump  { fn run_for(&mut self, ms: u32) -> Result<(), PumpError>; fn off(&mut self); }
pub trait SoilSensor { fn read(&mut self) -> Result<SoilReading, SensorError>; }
pub trait Clock { fn now_ms(&self) -> Option<i64>; }   // None = unsynced
pub trait NvsStore { fn load(&self) -> Option<PersistedState>; fn store(&mut self, s: &PersistedState); }
```

`cargo test` in the firmware workspace runs these on the host with fake
adapters, covering SAFETY-011 (boot-safe state, interrupted-dose reporting)
without a board.

### Preventing skew — the mechanisms

1. **Path dependency** — one source of truth on disk, for both shared crates.
2. **One validator and one offline evaluator** — no second implementation of
   either rule set.
3. **Shared fixtures** — both workspaces decode the same bytes.
4. **CI builds the firmware whenever the contract changes** (M9-002); a
   `no_std` compile check runs on every change (M1-011).
5. **A conformance test** (M9-014): the same scenario script runs against the
   simulator and against firmware-with-fake-adapters, asserting identical
   published message sequences modulo ids and timestamps.

Mechanism 5 is the one that would catch a behavioural divergence the type system
cannot.

## Alternatives considered

**Duplicate the types in the firmware.** Rejected: guarantees divergence. This
is the failure mode the whole ADR exists to prevent.

**Publish `rhizo-mqtt-contract` to crates.io and depend by version.** Rejected
for V1: adds a release step to every protocol change and permits the firmware to
lag behind by a version. Revisit if the firmware ever needs to be built outside
this repository.

**Share `rhizo-domain` too.** Rejected: the device deliberately has no
irrigation intelligence ([ADR-006](006-irrigation-state-machine-ownership.md)).
Sharing it would invite that intelligence onto the device and would force the
domain crate to be `no_std` for no benefit.

**Generate the contract types from a schema (JSON Schema, protobuf).** Rejected
as premature: one hand-written `no_std` Rust crate is simpler than a code
generator, and there is no second language to generate for — precisely because
there is no Go, C++, or TypeScript in this project.

**A single workspace with target-specific dependencies.** Rejected in
[ADR-001](001-rust-workspace-and-crate-boundaries.md): Cargo feature unification
across a workspace makes `no_std` guarantees unreliable.

## Consequences

Positive:

- The simulator provably enforces the same command rules as the firmware, which
  is what makes SAFETY-002 and SAFETY-007 meaningful in M6 — three milestones
  before hardware exists.
- Firmware safety logic is covered by fast host tests.
- A protocol change is one edit, and both consumers fail loudly if it is wrong.

Negative, accepted:

- `rhizo-mqtt-contract` must stay `no_std` and allocation-frugal forever. This
  is a real constraint on contributors and is guarded by a CI job.
- The firmware workspace cannot be relocated out of this repository without
  changing the dependency style.
- Hard limits being compile-time constants means changing them requires
  reflashing every device. This is intentional (SAFETY-007) but is a genuine
  operational cost worth stating plainly.

## Risks

- **The simulator grows a shortcut** — someone adds a `--unsafe-allow-any-dose`
  flag for debugging and it survives into a test config. *Mitigation:* the
  simulator has no code path to actuate that does not go through
  `validate_water_command`; a flag would have to remove the call, which is a
  visible change to a safety-critical file. Test `safety_007_simulator_refuses_like_hardware`
  asserts the refusal directly.
- **Fixture rot** — fixtures stop reflecting real messages. *Mitigation:*
  M2-011 adds a mode that captures live simulator output and diffs it against
  the fixtures, so drift is detected rather than assumed absent.
- **`no_std` regression via a transitive dependency.** *Mitigation:* the M1-011
  CI job builds the contract crate for a bare-metal target on every change.

## Follow-up

- M1-009 implements `validate_water_command` and its unit tests.
- M1-010 creates the fixture corpus.
- M1-011 adds the `no_std` compile job.
- M2-011 adds fixture-drift detection.
- M9-014 adds the simulator/firmware conformance test.
