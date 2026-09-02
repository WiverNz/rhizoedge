# PRD 090 — ESP32 Rust Firmware Foundation

**Milestone:** M9 · **Status:** PLANNED · **Depends on:** M8

> **Revised 2026-08-26.** The firmware now also implements **offline autonomy**
> ([ADR-015](../adr/015-device-offline-autonomy.md)): an NVS policy store with
> atomic activation, the shared evaluator, a bounded tiered event buffer, and a
> monotonic budget that survives reboots conservatively. Issues M9-015…M9-018
> were added.
>
> This is a genuine increase in firmware complexity, in the hardest place to
> debug, and it is accepted deliberately: the alternative is a plant that dies
> because the router rebooted while its owner was away.
>
> The constraints that keep it auditable: the evaluator is the **shared**
> `rhizo-policy` function with one call site; actuation still routes through
> `validate_water_command`; and `src/app/` remains free of `esp_idf_*` imports so
> SAFETY-013…020 are host-testable with no board.
>
> **Additional acceptance criteria:** interruption at every step of a policy
> update leaves exactly one valid policy active; a reboot never replenishes the
> budget or shortens a cooldown; audit events are durable across power loss while
> telemetry may be lost; `event_id` is stable across replay.

> **Revised 2026-08-28 — battery and deep sleep.** "Low power or deep sleep" was
> a non-goal here, deferred to M14. It is now a deliverable:
> [ADR-018](../adr/018-battery-and-deep-sleep-device-mode.md) makes a
> battery-powered Wi-Fi node a supported deployment, and issues M9-019…M9-021 add
> the power mode, the peripheral rails, and the awake hold. **One firmware image
> serves both power modes**, for the same reason there is one
> `validate_water_command`: two images would be two safety paths, and M9-014's
> conformance test would cover only one of them.
>
> The hard part is not `esp_deep_sleep`; it is that sleep destroys RAM while
> SAFETY-015's accounting must survive it honestly. The rule is narrow and stated
> as a single function: a **timer wake with a valid RTC checksum** credits the RTC
> counter's measured elapsed time, and **every other reset reason, and any
> checksum failure, credits zero** — which is SAFETY-015's existing behaviour,
> unchanged. A deep-sleep wake is not a reboot for accounting purposes; a reboot
> is still never a way to earn budget.
>
> **Additional acceptance criteria:** an absent or unrecognised `power.mode`
> yields `AlwaysOn`; `deep_sleep` has exactly one call site, checked
> structurally; the device does not sleep while a dose is in progress or before
> the result is acknowledged; both power rails are off during sleep and driven
> off in the boot-safe sequence; no stabilisation constant for any specific sensor
> part appears in the firmware; battery fields are **absent** on hardware that
> cannot measure them, never zero; no battery field appears in `IrrigationInputs`
> or in any argument to `validate_water_command`.

> **Revised 2026-08-28 — board portability.** F-090-43 previously said only "pin
> assignments in one `board.rs`", which is a filing convention rather than a
> requirement. It is now an explicit portability requirement
> ([ADR-007](../adr/007-esp32-rust-framework-and-toolchain.md), amended):
> **The official Espressif ESP32-C3-DEVKITM-1-N4X is the initial development
> and reference board**, the
> Seeed XIAO ESP32-C3 is a candidate battery-deployment board, and a custom
> ESP32-C3 PCB must stay possible. All three are ESP32-C3, so **changing the
> board must not change application logic, MQTT behaviour, offline policy
> evaluation, command validation, persistence semantics, sensor logic, or any
> safety state machine.**
>
> Board wiring lives behind a board layer selected by a Cargo feature
> (`board-devkitm1`, later `board-xiao-esp32c3`); exactly one profile per build,
> enforced at compile time. **M9 ships the DEVKITM-1 profile only** — what M9
> must deliver is the seam, so that adding the XIAO is a board-profile addition
> rather than a firmware refactor.
>
> **Additional acceptance criteria:** F-090-43…F-090-48 below, checked
> structurally rather than by convention.

## Summary

