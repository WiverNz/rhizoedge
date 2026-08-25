# Testing Strategy

The project's claim is that a system can be trusted with a real plant. That
claim rests on tests, and specifically on tests of things that are hard to test:
duplicate delivery, crash recovery, clock anomalies, and hardware refusing to
cooperate.

---

## 1. The test pyramid, and where the weight goes

```text
        ┌───────────────────────────────┐
        │ Hardware-in-the-loop  (M11+)  │  a handful, manual
        ├───────────────────────────────┤
        │ End-to-end scenarios  (M8)    │  ~12, Docker, minutes
        ├───────────────────────────────┤
        │ Integration  (M3+)            │  ~40, real broker + SQLite, seconds
        ├───────────────────────────────┤
        │ Property / invariant  (M6)    │  ~15, pure, milliseconds  ◄ the weight
        ├───────────────────────────────┤
        │ Unit                          │  hundreds, microseconds
        └───────────────────────────────┘
```

Unusually, the **property layer carries the most safety weight**, not the
end-to-end layer. This is a direct consequence of
[ADR-006](../adr/006-irrigation-state-machine-ownership.md): the irrigation
state machine is a pure function, so exhaustive randomised testing of the safety
gate costs milliseconds. An end-to-end test can show that one scenario works; a
property test can show that ten thousand scenarios cannot violate SAFETY-006.

---

## 2. Naming and discoverability

```text
safety_NNN_<description>     proves a SAFETY-nnn invariant   → cargo test safety_
prop_<description>           property test
it_<description>             integration test
scenario_<name>              end-to-end scenario
```

`cargo test safety_` runs the entire safety suite. A milestone that claims to
enforce an invariant must have its `safety_NNN_*` test green — that is the
evidence, and no other form of evidence is accepted.

---

## 3. Unit tests

Fast, pure, no I/O. Located beside the code (`#[cfg(test)] mod tests`).

Coverage targets by crate:

**`rhizo-mqtt-contract`**
- envelope encode/decode round trip for every kind
- topic build/parse round trip; malformed topics rejected
- `DeviceId` grammar: valid cases, and specifically `x/#`, `+`, uppercase,
  too short, too long, leading/trailing hyphen
- range validation at boundaries: 0.0, 100.0, 100.1, −0.1, NaN, Infinity
- `validate_water_command` — every branch of §5.8's ordered checks, each
  asserting the exact `RejectReason`
- unknown fields ignored; unknown enum values decode to the conservative variant

**`rhizo-domain`**
- every irrigation state transition, including illegal ones
- safety gate: each lockout reason triggered in isolation
- recommendation engine: reason lists asserted structurally
- moisture trend and manual-watering detection
- pump duration arithmetic (`ml / ml_per_second`), including clamping
- profile validation rejections

**`rhizo-storage`**
- each repository method against an in-memory SQLite
- the dedup transaction: insert, duplicate, rollback-leaves-nothing
- migration idempotency

**`rhizo-telemetry`**
- backoff: monotonic bounds, cap respected, jitter within range, reset on success
- `classify()` for every error variant

### Determinism rule

Unit tests use `TestClock` exclusively. **No test sleeps to advance logical
time.** A test that needs real elapsed time must justify it in a comment; the
default review response is "advance the `TestClock`".

---

## 4. Property and invariant tests

Using `proptest`. This is where the safety argument lives.

| Test | Invariant | Generated inputs |
|---|---|---|
| `safety_006_rolling_24h_cap_never_exceeded` | SAFETY-006 | random command/result/restart/clock-jump sequences over 72 simulated hours |
| `safety_012_missing_input_never_waters` | SAFETY-012 | `IrrigationInputs` with each field randomly `None` |
| `safety_003_leak_blocks_all_modes` | SAFETY-003 | random states × random modes with leak asserted |
| `safety_004_tank_unknown_or_low_blocks` | SAFETY-004 | random tank values including `None` |
| `safety_005_stale_or_invalid_blocks_auto` | SAFETY-005 | random sample ages and validity |
| `safety_002_expired_never_accepted` | SAFETY-002 | random issue/expiry/now triples |
| `safety_007_clamp_never_exceeds_hard_max` | SAFETY-007 | random `requested_ml` including absurd values |
| `safety_010_terminal_commands_never_reissued` | SAFETY-010 | random command histories with restarts |
| `prop_state_machine_total` | — | every state × every input reaches a defined state |
| `prop_dedup_idempotent` | SAFETY-001 | random duplication of a message stream |

The flagship is `safety_006_rolling_24h_cap_never_exceeded`. It generates
adversarial histories — restarts between publish and result, clock steps,
interrupted doses credited conservatively — and asserts the rolling sum never
exceeds the cap. If one property test is kept, it is that one.

**Regression corpus.** `proptest` failures are persisted to
`proptest-regressions/` and committed. A shrunk counterexample is permanent
evidence and must keep passing.

---

## 5. Integration tests

Real Mosquitto (via `testcontainers` or a compose-managed instance), real
SQLite, real edge code. No mocks of the broker or the database — mocking them
would test the mock's model of QoS 1 redelivery rather than the real thing.

Representative set:

