# Safety Invariant Registry

These are the properties Rhizo Edge must never violate. They are numbered,
stable, and referenced from PRDs, issues, and test names. An invariant is not
satisfied by a code review; it is satisfied by an automated test that fails when
the invariant is broken.

**Naming convention.** Every test proving an invariant is named
`safety_NNN_<short_description>` so that `cargo test safety_` runs the whole
safety suite.

**Status legend.** `PLANNED` — specified, not yet enforced. `ENFORCED` — code
and test exist and are green.

---

## Summary table

| ID | Invariant | Primary enforcer | Enforced at | Status |
|---|---|---|---|---|
| SAFETY-001 | A duplicate watering command never causes duplicate physical watering | Device + Edge | M6 (edge), M9 (device) | PLANNED |
| SAFETY-002 | An expired watering command never executes | Device | M6 (sim), M9 (fw) | PLANNED |
| SAFETY-003 | Leak detected disables all watering | Edge + Device | M6 | PLANNED |
| SAFETY-004 | Tank below minimum disables watering | Edge + Device | M6 | PLANNED |
| SAFETY-005 | Stale or invalid moisture disables automatic watering | Edge | M6 | PLANNED |
| SAFETY-006 | Daily automatic water never exceeds the configured maximum | Edge | M6 | PLANNED |
| SAFETY-007 | The device hard maximum cannot be bypassed by edge or cloud | Device firmware | M6 (sim), M11 (hw) | PLANNED |
| SAFETY-008 | Cloud unavailability never disables local monitoring | Edge | M7 | PLANNED |
| SAFETY-009 | Cloud unavailability never bypasses local watering safety | Edge | M7 | PLANNED |
| SAFETY-010 | Edge restart never re-executes a completed command | Edge | M6 | PLANNED |
| SAFETY-011 | Device restart during watering converges to pump-off | Device | M9 (fw), M11 (hw) | PLANNED |
| SAFETY-012 | Uncertainty defaults to not watering | Edge domain | M6 | PLANNED |

Milestone M8 re-verifies SAFETY-001 … SAFETY-010 and SAFETY-012 end-to-end in
the full Docker environment.

---

## SAFETY-001 — Duplicate command, single watering

**Statement.** If the same watering command (`command_id`) is delivered to a
device more than once — by MQTT QoS 1 redelivery, by an edge retry, or by a
broker restart — the pump runs at most once.

**Rationale.** QoS 1 is at-least-once by definition. Without command-level
idempotency, every network hiccup is an over-watering event.

**Enforcing components.**
- *Device*: maintains a bounded set of recently executed `command_id`s. Before
  actuating, the id is written to NVS; on repeat, the stored result is
  re-published rather than the pump re-run.
- *Edge*: `commands.command_id` is the primary key; a result for an already
  terminal command updates nothing and creates no second `watering_event`.

**Persisted state required.**
- Device NVS: ring of last 16 `command_id`s + their outcomes.
- Edge SQLite: `commands` (PK `command_id`), `processed_messages`.

**Failure scenarios covered.** Broker redelivery after reconnect; edge crash
between publish and result; device reboot between receipt and result publish.

**Planned tests.**
- `safety_001_duplicate_command_single_actuation` (integration, M6): publish the
  same command twice, assert exactly one `watering_event` and one pump run.
- `safety_001_repeat_result_is_idempotent` (unit, M6).
- Property test: N random duplications of a command sequence produce actuation
  count == distinct `command_id` count.

**Becomes enforced.** M6 for edge and simulator; M9 for firmware; re-verified in
M8 scenario `duplicate-command`.

---

## SAFETY-002 — Expired command never executes

**Statement.** A water command whose `expires_at` has passed is refused by the
device and never actuates the pump.

**Rationale.** A device that was offline for six hours must not wake up and
execute the queue of commands the broker held for it. Yesterday's decision is
not evidence about today's soil.

**Enforcing component.** Device (authoritative). The edge also refuses to count
an expired command as delivered.

**Persisted state required.** None beyond the command itself — but the device
needs a trustworthy wall clock, which is why:

> **A device whose clock is not SNTP-synced must refuse every water command**
> with `reason = clock_unsynced`. It cannot evaluate TTL, so under SAFETY-012 it
> must decline. See [time-model.md](time-model.md).