Replace the simulator's protocol endpoint with real Rust firmware on an
ESP32-C3 — Wi-Fi, MQTT, Edge time sync, NVS, identity, commands, and safety — using fake
sensor and pump adapters so no analogue hardware is required yet.

## Problem

Everything so far assumes the simulator is a faithful stand-in. M9 tests that
assumption. It is also where the project's most unfamiliar toolchain lives, so
it is deliberately scoped to *protocol and safety on real silicon*, with real
sensors and a real pump deferred to M10 and M11.

## Goals

1. A firmware workspace that builds for `riscv32imc-esp-espidf`.
2. Wi-Fi, MQTT with LWT, `edge.time` synchronisation, and NVS working on a real board.
3. Device identity and serial provisioning.
4. Command handling using the **shared** validator.
5. Boot-safe pump state and interrupted-dose reporting.
6. Fake sensor and pump adapters behind traits.
7. Host-testable application logic, independent of any particular board.
8. A board layer that makes a second ESP32-C3 board a profile addition.
9. A conformance test proving the firmware and simulator behave identically.
10. Verified, actually-executed build and flash instructions.

## Non-goals

- Real soil sensors (M10) or a real pump (M11).
- OTA updates, TLS, or certificates (post-V1).
- Light sleep, dynamic frequency scaling, or any low-power technique beyond deep
  sleep. One mechanism, measured, before a second is added.
- Measuring what any of it draws — M10-012, on a board, with a meter. **No
  autonomy figure is stated as a specification by this milestone.**
- Solar, charging, or outdoor power (M14-009).
- PCB design of any kind.
- **A working Seeed XIAO ESP32-C3 profile.** The board is not purchased and
  nothing about it is measured, so M9 delivers `board-devkitm1` and the seam
  that makes `board-xiao-esp32c3` a new file. Writing an unverifiable pin map is
  not the same as portability.
- Supporting a non-ESP32-C3 chip. The board layer is a board abstraction, not a
  chip abstraction; ESP32-S3 remains ADR-007's separate, documented fallback.
- Any irrigation intelligence on the device beyond the shared offline evaluator —
  that remains the edge's
  ([ADR-006](../adr/006-irrigation-state-machine-ownership.md)).

## User/system flows

**Provisioning:**

```text
flash firmware → connect serial → provisioning command writes
   { wifi_ssid, wifi_psk, mqtt_host, device_id?, mqtt_user, mqtt_pass } to NVS
   → reboot → device derives device_id from MAC if not set
   → connects → publishes retained status → edge auto-registers it (no plant)
```

**Command:**

```text
edge publishes command.water
   → firmware validates via validate_water_command
   → NVS write (command_id, started_at, requested_ml)
   → fake pump "runs"
   → command.result published, retried until acked
```

## Functional requirements

### Build and toolchain

| ID | Requirement |
|---|---|
| F-090-01 | Own Cargo workspace, own `rust-toolchain.toml`, excluded from the root workspace. Pins **1.98.0 where the Espressif ecosystem supports it**; otherwise the ESP-compatible toolchain, recorded in [ADR-007](../adr/007-esp32-rust-framework-and-toolchain.md). The host workspace is never downgraded to match. |
| F-090-02 | Target `riscv32imc-esp-espidf`; `esp-idf-svc` / `esp-idf-hal` pinned to exact versions |
| F-090-03 | `cargo build --release` succeeds **with no board attached** |
| F-090-04 | `rhizo-mqtt-contract` and `rhizo-policy` depended on by **path**, `default-features = false` |
| F-090-05 | Build and flash instructions **verified by executing them**, including on Windows; [ADR-007](../adr/007-esp32-rust-framework-and-toolchain.md) corrected from what actually happens |
| F-090-06 | CI job builds the firmware on changes to `firmware/**` or `crates/mqtt-contract/**` |

### Connectivity