```text
it_telemetry_ingested_and_persisted
it_duplicate_qos1_creates_one_row              → SAFETY-001
it_malformed_payload_quarantined
it_partial_invalid_fields_stored_as_null
it_lwt_marks_device_offline
it_retained_status_seen_on_subscribe
it_retained_config_delivered_to_late_device
it_no_retained_message_on_command_topics       → ADR-002
it_device_acl_cannot_publish_as_other_device   → ADR-012
it_broker_restart_resubscribes
it_edge_restart_preserves_history
it_edge_restart_no_command_replay              → SAFETY-010
it_command_result_updates_state
it_manual_water_blocked_by_leak_409            → SAFETY-003
it_cloud_down_local_operation_continues        → SAFETY-008
it_cloud_recovery_drains_outbox_once
it_simulator_refuses_oversized_command         → SAFETY-007
```

### The one non-negotiable integration test

`it_simulator_refuses_oversized_command` asserts that the simulator refuses a
command the firmware would refuse. If the simulator were more permissive, every
test above would be testing a system that does not exist
([ADR-008](../adr/008-shared-code-simulator-and-firmware.md)).

---

## 6. End-to-end scenarios (M8)

Full Docker topology, accelerated virtual time, one command:

```bash
docker compose -f deploy/docker-compose.yml -f deploy/docker-compose.test.yml \
  up --abort-on-container-exit --exit-code-from scenario-runner
```

Required scenarios are enumerated in
[failure-scenarios.md](failure-scenarios.md). The headline one:

```text
scenario_full_watering_cycle
  dry soil → recommendation → dose → absorption wait → recheck
  → second dose → recovered → Normal
```

and

```text
scenario_cloud_outage_recovery
  cloud stopped → events queue → local operation verified
  → cloud restarted → outbox drains → no duplicates in PostgreSQL
```

Scenarios assert on **observable state** — API responses, database rows, MQTT
messages captured by a spy subscriber — never on log strings.

---

## 7. Firmware testing (M9+)

Three layers, only the third needing hardware:

1. **Host unit tests** of `firmware/esp32-node/src/app/` with fake `Pump`,
   `SoilSensor`, `Clock`, and `NvsStore` adapters. Covers boot-safe state,
   interrupted-dose reporting, dedup ring behaviour, and command validation —
   SAFETY-002, -007, -011 without a board.
2. **Compile verification** for `riscv32imc-esp-espidf` on every change to
   `firmware/**` or `crates/mqtt-contract/**`.
3. **Conformance test** (M9-014): the same scenario script drives the simulator
   and firmware-with-fake-adapters, asserting identical published message
   sequences modulo ids and timestamps.

Layer 3 is what catches behavioural divergence the type system cannot.

---

## 8. Hardware-in-the-loop (M11+)

Manual, checklist-driven, documented in
[hardware-in-the-loop.md](hardware-in-the-loop.md).

Non-negotiable rule: **the first automatic watering test targets a measuring
cup, never a plant.**

---

## 9. The testkit

`crates/testkit` provides:

- `TestClock` with `set` / `advance`
- envelope and payload builders with sane defaults
  (`SoilTelemetryBuilder::new().moisture(24.0).build()`)
- an in-memory SQLite fixture with migrations applied
- a `MqttSpy` that records everything published on a pattern
- a fault-injection harness for the simulator
- profile fixtures (`profiles::monstera()`, `profiles::fast_drying()`)
- assertion helpers: `assert_locked_out(plant, LockoutReason::Leak)`,
  `assert_no_watering_events_since(t)`, `assert_delivered_within_24h(plant, ml)`

The testkit exists so that a new safety test is a few lines rather than a
morning of setup — the difference between safety tests that get written and
safety tests that get discussed.

---

## 10. CI gates

Every change:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo build -p rhizo-mqtt-contract --no-default-features --target thumbv7em-none-eabi
docker compose -f deploy/docker-compose.yml config
cargo run --manifest-path tools/docscheck/Cargo.toml
```

Additional jobs:

| Job | Trigger | Blocking |
|---|---|---|
| integration tests (with broker) | every change | yes |
| end-to-end scenarios | every change to `crates/**` or `deploy/**` | yes, from M8 |
| firmware build (`riscv32imc-esp-espidf`) | `firmware/**` or `crates/mqtt-contract/**` | yes, from M9 |
| UI build (wasm + Tauri) | `ui/**` | yes, from M12 |
| Docker image build | every change | yes |

**No milestone is complete while its acceptance tests are red.** This is stated
in every milestone's exit criteria and is the only definition of "done" the
roadmap recognises.

---

## 11. What is deliberately not tested

- **Coverage percentage as a target.** Coverage is measured and reported, not
  gated. A gate encourages tests of trivial getters while the property tests
  that matter are unaffected by the number.
- **The simulator's physical model against real soil.** It is a plausible
  approximation for exercising control logic, not a soil-physics claim. Real
  behaviour is validated in M10/M11 against hardware.
- **Mosquitto itself, `sqlx`, or `rumqttc`.** Their behaviour is exercised
  through integration tests but not asserted independently.
- **UI rendering.** Manual verification in M12; automated browser testing would
  require the JS tooling this project excludes.
