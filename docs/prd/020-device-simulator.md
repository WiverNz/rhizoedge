# PRD 020 — Device Simulator

**Milestone:** M2 · **Status:** DELIVERED · **Depends on:** M1

> **Boundary clarified 2026-08-27.** M2 models every device-side mechanic needed
> for later offline autonomy: capabilities, atomic policy persistence,
> monotonic runtime-state persistence, isolation/reconnect behaviour, and the
> bounded replay buffer. It deliberately does **not** evaluate the policy or
> schedule an autonomous dose. M6-019 implements the one shared
> `rhizo_policy::evaluate_offline` function and integrates its sole simulator
> call site. Issues M2-015…M2-018 remain device-mechanics work.
>
> The permissiveness rule remains structural: M2 calls
> `validate_water_command`, but contains **no simulator-specific offline
> evaluator**. When M6 activates offline decisions, the simulator calls the
> shared `rhizo_policy::evaluate_offline`; firmware later calls the same function.
>
> **Additional acceptance criteria:** an isolated M2 simulator continues
> sampling and buffering but never autonomously waters; policy activation and
> acknowledgement survive restart; audit events survive a telemetry flood; a
> replayed batch is byte-identical on `event_id` across replays.

## Summary

A host Rust binary that behaves like an ESP32 plant node, speaks the identical
MQTT protocol, models soil and water plausibly, injects faults on demand, and
runs on accelerated virtual time. It is the component that makes M3–M8
achievable without hardware.

## Problem

Building the edge controller against nothing means testing it against nothing.
Building it against a mock MQTT client means testing a model of MQTT rather than
MQTT. And building it against a *lenient* simulator is the worst outcome of all:
a green test suite for a system that does not exist, with the divergence
discovered when real water meets a real floor.

## Goals

1. A device indistinguishable from firmware at the protocol level.
2. A physical model good enough to exercise control logic, including the
   behaviours that punish naive controllers (absorption lag, probe overshoot,
   drainage beyond field capacity).
3. Fault injection covering every device-originated failure in
   [failure-model.md](../architecture/failure-model.md).
4. Virtual time so a multi-hour cycle is a six-second test.
5. **Refusal behaviour identical to firmware**, via the shared validator.

## Non-goals

- Soil-physics accuracy. The model is an approximation for exercising control
  logic, not a claim about real soil.
- Irrigation intelligence. Like the firmware, the simulator obeys or refuses.
- Offline-policy evaluation and autonomous dose scheduling. M6-019 adds the
  single shared evaluator and its simulator integration.
- Being deleted when hardware arrives — it remains the CI path and the
  conformance reference.

## User/system flows

```text
operator/CI  → cargo run -p device-simulator -- --device-id … --time-scale 600
             → connects, publishes retained status, begins telemetry
             → edge sees a device
             → edge publishes a water command
             → simulator validates via the SHARED validator
                  accept → pump model runs → moisture rises with lag
                  reject → publishes reason, no state change
             → publishes command.result
```

## Functional requirements

### Protocol conformance

| ID | Requirement |
|---|---|
| F-020-01 | LWT configured before connect; `clean_session = true` |
| F-020-02 | Retained `status: online` on connect; `offline` LWT on unclean disconnect |
| F-020-03 | Subscribes to exactly the seven exact topics of protocol §3, built from `Topic::device_subscriptions`; no wildcard, and no filter matching a topic it publishes |
| F-020-04 | Publishes one `telemetry.batch` payload on `rhizo/v1/devices/{id}/telemetry` per sampling cycle, containing all `MeasurementSample` values taken in that cycle; publishes a separate `actuator.state` payload on `rhizo/v1/devices/{id}/actuator` when actuator state changes |
| F-020-05 | `boot_id` fresh each start; `sequence` monotonic within a boot |
| F-020-06 | `message_id` is UUIDv7 when the clock is synced |
| F-020-07 | Applies retained config; ignores `config_version` ≤ applied; echoes `applied_config_version` |
| F-020-08 | Publishes a `command.result` for **every** command, including rejections |
| F-020-09 | Retries result publication up to 60 s; persists unpublished results across restart |
| F-020-10 | Never publishes retained on telemetry or command topics |
| F-020-11 | Persists and atomically activates retained offline policies; reports applied versions |
| F-020-12 | Models isolation/reconnect while sampling and buffering continue |
| F-020-13 | Persists monotonic offline runtime state needed by the later shared evaluator |
| F-020-14 | Contains no simulator-specific offline evaluator and performs no autonomous dose in M2 |
| F-020-15 | Buffers bounded audit/telemetry history and replays stable event ids in order |
| F-020-16 | Applies `event.ack` (protocol §5.13) cumulatively: ignores another `boot_id`, ignores a sequence beyond any it issued without clamping, ignores one not newer, and never discards an unsent `history.gap` |