| ID | Requirement |
|---|---|
| F-090-10 | Wi-Fi with reconnect, full-jitter backoff base 2 s cap 300 s, unlimited |
| F-090-11 | MQTT with `clean_session = true` and LWT configured **before** connect |
| F-090-12 | Retained `status: online` on connect; heartbeat every `5 × telemetry_interval` |
| F-090-13 | Wall clock synchronised from the Edge via `edge.time` over MQTT (no SNTP client); an `edge.time` **less than or equal to** the last applied one is ignored and does not refresh `synced_at_monotonic`; `clock_synced` reflects synchronisation **age** and is reported truthfully |
| F-090-14 | Subscribes to the eight exact topics of protocol §3 and to no wildcard; never to a topic it publishes |
| F-090-15 | Telemetry buffered across a disconnect to at most 16 samples, then dropped |
| F-090-16 | Command results retried until the **edge** acknowledges them with `command.result.ack` (protocol §5.14) — never retired on the broker's publish ack — and persisted to NVS so an unacknowledged result is republished after reboot |
| F-090-17 | The pending-result ledger is **bounded and durable**, and its saturation behaviour is explicitly designed, documented, and tested rather than defaulted into. **If it is full the firmware fails closed and does not silently discard an unacknowledged watering result in a way that can under-count delivered water** ([ADR-014](../adr/014-failure-and-retry-policy.md) §Device-side pending-result ledger). M9 states whether new actuation is refused while saturated, and with which refusal reason |
| F-090-18 | Saturation is **observable and accounted**: it emits a durable fault or event the edge and an operator can see — never an invisible steady state — and already-delivered water remains attributable and accounted for while it persists, including across reboot and NVS reload at the saturation boundary |
| F-090-19 | Space freed by a `command.result.ack` returns the device to normal operation without losing or double-counting an entry. **No "evict oldest unacknowledged result" policy is adopted unless it is proven safety-equivalent to retaining the entry** — the event buffer's gap-marker precedent does not transfer, because a gap reports a lost *record* while an evicted result silently removes a *quantity the edge's budget is derived from* |

### Identity and configuration

| ID | Requirement |
|---|---|
| F-090-20 | `device_id` read from NVS; derived as `plant-node-<3-byte MAC hex>` on first boot |
| F-090-21 | Serial provisioning writes credentials to NVS — **one firmware image for all devices** |
| F-090-22 | `boot_id` fresh each boot; `sequence` monotonic within a boot |
| F-090-23 | Retained config applied, persisted to NVS, `applied_config_version` echoed |
| F-090-24 | Config with `config_version ≤` applied is ignored |
| F-090-25 | Unrecognised config fields ignored |

### Safety

| ID | Requirement |
|---|---|
| F-090-30 | **The pump GPIO is driven inactive as the first statement in `main`**, before Wi-Fi, before MQTT |
| F-090-31 | The driver is chosen so an un-driven pin is electrically pump-off (gate pull-down), covering the bootloader window |
| F-090-32 | `validate_water_command` is the **only** actuation gate; no second implementation |
| F-090-33 | 16-entry `command_id` dedup ring persisted in NVS; a repeat re-publishes the stored result and does **not** actuate |
| F-090-34 | `(command_id, started_at, requested_ml)` written to NVS **before** actuation |
| F-090-35 | An unfinished NVS dose record on boot produces `status: "interrupted"`, `delivered_ml: null` |
| F-090-36 | `delivered_today_ml` persisted in NVS so the device daily cap survives reboot |
| F-090-37 | A run-duration timer independent of the MQTT task de-energises at `FIRMWARE_MAX_RUN_SECONDS` |
| F-090-38 | Hardware watchdog enabled; a watchdog reset leaves the pump off |
| F-090-39 | Every water command is refused while `clock_synced == false` |

### Power (M9-019…M9-021)