**Failure scenarios covered.** Long device offline period; broker holding QoS 1
messages; clock skew; device booting with an unset RTC.

**Planned tests.**
- `safety_002_expired_command_rejected` (unit, contract crate).
- `safety_002_offline_device_rejects_queued_commands` (integration, M6).
- `safety_002_unsynced_clock_rejects_command` (unit, M9 shared logic).

**Becomes enforced.** M6 (simulator, via the shared `validate_water_command`);
M9 (firmware).

---

## SAFETY-003 — Leak lockout

**Statement.** While the leak sensor reports water present, no watering occurs —
automatic, recommended, **or manual**. Clearing requires the leak signal to be
absent *and* an explicit operator reset.

**Rationale.** A leak means water is already going where it should not. The one
thing that must not happen next is more water. Manual override is excluded
deliberately: the operator who would click "water anyway" is exactly the person
who has not yet looked at the floor.

**Enforcing components.** Edge (gate, first check in `evaluate`) and device
(local refusal, in case the edge is stale).

**Persisted state required.** `plants.lockout_reason`, `plants.lockout_since`,
`plants.lockout_cleared_by` in SQLite; leak state on device in RAM.

**Failure scenarios covered.** Leak during a dose; leak while the edge is
restarting; leak sensor asserting between the safety check and actuation
(caught by the device's own check).

**Planned tests.**
- `safety_003_leak_blocks_automatic` (property, domain).
- `safety_003_leak_blocks_manual_api` (integration, M6) — asserts the REST
  endpoint returns 409 rather than issuing.
- `safety_003_leak_requires_explicit_reset` (unit).

**Becomes enforced.** M6. Real sensor in M11.

---

## SAFETY-004 — Empty reservoir lockout

**Statement.** When the tank level is at or below `tank_min_percent`, no command
is issued and the device refuses to run the pump.

**Rationale.** Peristaltic pumps are damaged by dry running, and a dose that
delivers nothing while the system believes it delivered 50 ml corrupts the daily
budget and the moisture model.

**Enforcing components.** Edge gate; device local check.

**Persisted state required.** Latest tank telemetry with its age; the same
staleness rule as SAFETY-005 applies — an unknown tank level is not a permissive
one.

**Failure scenarios covered.** Tank drains mid-cycle; tank sensor missing; tank
telemetry stale.

**Planned tests.**
- `safety_004_low_tank_blocks_dose` (property, domain).
- `safety_004_unknown_tank_blocks_dose` (unit) — absence is not permission.
- M8 scenario `tank-empty`.

**Becomes enforced.** M6. Real sensor in M11.

---

## SAFETY-005 — Stale or invalid moisture disables automatic watering

**Statement.** Automatic watering requires a moisture sample that is both
range-valid and no older than `max_sample_age` (default 3× the telemetry
interval, minimum 15 minutes). Otherwise the plant enters `SensorFault` or
`StaleData` lockout.

**Rationale.** Automatic irrigation is a control loop. A control loop without
fresh feedback is an open-loop timer pointed at a plant.

**Enforcing component.** Edge domain gate.

**Persisted state required.** `measurements.received_at` (edge clock — see
[time-model.md](time-model.md)); `devices.last_seen_at`.

**Note.** This invariant constrains *automatic* watering. Explicit operator
manual watering is permitted under sensor fault, because a human has taken
responsibility — but it is still subject to SAFETY-003, -004, -006, and -007.
This asymmetry is intentional and must be visible in the UI.

**Failure scenarios covered.** Sensor unplugged; device offline; probe returning
a constant; NaN readings.

**Planned tests.**
- `safety_005_stale_sample_blocks_auto` (property over random sample ages).
- `safety_005_invalid_sample_blocks_auto` (unit: NaN, out-of-range, absent).
- M8 scenarios `stale-sensor`, `invalid-sensor-value`.

**Becomes enforced.** M6.

---

## SAFETY-006 — Daily water cap

**Statement.** The total automatically delivered volume for a plant within any
rolling 24-hour window never exceeds `profile.max_daily_ml`.

**Rationale.** This is the last line of defence against a logic bug in the state
machine. Every other rule can fail and this one still bounds the damage.

**Design decision — rolling, not calendar.** A calendar-day cap allows two full
daily allowances within a few hours around midnight. The cap is computed over
`now - 24h`, summing `delivered_ml` from `watering_events`.

**Enforcing component.** Edge, checked immediately before issuing each dose and
again with the dose included (`delivered_today + dose <= max`).

**Persisted state required.** `watering_events(plant_id, completed_at,
delivered_ml)`, indexed on `(plant_id, completed_at)`. Survives restart by
construction.

**Failure scenarios covered.** State machine looping; repeated restarts each
issuing a dose; clock adjustment; partial delivery accounting.

**Planned tests.**
- `safety_006_rolling_24h_cap_never_exceeded` — **property test**, the flagship
  one: generate random command/restart/clock-jump sequences, assert the rolling
  sum never exceeds the cap.
- `safety_006_cap_survives_restart` (integration, M6).

**Becomes enforced.** M6.

---

## SAFETY-007 — Device hard maximum cannot be bypassed

**Statement.** No command from the edge or cloud, however well-formed, causes
the device to run the pump longer than `FIRMWARE_MAX_RUN_SECONDS` or deliver
more than `FIRMWARE_MAX_ML_PER_RUN` in one command, or exceed
`FIRMWARE_MAX_DAILY_ML` per device per day.

**Rationale.** This is the invariant that makes the system trustworthy with a
real plant and a real floor. Every other component may be compromised, buggy, or
misconfigured; the device still cannot flood the room.

**Enforcing component.** Device firmware **only**. These constants are compile-
time, are not present in the retained config topic, and there is no message that
can change them.

**Additional hardware backstop (M11).** A hardware watchdog and an unconditional
`pump_off()` timer independent of the MQTT task, so that a hung task cannot
leave the pump energised.

**Persisted state required.** Per-day delivered total in NVS, so the daily
device cap survives reboot.

**Failure scenarios covered.** Malicious or buggy edge sending `10000 ml`; a
compromised cloud; an operator typo; edge software regression.

**Planned tests.**
- `safety_007_oversized_command_rejected` (unit, contract crate — the shared
  `validate_water_command` both simulator and firmware call).
- `safety_007_simulator_refuses_like_hardware` (integration, M6) — the simulator
  must be no more permissive than firmware.
- M11: hardware-in-the-loop test with a measuring cup.

**Becomes enforced.** M6 for the shared validator and simulator; M9 for
firmware; M11 for real hardware including the watchdog.

---

## SAFETY-008 — Cloud outage does not disable monitoring

**Statement.** With the cloud unreachable, telemetry ingestion, persistence,
plant state, recommendations, the REST API, and metrics all continue to work.

**Rationale.** The whole thesis of the project.

**Enforcing component.** Edge architecture — the outbox pattern. The control
loop has no code path that awaits a cloud response.

**Persisted state required.** `pending_cloud_events` grows; a configured cap
(`outbox_max_rows`, default 500 000) triggers oldest-first pruning of
low-value events (measurements) while preserving high-value ones (watering
events, lockouts) and raising an alert.

**Planned tests.**
- `safety_008_local_operation_without_cloud` (integration, M7): cloud down for
  the whole test, assert full local function.
- M8 scenario `cloud-outage-recovery`.

**Becomes enforced.** M7.

---

## SAFETY-009 — Cloud outage does not bypass safety

**Statement.** A cloud outage never relaxes a lockout, never grants extra daily
volume, and never causes a dose to be issued that would not have been issued
with the cloud up.

**Rationale.** The inverse of SAFETY-008: degradation must be in features, never
in safety. A "we cannot reach the cloud so we will assume it is fine" path is
exactly the class of bug this forbids.

**Enforcing component.** Edge — cloud state is not an input to
`rhizo_domain::evaluate`. This is enforced structurally: `IrrigationInputs` has
no cloud-derived field, and the `domain` crate cannot depend on `cloud-client`.

**Planned tests.**
- `safety_009_decisions_identical_with_cloud_down` (integration, M7): run the
  same scenario with cloud up and down, assert the issued command sequence is
  identical.

**Becomes enforced.** M7.

---

## SAFETY-010 — Edge restart does not replay completed commands

**Statement.** Restarting the Edge Controller — at any point, including mid-dose
— never causes a previously completed command to be re-issued or a completed
watering event to be double-counted.

**Rationale.** Crash-restart loops are common in practice. A restart that
re-waters is a restart that floods.

**Enforcing component.** Edge, via the persist-before-publish ordering and
terminal command states.

**Recovery procedure on boot:**

```text
1. load all commands with status IN ('issued','in_flight')
2. for each:
     expires_at < now  → mark 'expired', irrigation_state → Recheck
     otherwise         → mark 'in_flight', await result until expires_at
3. never re-publish a command that already has a command_id on the wire
4. rebuild irrigation state from SQLite, never from defaults
```

**Persisted state required.** `commands.status` with terminal states
(`completed`, `rejected`, `expired`, `failed`); `irrigation_state` table.

**Planned tests.**
- `safety_010_restart_mid_command_no_replay` (integration, M6): kill the process
  after publish, restart, assert no second command and no second event.
- `safety_010_terminal_commands_never_reissued` (property).

**Becomes enforced.** M6.

---

## SAFETY-011 — Device restart converges to pump off

**Statement.** However a device restarts — power loss, watchdog, panic, OTA — it
comes up with the pump de-energised and does not resume an interrupted dose.

**Rationale.** An interrupted dose of unknown delivered volume must not be
completed blindly; the correct response is to report what is known and let the
edge re-evaluate with fresh soil data.

**Enforcing component.** Device firmware.

**Requirements this places on the design:**
- The pump GPIO must be driven to the inactive level as the first action in
  `main`, before Wi-Fi, before MQTT. The driver must be chosen so the *default*
  electrical state of an un-driven pin is pump-off (pull-down on the MOSFET
  gate), so the pump is also off during the bootloader window.
- Before actuating, the device records `command_id` + `started_at` in NVS.
- On boot, an unfinished record produces a `command_result` with
  `status = interrupted` and `delivered_ml = null`.
- The edge treats `interrupted` as a terminal, non-crediting outcome and moves
  to `Recheck` rather than assuming success or failure.

**Planned tests.**
- `safety_011_boot_state_pump_off` (unit, firmware host tests).
- `safety_011_interrupted_dose_reported` (integration, M9 with simulator
  restart; M11 with real hardware).

**Becomes enforced.** M9 (firmware logic, host-testable); M11 (hardware).

---

## SAFETY-012 — Uncertainty defaults to no watering

**Statement.** Whenever a required input to the watering decision is missing,
unparseable, contradictory, or of unknown age, the decision is *do not water*
plus a visible lockout — never *water anyway*.

**Rationale.** This is the meta-invariant. It converts every unforeseen gap in
the other eleven into a safe outcome rather than an undefined one.

**Enforcing component.** `rhizo_domain::evaluate`, structurally.

**Design requirement.** `IrrigationInputs` uses `Option<T>` for every input that
can be absent, and the safety gate matches exhaustively. There is no
`unwrap_or_default()` on a safety input, and no `_ =>` arm that falls through to
a permissive branch. A new input added without a gate arm must fail to compile.

**Failure scenarios covered.** Everything not enumerated elsewhere: new sensor
types, partially migrated databases, profile fields added later, corrupt rows.

**Planned tests.**
- `safety_012_missing_input_never_waters` — property test that generates
  `IrrigationInputs` with random fields set to `None` and asserts the decision is
  never `IssueDose` when any safety-relevant input is `None`.
- A compile-time guard: the gate `match` is exhaustive with no catch-all arm.

**Becomes enforced.** M6.

---

## How invariants are kept honest

1. Every invariant above names at least one test. M6 and M7 are not complete
   until those tests exist and pass.
2. `rhizo-docscheck` verifies that every `SAFETY-NNN` referenced anywhere in
   `docs/` exists in this file, and that this file's summary table has no
   invariant lacking a "Planned tests" section.
3. When an invariant moves to `ENFORCED`, the status column here and the
   milestone exit criteria in [ROADMAP.md](../../ROADMAP.md) are updated in the
   same change as the test.