### Safety parity — the critical requirements

| ID | Requirement |
|---|---|
| F-020-20 | **The only actuation path calls `rhizo_mqtt_contract::validate_water_command`.** No second implementation, no bypass flag. |
| F-020-21 | Maintains a 16-entry `command_id` dedup ring with outcomes, persisted to disk |
| F-020-22 | Persists `(command_id, started_at, requested_ml)` **before** actuating |
| F-020-23 | On restart with an unfinished dose, publishes `status: "interrupted"`, `delivered_ml: null` |
| F-020-24 | Tracks `delivered_today_ml` and enforces `FIRMWARE_MAX_DAILY_ML` |
| F-020-25 | Reports compile-time `limits` in status |
| F-020-26 | Corrupt safety-critical persisted state is observable and disables actuation; corruption never restores budget, clears cooldown, or substitutes an enabled policy |

### Physical model

| ID | Requirement |
|---|---|
| F-020-30 | Exponential drying toward a floor, temperature-scaled |
| F-020-31 | Watering enters a pending-absorption pool with time constant `absorption_tau` |
| F-020-32 | Surface probe overshoots by up to 15 % of ΔVWC, decaying over ~2 min |
| F-020-33 | Volume beyond `field_capacity_vwc` drains and is not measured |
| F-020-34 | Pot weight rises **immediately** on delivery while VWC lags |
| F-020-35 | Tank depletes by delivered volume; EC rises as VWC falls |
| F-020-36 | Gaussian noise on all readings, on by default |

### Fault injection

| ID | Requirement |
|---|---|
| F-020-40 | All faults in [simulator-strategy.md](../testing/simulator-strategy.md) §6 available via CLI and a runtime control API |
| F-020-41 | Control API is simulator-only and clearly separated from protocol code |

### Time

| ID | Requirement |
|---|---|
| F-020-50 | `--time-scale` accelerates virtual time; reported at startup |
| F-020-51 | One clock per process; no mixing of accelerated and system time |

## Interfaces

**MQTT:** exactly [docs/protocol/mqtt-v1.md](../protocol/mqtt-v1.md). No
extensions.

**Control API** (simulator only, default `:9090`):

```text
POST /sim/fault        { "fault": "leak", "enabled": true }
POST /sim/state        { "moisture_vwc": 20.0 }
GET  /sim/state
POST /sim/restart
GET  /sim/scale
```

**CLI:** see [simulator-strategy.md](../testing/simulator-strategy.md) §7.

## Data model

A small JSON state file (`--state-file`, default alongside the binary) holding
what NVS holds on real hardware. The following is a conceptual inventory, not a
frozen serialization schema:

```json
{
  "boot_count": 12,
  "applied_config_version": 7,
  "delivered_today_ml": 130.0,
  "delivered_day_epoch": 20325,
  "command_ring": [ { "command_id": "018f…", "outcome": {} } ],
  "in_flight_dose": null,
  "pending_results": [],
  "policy_active": {
    "payload": {},
    "checksum": "sha256:…",
    "versions": { "monstera-01": 7 }
  },
  "policy_staging": null,
  "applied_policy_versions": { "monstera-01": 7 },
  "offline_runtime": {
    "budget_window": { "elapsed_ms": 1800000, "delivered_ml": 70.0 },
    "cooldown_remaining_ms": 14400000,
    "confirmation_elapsed_ms": 45000,
    "dose_count": 2
  },
  "offline_events": {
    "events": [],
    "pending_ack_through_seq": null,
    "gap": { "from_seq": 4100, "to_seq": 4380, "lost_count": 281, "lost_tier": "telemetry" }
  },
  "persistent_state_fault": null
}
```

The file carries a **checksum over its whole contents**, not only over the
policy blob. ADR-015 §7's CRC on the policy is necessary but not sufficient:
JSON with one flipped digit is still valid JSON, so `delivered_today_ml: 460.0`
silently becoming `60.0` would decode cleanly and hand the device four hundred
millilitres of budget it had already spent. A checksum mismatch is corruption
and fails closed exactly like an unparseable file. The checksum covers the
*decoded* state rather than the raw bytes, so a field a future build adds is
still ignored rather than treated as damage (§9's forward-compatibility rule,
applied to storage).

Mirroring NVS deliberately: it makes `--fault restart-mid-dose` reproduce
SAFETY-011 behaviour faithfully rather than approximately. The daily fields
remain because `validate_water_command` enforces the compile-time daily hard
limit; the rolling `offline_runtime.budget_window` is distinct policy state and
does not replace that hard-limit accounting.

## State model

```text
Disconnected ──► Connecting ──► Online ──► Publishing
     ▲                                        │
     └────────────── disconnect ──────────────┘

Pump:  Idle ──► Running(command_id, until) ──► Idle
                     │
                     └── restart ──► Interrupted ──► report ──► Idle
```