| ID | Requirement |
|---|---|
| F-090-50 | `PowerMode` read from retained config and persisted to NVS; absent or unrecognised yields `AlwaysOn` |
| F-090-51 | Deep sleep with a timer wake source at `wake_interval_seconds`; **exactly one `deep_sleep` call site**, reachable only from the top of the wake loop |
| F-090-52 | The sleep announcement — retained status, `reason: "sleeping"`, `power` block — published and its PUBACK observed **before** sleep is entered |
| F-090-53 | RTC-retained sleep-cycle state with a checksum; a timer wake with a valid checksum credits measured elapsed time, every other reset reason and any checksum failure credits zero (SAFETY-015) |
| F-090-54 | `wake_reason` reported truthfully in `device.status` |
| F-090-55 | Separate `PowerRail`-gated supplies for the RS485 transceiver and the sensor, off during sleep, driven off in the boot-safe sequence, and released by a guard on every error path |
| F-090-56 | `sensor_warmup_ms` taken from configuration; **no compiled-in stabilisation constant for any specific sensor part** (M10-011 measures it) |
| F-090-57 | An awake **hold**, acquired before actuation and released after the `command.result` PUBACK, which gates sleep; `awake_budget_seconds` bounds only an idle wake |
| F-090-58 | `FIRMWARE_MAX_RUN_SECONDS` still ends a run on a timer independent of the hold and of the wake cycle |
| F-090-59 | `battery_voltage` published where measurable and **omitted** where not; `battery_percent` only from a configured chemistry curve; neither is ever an input to a decision |
| F-090-60 | NVS written on change and on watering, not per wake — at ~96 wakes a day NVS endurance is the limiting component; per-wake accounting lives in RTC memory |
| F-090-61 | Always-on behaviour is unchanged: rails enabled, session held, F-090-01…F-090-48 unaffected |

### Architecture

| ID | Requirement |
|---|---|
| F-090-40 | Hardware behind traits: `Pump`, `SoilSensor`, `TankSensor`, `LeakSensor`, `Scale`, `Clock`, `NvsStore`, `PowerRail`, `BatterySensor` |
| F-090-41 | Fake adapters for all of them, usable on the host |
| F-090-42 | `src/app/` contains **no `esp_idf_*` imports** and is host-testable |
| F-090-43 | **The official Espressif ESP32-C3-DEVKITM-1-N4X is the initial development and reference board**, and `board-devkitm1` is the first supported board profile |
| F-090-44 | All board-specific detail is isolated behind the board layer (`src/board/`): GPIO numbers, UART pins, RS485 DE/RE pins, pump-control GPIO, sensor power-enable / load-switch GPIO, tank and leak input pins, active-high/active-low polarity, board-specific peripheral construction, and any board-specific power-control pin |
| F-090-45 | **No file under `src/app/`, `src/safety/`, `src/sensors/`, `src/pump/`, or `src/net/` contains a concrete GPIO number or pin polarity.** Everything above the board layer receives constructed trait objects and cannot observe which board it runs on |
| F-090-46 | Board selection is **compile-time**, by Cargo feature; **exactly one** profile per build, with zero or more than one a `compile_error!` naming the available profiles — never a runtime default and never a runtime pin table |
| F-090-47 | Adding a second ESP32-C3 board is a new board mapping plus a feature entry, with **no change** to application, safety, sensor, pump, or networking code, and no change to the MQTT contract, identity semantics, configuration semantics, or the NVS data model |
| F-090-48 | Once `board-xiao-esp32c3` exists, **both profiles compile against the same application code**, CI builds both, and the host `app/` tests — which are board-independent — produce identical results under either profile |

## Interfaces

MQTT: exactly [docs/protocol/mqtt-v1.md](../protocol/mqtt-v1.md), identical to
the simulator.

```rust
pub trait Pump {
    fn run_for(&mut self, ms: u32) -> Result<(), PumpError>;
    fn off(&mut self);
    fn is_faulted(&self) -> bool;
}
pub trait SoilSensor { fn read(&mut self) -> Result<SoilReading, SensorError>; }
pub trait TankSensor { fn read(&mut self) -> Result<TankReading, SensorError>; }
pub trait LeakSensor { fn read(&mut self) -> Result<bool, SensorError>; }
pub trait Clock { fn now_ms(&self) -> Option<i64>; }   // None = unsynced
pub trait NvsStore {
    fn load(&self) -> Option<PersistedState>;
    fn store(&mut self, s: &PersistedState) -> Result<(), NvsError>;
}
```

