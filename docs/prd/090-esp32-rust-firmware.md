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
7. Host-testable application logic.
8. A conformance test proving the firmware and simulator behave identically.
9. Verified, actually-executed build and flash instructions.

## Non-goals

- Real soil sensors (M10) or a real pump (M11).
- OTA updates, TLS, or certificates (post-V1).
- Low power or deep sleep (M14).
- Any irrigation intelligence on the device — that remains the edge's
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
| F-090-04 | `rhizo-mqtt-contract` depended on by **path**, `default-features = false` |
| F-090-05 | Build and flash instructions **verified by executing them**, including on Windows; [ADR-007](../adr/007-esp32-rust-framework-and-toolchain.md) corrected from what actually happens |
| F-090-06 | CI job builds the firmware on changes to `firmware/**` or `crates/mqtt-contract/**` |

### Connectivity

| ID | Requirement |
|---|---|
| F-090-10 | Wi-Fi with reconnect, full-jitter backoff base 2 s cap 300 s, unlimited |
| F-090-11 | MQTT with `clean_session = true` and LWT configured **before** connect |
| F-090-12 | Retained `status: online` on connect; heartbeat every `5 × telemetry_interval` |
| F-090-13 | Wall clock synchronised from the Edge via `edge.time` over MQTT (no SNTP client); an `edge.time` **less than or equal to** the last applied one is ignored and does not refresh `synced_at_monotonic`; `clock_synced` reflects synchronisation **age** and is reported truthfully |
| F-090-14 | Subscribes to the seven exact topics of protocol §3 and to no wildcard; never to a topic it publishes |
| F-090-15 | Telemetry buffered across a disconnect to at most 16 samples, then dropped |
| F-090-16 | Command results retried up to 60 s, then persisted to NVS and republished after reboot |

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

### Architecture

| ID | Requirement |
|---|---|
| F-090-40 | Hardware behind traits: `Pump`, `SoilSensor`, `TankSensor`, `LeakSensor`, `Scale`, `Clock`, `NvsStore` |
| F-090-41 | Fake adapters for all of them, usable on the host |
| F-090-42 | `src/app/` contains **no `esp_idf_*` imports** and is host-testable |
| F-090-43 | Pin assignments in one `board.rs` |

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
  pending_result       blob | absent
```

Deliberately identical in content to the simulator's state file
([PRD 020](020-device-simulator.md)), so restart behaviour is comparable
between them.

## State model

```text
Boot ──► PumpOff ──► NvsLoad ──► [unfinished dose? → report interrupted]
      ──► WifiConnect ──► MqttConnect ──► Subscribed ──► TimeSynced
      ──► Running

Running:
   telemetry timer  → sample sensors → publish
   command received → validate → (actuate | reject) → publish result
   config received  → validate → apply → persist → status
   edge.time        → if >= last applied → set clock, stamp monotonic, status

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
| Wi-Fi unavailable | keep sampling, retry with backoff, pump stays off; no autonomous watering |
| `edge.time` never received | telemetry continues; **every** water command refused with `clock_unsynced`; status republished at a bounded rate so the edge retries |
| `edge.time` stops arriving | `clock_synced` ages out after `TIME_SYNC_MAX_AGE_SECONDS`; commands refused from that point; monitoring unaffected |
| MQTT broker down | reconnect; telemetry ring caps at 16 samples |
| NVS corrupt | start with defaults, log, publish a `nvs_reset` event — a corrupt store must not block boot but must not be trusted |
| NVS write fails before actuation | **abort the dose**; report `failed`. Never actuate without a durable record. |
| Power loss mid-dose | boot → pump off → report `interrupted` |
| Watchdog reset | same |
| Pump run exceeds the limit | independent timer de-energises; `pump_fault`; further commands refused |
| Sensor read error | publish `null` for that field; increment the sensor error counter |
| Heap exhaustion | watchdog reset; pump off on boot |

The NVS-write-failure case is worth stating plainly: if the device cannot record
that it is about to pump, it must not pump.

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

Three layers, only one needing a board:

1. **Host unit tests** of `src/app/` with fake adapters: boot sequence ordering,
   interrupted-dose detection, dedup ring eviction and persistence, command
   validation dispatch, config version handling, NVS round trip, daily-total
   rollover. This covers SAFETY-002, -007, -011 with no hardware.
2. **Compile verification** for the ESP target on every relevant change.
3. **Conformance (M9-014)** — the same scenario script drives the simulator and
   firmware-with-fake-adapters, asserting identical published message sequences
   modulo ids and timestamps. This is what catches behavioural divergence the
   type system cannot.

With a board attached: HIL-1 and HIL-2 from
[hardware-in-the-loop.md](../testing/hardware-in-the-loop.md).

## Acceptance criteria

- [ ] `cargo build --release` succeeds for `riscv32imc-esp-espidf` with no board.
- [ ] The CI firmware job passes.
- [ ] [ADR-007](../adr/007-esp32-rust-framework-and-toolchain.md)'s toolchain
      section has been **executed** and corrected, including on Windows.
- [ ] Host tests cover boot safety, interrupted dose, dedup ring, and command
      validation.
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

The board-dependent criteria are marked so the milestone can be substantially
completed and reviewed before hardware arrives.

## Dependencies

- M8 (a proven software system to compare against).
- M1 (the shared contract and validator).
- Hardware: one ESP32-C3 board and a USB cable. Nothing analogue.

## Open questions

1. **Exact `esp-idf-svc` version.** 0.52.x at the time of writing; pinned in
   M9-001 to whatever actually builds.
2. **Whether stock Rust 1.98.0 can build `riscv32imc-esp-espidf` directly**, or
   whether the espup-provided channel is required. `riscv32imc-esp-espidf` is a
   tier-3 target, so `-Z build-std` may still be needed. M9-001 resolves this
   empirically; either outcome is contained to the firmware workspace.
3. **Windows toolchain friction.** The primary machine is Windows and ESP-IDF is
   better exercised on Linux. M9-001 records the real procedure; M9-002 covers
   the documented fallback of building in a Linux container and flashing from
   the host.
4. **Telemetry ring size (16)** — a balance between RAM and gap tolerance. Easily
   tuned; nothing depends on it.

## Future work

- Real sensors (M10), real pump (M11).
- OTA updates, signed firmware, TLS, per-device certificates (post-V1).
- Deep sleep and battery operation ([PRD 140](140-field-readiness.md)).