The pump model never re-enters `Running` for a `command_id` already in the ring.

## Failure modes

| Failure | Simulator behaviour |
|---|---|
| Broker unavailable | reconnect with full jitter, unlimited |
| Command with unknown fields | ignored, command still processed |
| Command failing validation | rejected with the exact reason; no state change |
| Safety-critical state file corrupt | Start in diagnostic/monitoring mode with an explicit persistent-state fault; disable pump/actuation, refuse commands that require persisted safety state, refuse/inactivate offline policy, and preserve the most restrictive budget/cooldown interpretation. Never replenish a budget, shorten a cooldown, clear dedup/in-flight uncertainty, or substitute defaults. |
| Non-safety physical-model state corrupt | May reset only the separable physical-model state; the persistent-state fault and actuation lockout remain until safety-critical state is explicitly recovered. |
| Result publish fails | retry 60 s, then persist and republish next boot |
| Control API request during a dose | accepted; faults may be injected mid-dose (that is the point) |

## Safety implications

The simulator does not *enforce* invariants for the real system — it is a test
device. But it is the vehicle by which three invariants are **tested** before
hardware exists:

- **SAFETY-002** — refuses expired and clock-unsynced commands.
- **SAFETY-007** — clamps or rejects oversized doses. SCEN-032 publishes
  `requested_ml: 10000` directly to the broker, bypassing the edge, and asserts
  the hard limit holds.
- **SAFETY-011** — restart mid-dose reports `interrupted`.
- **SAFETY-001, SAFETY-012, SAFETY-015** — corrupt persistent state cannot
  erase deduplication uncertainty, grant actuation permission, replenish a
  budget, or shorten a cooldown.

F-020-20 is the requirement everything else rests on. If the simulator ever
becomes more permissive than firmware, the M6 safety suite silently stops
meaning anything.

## Observability

Structured logging via `rhizo-telemetry` with `device_id` on every event. INFO
for connect/disconnect, command accept/reject, and dose start/end. DEBUG for
telemetry publication. The physical model's internal state is exposed through
`GET /sim/state` rather than logged.

No Prometheus metrics — the simulator is a test fixture, and its behaviour is
asserted directly by tests rather than sampled.

## Testing strategy

- Unit: drying curve monotonicity; absorption converges to the expected ΔVWC;
  drainage caps at field capacity; weight/VWC divergence; tank depletion
  arithmetic.
- Unit: dedup ring eviction; state-file round trip; interrupted-dose detection.
- Integration (real broker): connect/LWT/retained status; config apply and echo;
  command accept and reject paths; duplicate command produces one actuation;
  no retained messages on command topics; ACL isolation between two simulators.
- **`safety_007_simulator_refuses_like_hardware`** — publish an oversized
  command directly, assert clamping or rejection.
- Fixture drift: `--capture-fixtures` output diffed against
  `test/fixtures/protocol/valid/`.
- Structural: no `evaluate_offline` implementation or autonomous-dose scheduler
  exists in the simulator during M2.

## Acceptance criteria

- [x] `docker compose up mosquitto device-simulator` runs standalone; telemetry
      visible via `mosquitto_sub`.
- [x] A subscriber sees retained `status` and `config` and **nothing** on
      `commands/*`.
- [x] Killing the simulator produces the LWT within the keepalive window.
- [x] The same `command_id` published three times causes one actuation and three
      results.
- [x] `requested_ml: 10000` never delivers more than `FIRMWARE_MAX_ML_PER_RUN`.
- [x] `--fault restart-mid-dose` produces `status: "interrupted"` with
      `delivered_ml: null`.
- [x] At `--time-scale 600`, a dose-plus-absorption-plus-recheck sequence
      completes in under 10 seconds of wall time.
- [x] Two simulators with different credentials cannot publish into each other's
      topics.
- [x] Isolation keeps sampling and bounded buffering active without autonomous
      actuation.
- [x] Policy activation, version acknowledgement, and replay mechanics survive restart.
- [x] The simulator contains no offline-policy decision implementation.

## Dependencies

- M1 (contract crate, shared validator).
- M0 (Mosquitto with auth, telemetry crate).

The edge controller is **not** a dependency: the simulator must be runnable and
testable against a bare broker.

## Open questions

1. **Whether the control API should be feature-gated out of release builds.**
   Leaning yes for tidiness, but the simulator is never deployed, so it is
   cosmetic. Decided in M2-009.
2. **Default `absorption_tau` and `field_capacity_vwc`.** Chosen as plausible
   starting values (6 min, 45 %) and refined against real observations in M10.
   Nothing depends on their accuracy.

## Future work

- Multi-plant simulation within one process (M13 may prefer separate processes).
- Replay of captured real-device telemetry as a scenario source (M10+).
- LoRaWAN duty-cycle simulation (M14).