`Clock::now_ms` returning `Option` rather than a sentinel is deliberate: an
unsynced clock is not a time, and the type makes forgetting to check impossible.

Serial provisioning:

```text
> provision wifi <ssid> <psk>
> provision mqtt <host> <user> <pass>
> provision device-id <id>        # optional override
> provision show                  # secrets redacted
> provision commit
```

## Data model

NVS layout:

```text
namespace "rhizo"
  device_id            str
  wifi_ssid, wifi_psk  str
  mqtt_host/user/pass  str
  config_version       u32
  config_blob          blob (validated device config)
  delivered_today_ml   f32
  delivered_day_epoch  u32
  cmd_ring             blob (16 × { command_id, outcome })
  in_flight_dose       blob (command_id, started_at, requested_ml) | absent
  pending_results      bounded durable ledger of results the edge has not
                       acknowledged (protocol §5.14); capacity and saturation
                       behaviour are M9 decisions — see F-090-17 and Open
                       question 6. NOT a single slot: a device that waters while
                       the edge is down accumulates entries
  boot_generation      u64 (monotonic across reboot for status ordering)
  policy_active        blob (versioned, validated, checksummed) | absent
  policy_staging       blob + checksum | absent
  offline_runtime      blob (budget/cooldown/last action, conservative across reboot)
  event_buffer         bounded tiered audit/telemetry records
  gap_state            mutable unsent gap or sealed replay marker
  replay_ack           boot id + highest acknowledged device sequence
```

Deliberately identical in content to the simulator's state file
([PRD 020](020-device-simulator.md)), so restart behaviour is comparable
between them.

**One deliberate exception: `pending_results` capacity and overflow.** The
simulator bounds it at 32 entries and evicts the oldest. That is acceptable on a
host — no flash-endurance limit, autonomous doses carry the same volumes through
a second path as `watering.offline_autonomous` audit events, and its job is to
exercise the protocol rather than keep a plant alive. **None of those hold on an
ESP32.** The simulator's constant is not a specification and must not be copied
across; firmware owns this decision and must satisfy F-090-17 on its own terms.

## State model

```text
Boot ──► PumpOff ──► NvsLoad ──► [unfinished dose? → report interrupted]
      ──► WifiConnect ──► MqttConnect ──► Subscribed ──► TimeSynced
      ──► Running

Running:
   telemetry timer  → sample sensors → publish
   command received → validate → (actuate | reject) → publish result
   config received  → validate → apply → persist → status
   edge.time        → if strictly newer → set clock, stamp monotonic, status

Pump: Idle ──► Running(command_id, deadline) ──► Idle
                    │
                    ├─ deadline exceeded ──► Off + Faulted
                    └─ reset/watchdog ─────► (next boot) Interrupted
```

`PumpOff` precedes `NvsLoad`, which precedes everything network-related. The
ordering is the requirement.

`TimeSynced` gates **commands only**, not `Running`. The device samples and
publishes telemetry with no synchronised clock at all — the edge stamps
`received_at` itself — so an unsynchronised node is a degraded actuator, never a
degraded sensor.

## Failure modes

| Failure | Behaviour |
|---|---|
| Wi-Fi unavailable | keep sampling and buffering; retry with backoff; a validated enabled persisted policy may drive only the restricted shared offline evaluator, otherwise no autonomous actuation |
| `edge.time` never received | telemetry continues; **every** water command refused with `clock_unsynced`; status republished at a bounded rate so the edge retries |
| `edge.time` stops arriving | `clock_synced` ages out after `TIME_SYNC_MAX_AGE_SECONDS`; commands refused from that point; monitoring unaffected |
| MQTT broker down | reconnect; telemetry ring caps at 16 samples |
| Safety-critical NVS corrupt | fail closed and report the fault; do not activate defaults, clear dedup uncertainty, replenish budget, shorten cooldown, erase in-flight ambiguity, or grant actuation permission |
| NVS write fails before actuation | **abort the dose**; report `failed`. Never actuate without a durable record. |
| Pending-result ledger saturated | **fail closed** (F-090-17). An unacknowledged result is never silently discarded in a way that can under-count delivered water; saturation is emitted as a durable, visible fault and clears as acknowledgements free space. The exact behaviour — including whether actuation is refused while saturated — is an M9 decision, recorded in the M9 report ([ADR-014](../adr/014-failure-and-retry-policy.md) §Device-side pending-result ledger, Open question 6) |
| Power loss mid-dose | boot → pump off → report `interrupted` |
| Watchdog reset | same |
| Pump run exceeds the limit | independent timer de-energises; `pump_fault`; further commands refused |
| Sensor read error | publish `null` for that field; increment the sensor error counter |
| Heap exhaustion | watchdog reset; pump off on boot |

The NVS-write-failure case is worth stating plainly: if the device cannot record
that it is about to pump, it must not pump.

The saturated-ledger case is the same sentence one step later: **if the device
cannot record what it has already pumped, it must not keep pumping on the
assumption that someone else is counting.** The edge's rolling 24-hour cap is
derived from the rows results produce, so a quietly dropped result under-counts
delivered water — and under-counting is the direction that waters again too
soon. This is why the event buffer's "evict oldest and record a gap" is not
transferable here: a gap tells the edge it is missing a *record*, while a
dropped result leaves the edge's *arithmetic* wrong with nothing to notice.

## Safety implications

M9 moves three invariants from "simulator-verified" to "firmware-verified":

- **SAFETY-002** — F-090-39 and the shared validator's TTL check.
- **SAFETY-007** — F-090-32, F-090-36, F-090-37. The hard limits are compile-time
  constants in the shared crate; no message can change them
  ([ADR-011](../adr/011-configuration-and-secrets-model.md)).
- **SAFETY-011** — F-090-30, -31, -34, -35, -38. This invariant genuinely
  requires firmware; the simulator can only model it.

F-090-31 deserves emphasis because it is the one thing software cannot fix: the
pump must be electrically off when the pin is un-driven, which is true during
reset and during the bootloader window before any Rust runs. If the hardware is
wired the other way, no amount of correct firmware helps.

**SAFETY-001** gains its second enforcement point here (the device-side dedup
ring), complementing the edge's `command_id` primary key.

## Observability

Device-side: status messages carry uptime, free heap, RSSI, sensor health,
error counters, and the compile-time limits. `esp-idf` logging goes to serial at
a configurable level.

Edge-side: the firmware is just another device; every M4 metric applies.

No Prometheus endpoint on the device — status telemetry is the channel, and an
HTTP server on a constrained device would be surface area for no benefit.

## Testing strategy

Four layers, only one needing a board:

1. **Host unit tests** of `src/app/` with fake adapters: boot sequence ordering,
   interrupted-dose detection, dedup ring eviction and persistence, command
   validation dispatch, config version handling, NVS round trip, daily-total
   rollover, and **pending-result ledger saturation** — filling it, asserting
   the fail-closed behaviour F-090-17 requires, power-cycling at the boundary,
   and draining it with acknowledgements. Saturation is reachable on a host with
   fake adapters and needs no board, so there is no excuse for leaving it
   untested until it happens in a plant. This covers SAFETY-002, -007, -011 with
   no hardware.
2. **Compile verification** for the ESP target on every relevant change, for
   every board profile that exists — one in M9, two once the XIAO profile is
   added.
3. **Structural board-isolation check**, run as an ordinary test in the
   firmware workspace: a literal GPIO number or pin polarity outside
   `src/board/` fails the suite. Board isolation that is only a convention stops
   being true the first time somebody is in a hurry.
4. **Conformance (M9-014)** — the same scenario script drives the simulator and
   firmware-with-fake-adapters, asserting identical published message sequences
   modulo ids and timestamps. This is what catches behavioural divergence the
   type system cannot.

With a board attached: HIL-1 and HIL-2 from
[hardware-in-the-loop.md](../testing/hardware-in-the-loop.md).

## Acceptance criteria

- [ ] `cargo build --release` succeeds for `riscv32imc-esp-espidf` with no board.
- [ ] The CI firmware job passes.
- [ ] The official Espressif ESP32-C3-DEVKITM-1-N4X is the initial
      development/reference board, and `board-devkitm1` builds.
- [ ] All board-specific GPIO/peripheral mapping is isolated behind the board
      layer.
- [ ] No file under application, safety, sensor, pump, or networking code
      contains a concrete GPIO number, and the structural check proves it.
- [ ] Firmware application logic does not depend on a specific ESP32-C3
      development board.
- [ ] Selecting zero or two board features fails the build with a clear message.
- [ ] Adding a second ESP32-C3 board requires a new board mapping/profile, not
      changes to application logic.
- [ ] Both supported board profiles compile against the same application code
      once the second board is introduced, and switching profiles changes no
      MQTT, domain, or safety test result.
- [ ] [ADR-007](../adr/007-esp32-rust-framework-and-toolchain.md)'s toolchain
      section has been **executed** and corrected, including on Windows.
- [ ] Host tests cover boot safety, interrupted dose, dedup ring, and command
      validation.
- [ ] The pending-result ledger's capacity and saturation behaviour are
      **decided and written down** — in the M9 report and in this PRD's Open
      question 5 — rather than left implicit in the ring implementation.
- [ ] A saturated ledger **fails closed**: no unacknowledged watering result is
      silently discarded in a way that can under-count delivered water, and the
      decision on whether new actuation is refused while saturated is stated
      with its refusal reason.
- [ ] Saturation emits a durable fault or event visible to the edge and to an
      operator, and already-delivered water stays attributable while it lasts.
- [ ] The ledger's state at saturation survives a reboot, and a power cycle at
      the boundary neither drops nor duplicates a result.
- [ ] Acknowledgement frees space and restores normal operation with no entry
      lost or double-counted.
- [ ] If any eviction of an unacknowledged result is adopted, the M9 report
      **argues its safety equivalence explicitly**; absent that argument, the
      answer is that the firmware does not evict.
- [ ] The conformance test shows identical behaviour to the simulator.
- [ ] **With a board:** it connects, appears online in the edge API, publishes
      telemetry from fake sensors, applies retained config, and echoes
      `applied_config_version`.
- [ ] **With a board:** a duplicate `command_id` across a power cycle is still
      deduplicated (the NVS ring survived).
- [ ] **With a board:** an oversized command is clamped or rejected.
- [ ] **With a board:** HIL-1 passes — the pump line never asserts across 20
      resets, a watchdog reset, and 10 mid-boot power cuts.
- [ ] **With a board:** withholding `edge.time` causes every water command to be
      refused while telemetry continues.
- [ ] Offline policy staging/activation is atomic and a corrupt update never
      replaces the last valid policy.
- [ ] The sole firmware `evaluate_offline` call site uses persisted conservative
      budget/cooldown state and still routes any dose through the shared actuation gate.
- [ ] Audit events, sealed history gaps, replay progress, and cumulative
      `event.ack` handling survive reboot with stable `event_id` values.

The board-dependent criteria are marked so the milestone can be substantially
completed and reviewed before hardware arrives.

## Dependencies

- M8 (a proven software system to compare against).
- M1 (the shared contract and validator).
- Hardware: one official **Espressif ESP32-C3-DEVKITM-1-N4X** and a USB data
  cable. Nothing analogue. Any
  ESP32-C3 board works for the host and compile criteria; the board-dependent
  criteria — HIL-1 in particular — assume the DEVKITM-1's exposed pins. A real
  board is required to complete the hardware-verification criteria honestly.

## Open questions

1. ~~**Exact `esp-idf-svc` version.**~~ **Resolved 2026-09-02 (M9-001):**
   `esp-idf-svc` 0.52.1, `esp-idf-hal` 0.46.2, `esp-idf-sys` 0.37.2,
   `embedded-svc` 0.29.0, ESP-IDF v5.5.1. All pinned with `=` in
   `firmware/esp32-node/Cargo.toml`; the full table is in
   [ADR-007](../adr/007-esp32-rust-framework-and-toolchain.md).
2. ~~**Whether stock Rust 1.98.0 can build `riscv32imc-esp-espidf` directly.**~~
   **Resolved 2026-09-02 (M9-001): it cannot.** `rustup target add` refuses the
   triple on 1.98.0 — it is a recognised tier-3 target with no distributed
   `std` — so `std` is built from source with `-Z build-std=std,panic_abort`,
   which is nightly-only. The firmware image workspace pins
   `nightly-2026-07-01`. The host workspace is unchanged at 1.98.0, and so is
   `firmware/node-app`, where the safety logic and its tests live.
3. ~~**Windows toolchain friction.**~~ **Resolved 2026-09-02 (M9-001), with
   three findings**, all recorded in ADR-007: `espup install --targets esp32c3`
   installs no `libclang` and the build fails in bindgen six minutes in, so
   `--targets all --std` is required; the export script's `PATH` entry is
   load-bearing and not just `LIBCLANG_PATH`; and `esp-idf-sys` refuses an
   output directory over 88 characters, so `CARGO_TARGET_DIR` must be short.
   With those three the native Windows build works, and the Linux-container
   fallback remains documented and is what M9-002's CI job exercises.
4. **Telemetry ring size (16)** — a balance between RAM and gap tolerance. Easily
   tuned; nothing depends on it.
5. ~~**The pending-result ledger's capacity and saturation behaviour.**~~
   **Resolved 2026-09-02 (M9-011): capacity 16, no eviction, actuation refused
   at 15.**

   - **Refusal while saturated:** yes, with `RejectReason::ResultLedgerFull`, a
     variant added additively to the shared contract (protocol §5.8 step 13a,
     §5.10, §9). The check runs **after** `validate_water_command` has already
     accepted, so the shared gate stays the only gate and this can only stop a
     dose. A device that cannot record what it delivered does not deliver more.
   - **Already-delivered water:** nothing is evicted, so every entry is still
     held; and `delivered_today_ml` rides on every result and in
     `device.status`, giving the edge a running total to reconcile against.
   - **The durable fault:** latched once on the crossing into saturation and
     cleared on the crossing back — a state, not one event per refused command
     — reported in status with the volume the edge has not yet counted.
   - **Recovery:** per `command_id`. An acknowledgement removes exactly the
     named entry, one for an entry not held is a no-op, and freeing a slot
     below the threshold clears the fault. Nothing is re-keyed, so nothing can
     be double-counted.
   - **Reboot at the boundary:** the ledger is persisted state, written before
     the publish, in the inactive of two CRC-protected NVS slots. A power cut
     leaves the previous complete state; re-publishing is expected and the edge
     deduplicates on `command_id`.
   - **Eviction:** **not adopted**, and no safety-equivalence argument is
     offered, because there is not one to make.

   **Why 16, and why the reserve.** 16 matches `COMMAND_DEDUP_RING`: a deeper
   ledger could hold a result for a command the ring had forgotten. Actuation
   stops one slot early because a refusal is itself a `command.result` and needs
   somewhere to live — without the reserve the device could reach a state where
   it cannot record the refusal it just issued.

   Tested in `firmware/node-app/src/ledger.rs` and
   `src/command.rs`: filling it, asserting the refusal and its reason,
   power-cycling at the boundary, and draining it with acknowledgements.

6. **Which board actually gets deployed on battery.** The XIAO ESP32-C3 is the
   candidate; a custom ESP32-C3 PCB is the plausible end state. Deliberately
   unresolved here — M10-012 measures, and the board layer means the answer
   costs a file rather than a refactor.

## Future work

- Real sensors (M10), real pump (M11).
- A `board-xiao-esp32c3` profile, written when the board is purchased and
  measured, and a custom ESP32-C3 PCB after that.
- OTA updates, signed firmware, TLS, per-device certificates (post-V1).
- Light sleep and further power techniques, if M10-012's measurements show deep
  sleep alone does not reach the target.
- `no_std` firmware for battery nodes, reconsiderable per
  [ADR-007](../adr/007-esp32-rust-framework-and-toolchain.md) — though deep-sleep
  current is dominated by board hardware rather than by whether the firmware links
  `std`, so it is likely the wrong lever.
