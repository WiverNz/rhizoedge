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

M2/M3 already provide tested enforcement points without completing the full
end-to-end invariants: the simulator's single actuation path and shared command
validator support SAFETY-001/-002/-007/-012; fail-closed persistent-state and
policy mechanics support SAFETY-013/-015/-019/-020; and M3 supplies durable
effect identity, status ordering, event replay, and history-gap persistence for
SAFETY-001/-011/-016/-020. These rows remain `PLANNED` until every enforcement
point named by the invariant exists; no M6/M9 work is claimed complete here.

---

## Summary table

| ID | Invariant | Primary enforcer | Enforced at | Status |
|---|---|---|---|---|
| SAFETY-001 | A duplicate watering command never causes duplicate physical watering | Device + Edge | M6 (edge), M9 (device) | ENFORCED (edge, simulator) |
| SAFETY-002 | An expired watering command never executes | Device | M6 (sim), M9 (fw) | ENFORCED (simulator) |
| SAFETY-003 | Leak detected disables all watering | Edge + Device | M6 | ENFORCED |
| SAFETY-004 | Tank below minimum disables watering | Edge + Device | M6 | ENFORCED |
| SAFETY-005 | Stale or invalid moisture disables automatic watering | Edge | M6 | ENFORCED |
| SAFETY-006 | Daily automatic water never exceeds the configured maximum | Edge | M6 | ENFORCED |
| SAFETY-007 | The device hard maximum cannot be bypassed by edge or cloud | Device firmware | M6 (sim), M11 (hw) | ENFORCED (validator, simulator) |
| SAFETY-008 | Cloud unavailability never disables local monitoring | Edge | M7 | ENFORCED |
| SAFETY-009 | Cloud unavailability never bypasses local watering safety | Edge | M7 | ENFORCED |
| SAFETY-010 | Edge restart never re-executes a completed command | Edge | M6 | ENFORCED |
| SAFETY-011 | Device restart during watering converges to pump-off | Device | M9 (fw), M11 (hw) | PLANNED |
| SAFETY-012 | Uncertainty defaults to not watering | Edge domain + device | M6 | ENFORCED |
| SAFETY-013 | Autonomous action requires a validated persisted policy | Device | M6 (sim), M9 (fw) | ENFORCED (evaluator, simulator) |
| SAFETY-014 | Offline doses obey the same caps and hard limits as commanded doses | Device + Edge | M6 | ENFORCED |
| SAFETY-015 | Clock uncertainty never grants budget or shortens a cooldown | Device | M6 (sim), M9 (fw) | ENFORCED (evaluator, simulator) |
| SAFETY-016 | Offline actions reconcile exactly once; no dose spans the seam twice | Edge + Device | M6 (edge), M9 (device) | ENFORCED (edge) |
| SAFETY-017 | A required measurement that is missing or stale blocks autonomous action | Device | M6 | ENFORCED |
| SAFETY-018 | A plant with no actuator binding has no actuation path at all | Edge | M5 | ENFORCED |
| SAFETY-019 | Policy activation is atomic; a bad update never replaces a good policy | Device | M9 | PLANNED |
| SAFETY-020 | Lost buffered history is reported as an explicit gap, never silently dropped | Device + Edge | M9 | PLANNED |
| SAFETY-021 | Expected sleep is bounded and never masks an unexpected absence | Edge | M4 | ENFORCED |
| SAFETY-022 | A learned estimate may narrow a watering decision, never widen one | Edge domain | M15 | PLANNED |

**Fourteen invariants moved to `ENFORCED` on 2026-08-31**, when M6 landed the
safety gate, the irrigation machine, the command lifecycle, the shared offline
evaluator, and reconciliation. Four of them are enforced for the halves M6 owns
and name the milestone that completes them:

- **SAFETY-001** holds on the edge (`commands.command_id` as the primary key, a
  terminal-status check on every result) and in the simulator's dedup ring. The
  firmware half is M9.
- **SAFETY-002** and **SAFETY-007** are enforced by the shared validator and by
  the simulator that calls it. M9 puts the same function in firmware; M11 adds
  the hardware backstop.
- **SAFETY-013** and **SAFETY-015** are enforced by `rhizo_policy` and by the
  simulator's single call site. M9 adds the firmware NVS path.
- **SAFETY-016** is enforced on the edge — replay is idempotent on `event_id`
  and no dose is issued to a reconciling plant. The device half shipped in M2.


Milestone M8 re-verifies SAFETY-001 … SAFETY-010 and SAFETY-012 end-to-end in
the full Docker environment, and SAFETY-013 … SAFETY-020 in its network-isolation
scenarios (SCEN-090…SCEN-107). Time synchronisation is covered by
SCEN-073…SCEN-078.

**SAFETY-001 … SAFETY-012 are unchanged and are never renumbered.** SAFETY-013
onward were added on 2026-08-26 when device offline autonomy became a
requirement ([ADR-015](../adr/015-device-offline-autonomy.md)). Two existing
invariants gained scope rather than changing meaning:

- **SAFETY-006** (rolling cap) now counts autonomous doses in the same window —
  one budget per plant, not one per control path. See SAFETY-014.
- **SAFETY-012** (uncertainty ⇒ no watering) now also governs the device's
  offline gate, which is why it lists the device as an enforcer.

**Edge time synchronisation added no invariant.** When the device clock source
changed from NTP to `edge.time` over MQTT on 2026-08-26
([ADR-013](../adr/013-clock-and-time-semantics.md)), the count stayed at twenty
deliberately. The mechanism is a *means* of satisfying SAFETY-002 — "an expired
command never executes" — not a new safety property, so SAFETY-002 gained the
enforcing rule, the failure scenarios, and three tests instead. The isolated-device
case, where no synchronisation is possible at all, is already SAFETY-015. A
twenty-first invariant would have restated one of the two without constraining
anything new, and an invariant catalogue is only useful while every entry earns
its number.

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
- `safety_001_duplicate_command_single_actuation` — **green since M2**
  (`device-simulator`, unit) plus
  `safety_001_the_same_command_published_three_times_actuates_once`
  (`tests/safety_007.rs`, real broker): three publications, three results, one
  actuation, confirmed against the reservoir. The edge half remains M6.
- `safety_001_duplicate_command_single_actuation` (integration, M6): publish the
  same command twice, assert exactly one `watering_event` and one pump run.
- `safety_001_repeat_result_is_idempotent` (unit, M6).
- Property test: N random duplications of a command sequence produce actuation
  count == distinct `command_id` count.

**Enforced since M6 (2026-08-31)** for the edge and the simulator; M9 for
firmware; re-verified in M8 scenario `duplicate-command`. The edge half is
`safety_001_duplicate_command_single_actuation` (`edge-controller`, real
broker): one command, three identical results with distinct `message_id`s, one
`watering_event`.

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

> **A device that is not synchronised to the Edge clock must refuse every water
> command** with `reason = clock_unsynced`. It cannot evaluate TTL, so under
> SAFETY-012 it must decline. Synchronisation arrives over MQTT
> ([mqtt-v1.md](../protocol/mqtt-v1.md) §5.12) and expires by age, so this covers
> both "never synchronised" and "synchronisation too old". See
> [time-model.md](time-model.md).

**Failure scenarios covered.** Long device offline period; broker holding QoS 1
messages; clock skew; device booting with an unset RTC; **a stale `edge.time`
attempting to move the device clock backwards**; **a duplicated `edge.time`
replayed to keep the synchronisation window alive without a newer Edge
timestamp**; synchronisation ageing out while connected; commands arriving in the
window after reconnect but before re-synchronisation.

**Planned tests.**
- `safety_002_expired_command_rejected` (unit, contract crate; and **green
  since M2** at device level in `device-simulator`).
- `safety_002_offline_device_rejects_queued_commands` (integration, M6).
- `safety_002_unsynced_clock_rejects_command` (unit, M9 shared logic).
- `safety_002_stale_time_sync_never_applied` (unit): an `edge.time` older than
  the last applied one is ignored, so the clock never moves backwards.
- `safety_002_duplicate_time_sync_does_not_extend_validity` (unit): the same
  `edge_time_ms` replayed many times never refreshes `synced_at_monotonic`, so
  `clock_synced` still becomes false once `TIME_SYNC_MAX_AGE_SECONDS` elapses.
  Acceptance is **strictly** increasing, not non-decreasing.
- `safety_002_sync_age_expiry_rejects_command` (unit): synchronisation older than
  `TIME_SYNC_MAX_AGE_SECONDS` makes `clock_synced` false and commands refused.
- `safety_002_command_refused_until_resync_after_reconnect` (integration, M8).

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

**Enforced since M6 (2026-08-31).** Real sensor in M11.
`safety_003_leak_blocks_all_modes` is the property over random states and modes;
`safety_003_leak_blocks_manual_api` is the API half, and the broker-backed
`safety_003_leak_blocks_manual_api_with_nothing_published` adds the assertion a
status code cannot make — that no message appears on any command topic.

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

**Enforced since M6 (2026-08-31).** Real sensor in M11.
`safety_004_tank_unknown_or_low_blocks` is the property; the gate answers
`Uncertain` for a level it cannot read and `TankLow` for one it measured, which
is the same distinction protocol §5.8 step 7 draws.

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

**No power field may widen this window.** `max_sample_age` derives from the
configured telemetry interval and from per-plant `MeasurementPolicy.stale_after`,
and from nothing else. A battery device's `wake_interval_seconds` is a
*device-declared* field admitting values up to 86 400 seconds; it feeds the
registry's connectivity/liveness indication and is bounded there
([PRD 040](../prd/040-device-registry-and-health.md) F-040-26), but it must never
reach this threshold. Otherwise a device could advertise itself a three-day
freshness window and make an old moisture reading look like grounds for watering
— SAFETY-021's failure arriving through the staleness door. The two calculations
are deliberately separate functions, `max_sample_age_seconds` and
`liveness_interval_seconds`, so that using the wrong one is a visible choice.

**Failure scenarios covered.** Sensor unplugged; device offline; probe returning
a constant; NaN readings; a battery device declaring a very long wake interval.

**Planned tests.**
- `safety_005_stale_sample_blocks_auto` (property over random sample ages).
- `safety_005_control_freshness_cannot_be_widened_by_a_declared_wake_interval`
  (unit, already green in M4's registry).
- `safety_005_invalid_sample_blocks_auto` (unit: NaN, out-of-range, absent).
- M8 scenarios `stale-sensor`, `invalid-sensor-value`.

**Enforced since M6 (2026-08-31).** `safety_005_stale_or_invalid_blocks_auto`
is the property, and it asserts both halves: automatic watering is blocked, and
manual watering is not.

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

**Enforced since M6 (2026-08-31).** `safety_006_rolling_24h_cap_never_exceeded`
passes at `PROPTEST_CASES=10000` against histories containing restarts, forward
and backward clock steps, interrupted doses, and duplicate results.

**The cap is only as good as the rows it sums** (post-M6 correction,
2026-08-31). The invariant is derived from `watering_events`, so anything that
loses or misfiles a row weakens it silently, in the over-watering direction. Two
such paths were found and closed:

- **A lost `command.result`.** Protocol §5.10 previously stopped a device's
  retry on the broker's QoS 1 PUBACK, which is written on receipt and says
  nothing about whether the edge committed. An edge that crashed in between lost
  the row and the device never sent it again, so a delivered dose was never
  counted. Closed by `command.result.ack` (§5.14): a result is retained on the
  device until the edge acknowledges its own commit.
- **A misattributed offline dose.** A replayed `watering.offline_autonomous`
  event was charged to whichever plant held the actuator binding *at replay
  time*. Rebinding the pump while a device was isolated therefore charged the
  wrong plant — leaving the plant that really was watered with a clean budget
  and free to be watered again. Closed by `detail.plant_id` (§5.4): the dose
  names its own subject, and that name is written in the same transaction as the
  event.

Both are covered by named tests; neither changed the invariant's statement, its
enforcing component, or its rolling-window derivation.

**The same argument has a device-side half, and it is M9 work.** Retaining a
result until the edge acknowledges it turns the device's single `pending_result`
slot into a **bounded durable ledger**, and a bounded ledger saturates. Firmware
must therefore fail closed when it is full and must not silently discard an
unacknowledged watering result in a way that can under-count delivered water —
which would starve this cap from the far end, invisibly. The requirement is
[ADR-014](../adr/014-failure-and-retry-policy.md) §Device-side pending-result
ledger and PRD 090 F-090-17…19; M9-011 decides the mechanism and M9-022 verifies
it. **Not enforced today**, because no firmware exists: the device simulator
bounds its ledger at 32 entries and evicts the oldest, which is acceptable for a
host that is exercising the protocol and is explicitly *not* a specification for
firmware.

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
- `safety_007_simulator_refuses_like_hardware` — **green since M2**
  (`crates/device-simulator/tests/safety_007.rs`), earlier than planned because
  the simulator exists: it publishes `requested_ml: 10000` straight to the
  broker and asserts on the reservoir as well as the reported number.
- `safety_007_oversized_command_clamped_not_delivered` (unit, M2).
- `safety_007_simulator_refuses_like_hardware` (integration, M6) — the simulator
  must be no more permissive than firmware.
- M11: hardware-in-the-loop test with a measuring cup.

**Enforced since M6 (2026-08-31)** for the shared validator and the simulator;
M9 for firmware; M11 for real hardware including the watchdog. M6 extracted
steps 10-12 into `rhizo_mqtt_contract::safety::bound_dose` so the *autonomous*
actuation path applies the same ceilings from the same function.

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

**Planned tests.** Demonstrated in M7:
- `safety_008_edge_readiness_stays_200_without_cloud` and the real-Mosquitto
  Edge integration/irrigation suites with `cloud-api` stopped (M7).
- M8 scenario `cloud-outage-recovery`.

**Enforced since M7 (2026-08-31).**

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

**Planned tests.** Demonstrated in M7:
- `safety_009_decisions_identical_with_cloud_down` (integration, M7): run the
  same scenario with cloud up and down, assert the issued command sequence is
  identical.

**Enforced since M7 (2026-08-31).**

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

**Enforced since M6 (2026-08-31).** `safety_010_restart_mid_command_no_replay`
and the broker-backed `restart_mid_command` both assert the two halves: no
second command, and exactly one `watering_event`.

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
- `safety_011_interrupted_dose_reported` — **green since M2**
  (`device-simulator`, unit): a restart mid-dose reports `interrupted` with
  `delivered_ml: null`, credits the full requested volume, and deduplicates the
  command afterwards. `restart_mid_dose_kills_the_device_after_the_state_write_and_reports_interrupted`
  covers the fault path.
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
- `safety_012_corruption_never_makes_the_device_more_permissive` — **green since
  M2** (`device-simulator`, `tests/state_fails_closed.rs`): arbitrary corruption
  of persisted state can never restore actuation permission, replenish a budget,
  shorten a cooldown, or substitute a policy.
- `safety_012_missing_input_never_waters` — property test that generates
  `IrrigationInputs` with random fields set to `None` and asserts the decision is
  never `IssueDose` when any safety-relevant input is `None`.
- A compile-time guard: the gate `match` is exhaustive with no catch-all arm.

**Enforced since M6 (2026-08-31).** `safety_012_missing_input_never_waters` is
the property, and `safety_012_no_catch_all_arm_on_a_safety_match` is its
compile-time half, asserted against the gate's own source.

---

## SAFETY-013 — Autonomous action requires a validated, persisted policy

**Statement.** An isolated device actuates only from an offline policy that the
Edge validated, that the device re-validated and persisted, and that is currently
active. No policy, an unparseable policy, a failed CRC, an unknown version, or
`enabled = false` all mean **no actuation**.

**Rationale.** This is the invariant that separates "provisioned autonomy" from
"a device improvising". A device that invents a threshold because it has none is
more dangerous than a device that does nothing, because nobody authorised what it
does and nobody can predict it. Absence of configuration is not permission —
SAFETY-012 applied to the offline path.

**Enforcing component.** Device, in `rhizo_policy::evaluate_offline`, gate steps
1–2 ([offline-autonomy.md](offline-autonomy.md) §4).

**Persisted state required.** NVS: `policy_blob` + `policy_version` + CRC, and the
active/staging pointer. The simulator mirrors this in its state file.

**Failure scenarios covered.** Never-provisioned device; NVS erased by a
reflash; corrupted flash sector; policy for a plant whose actuator binding was
removed; operator disabled autonomy but the device has not yet been told.

**Planned tests.**
- `safety_013_no_policy_never_actuates` (property, `rhizo-policy`): generate
  arbitrary inputs with `policy = None`; assert the decision is never `Dose`.
- `safety_013_corrupt_policy_never_actuates` (unit): bit-flipped blob, bad CRC,
  truncated blob, unknown version — each refuses and keeps the previous policy.
- `safety_013_disabled_policy_never_actuates` (unit).
- SCEN-093, SCEN-094.

**Enforced since M6 (2026-08-31)** by the evaluator and the simulator; M9 adds
the firmware NVS path.

---

## SAFETY-014 — Offline doses obey the same caps and hard limits

**Statement.** Water delivered autonomously counts toward the **same** rolling
volume window as commanded water, and is bounded by the same firmware hard
limits. There is one budget per plant, not one per control path.

**Rationale.** Two independent budgets is the obvious way to build this and the
obvious way to double-water a plant: the device spends its allowance while
isolated, reconnects, and the Edge — whose own counter never moved — spends the
allowance again.

**Enforcing components.**
- *Device*: `budget_used_ml` accumulator checked before every autonomous dose;
  `validate_water_command`'s hard limits apply unchanged to the actuation path.
- *Edge*: after reconciliation the rolling window is recomputed from
  `watering_events` rows, which now include `origin = 'offline_autonomous'`
  entries — so the Edge's budget absorbs the device's spending automatically.

**Persisted state required.** Device NVS accumulator + window start; edge
`watering_events` with `origin`.

**Failure scenarios covered.** Long isolation with several autonomous cycles;
reconnect immediately after an autonomous dose; device and edge disagreeing about
elapsed time; repeated reboots during isolation.

**Planned tests.**
- `safety_014_offline_dose_counts_in_rolling_window` (integration, M6).
- `safety_014_combined_budget_never_exceeded` (**property**): interleave random
  commanded and autonomous doses across a simulated 72 h with reconnections;
  assert the rolling 24 h sum never exceeds `max_volume_per_window`.
- `safety_014_hard_limit_applies_offline` (unit): a policy dose is still clamped
  by `FIRMWARE_MAX_ML_PER_RUN`.
- SCEN-096, SCEN-101.

**Enforced since M6 (2026-08-31)**, re-verified M8, hardware M11.
`safety_014_combined_budget_never_exceeded` passes at `PROPTEST_CASES=10000`.

---

## SAFETY-015 — Clock uncertainty never grants budget or shortens a cooldown

**Statement.** An isolated device evaluates every offline rule from **monotonic
elapsed time**. Loss of wall-clock sync, clock drift, or a reboot never
replenishes the volume budget, never shortens a cooldown, and never shortens a
confirmation or absorption interval.

**Rationale.** Offline rules are durations, so they do not need a calendar
([ADR-013](../adr/013-clock-and-time-semantics.md)). But a reboot resets the
monotonic clock, and the naive recovery — "start the day fresh" — would let a
device in a reboot loop water without limit. The safe direction is to assume no
time has passed.

**Enforcing component.** Device. `evaluate_offline` takes elapsed time as a
parameter and has no clock access at all, so this is structural rather than
disciplined.

**Persisted state required.** `budget_used_ml`, `window_started_monotonic`, and
`cooldown_remaining_ms` — the cooldown stored as a **remaining duration**, never
as an absolute deadline the device might not be able to interpret after boot.

**Note.** Edge commands are unaffected: they still require a synced wall clock and
are still refused otherwise (SAFETY-002). This invariant governs only the
autonomous path.

**Failure scenarios covered.** Boot with no Edge synchronisation ever;
synchronisation lost mid-isolation; wall clock jumping backward or forward when a
late `edge.time` is applied on reconnect; repeated power cycling; overflow of the
monotonic counter.

**Planned tests.**
- `safety_015_reboot_does_not_replenish_budget_or_shorten_cooldown` — **green
  since M2** (`device-simulator`, `tests/isolation.rs`): ten consecutive reboots
  leave the stored cooldown and budget untouched, and observed time is the only
  thing that moves them.
- `safety_015_reboot_does_not_replenish_budget` (property): random reboot points
  in a dosing sequence; assert total delivered never exceeds the window cap.
- `safety_015_reboot_does_not_shorten_cooldown` (unit).
- `safety_015_unsynced_clock_still_refuses_commands` (unit) — SAFETY-002 holds.
- `safety_015_monotonic_overflow_is_safe` (unit).
- SCEN-097, SCEN-098.

**Enforced since M6 (2026-08-31)** by the evaluator; M9 adds firmware
persistence. `evaluate_offline` takes credited elapsed time as a parameter and
the crate cannot read a clock, so this is structural rather than disciplined.

---

## SAFETY-016 — Offline actions reconcile exactly once

**Statement.** Events buffered during isolation are applied to Edge history
exactly once however many times they are replayed, and the Edge issues no dose to
a reconnecting plant until reconciliation completes.

**Rationale.** Two failure modes meet at the reconnection seam. Replaying a
buffered dose twice inflates history and the budget. Issuing a fresh dose to a
plant that autonomously watered ninety seconds ago delivers double water to a
plant that already has enough — and the Edge cannot know that until it has read
the buffer.

**Enforcing components.**
- *Device*: stable `event_id` per buffered event, generated once at buffering
  time and never regenerated on replay; events retained until acknowledged.
- *Edge*: dedup on `event_id` through the existing `processed_messages`
  transaction; plant held in `Uncertain` until the device signals replay complete
  and the Edge has committed it.

**Persisted state required.** Device event ring with ids and `device_seq`;
`last_reconciled_seq`; edge `processed_messages`, `watering_events.origin`.

**Failure scenarios covered.** Device disconnects mid-replay and reconnects; edge
crashes mid-reconciliation; broker redelivers a replay batch; device reboots
after replay but before acknowledgement; two reconnections racing.

**Planned tests.**
- `safety_016_replay_is_idempotent` — **green since M2**
  (`device-simulator`, `tests/replay.rs`), with
  `safety_016_event_id_is_stable_across_every_replay` (unit) and
  `event_ids_survive_a_restart_unchanged`. The device half is complete; the edge
  half remains M6.
- `safety_016_replay_is_idempotent` (property): replay a buffered set an
  arbitrary number of times in arbitrary order; assert one `watering_event` per
  distinct `event_id`.
- `safety_016_no_dose_before_reconciliation_completes` (integration, M6): assert
  the Edge publishes no command while a plant is reconciling.
- `safety_016_edge_crash_midreplay_loses_nothing` (integration).
- SCEN-100, SCEN-101, SCEN-102.

**Enforced since M6 (2026-08-31)** on the edge; the device side shipped in M2;
re-verified M8. The hold is derived from `replay_progress` rather than stored in
a flag, so a routine `device.status` cannot clear it.

**"Exactly once" also means "to exactly one plant"** (post-M6 correction,
2026-08-31). Idempotence on `event_id` guarantees one row per dose; it says
nothing about *whose* row it is. Attribution originally resolved the plant from
`actuator_bindings` at replay time, which asks a question about the present and
applies the answer to the past — and an isolated device is precisely when an
operator has time to rebind a pump. A dose delivered to plant A while A was
alone was then charged to plant B, leaving A with a clean budget and free to be
watered again immediately. A replayed `watering.offline_autonomous` event now
carries `detail.plant_id` (protocol §5.4), written onto the `watering_events`
row inside the same transaction as the event, so the charge is fixed when the
history is committed. Covered by
`safety_016_a_replayed_dose_is_charged_to_the_plant_the_device_named` and
`safety_016_a_named_dose_survives_repeated_replay_and_further_rebinding`, with
`without_a_named_plant_the_dose_follows_the_binding` as the negative control
that pins the fallback for a device predating the field.

---

## SAFETY-017 — A required measurement that is missing or stale blocks action

**Statement.** A plant's offline policy names the measurements it requires. If any
required measurement is absent, stale beyond its limit, or of non-`Ok` quality,
autonomous actuation is refused. Advisory measurements never gate actuation.

**Rationale.** SAFETY-005 says the same thing about the Edge's control loop; this
extends it to the device and makes it *per plant*. It also states the converse,
which matters just as much: a plant that never had a pot scale must not be
blocked by the absence of one. Requirements are declared, not inferred.

**Enforcing component.** Device, gate steps 6–7. The `role` on each
`SensorBinding` (`Control` / `Required` / `Advisory`) is the source of truth
([ADR-016](../adr/016-plant-binding-and-policy-model.md)).

**Persisted state required.** The required-measurement list inside the persisted
policy; per-kind last-sample timestamps in device RAM.

**Failure scenarios covered.** Probe unplugged mid-isolation; sensor returns
`Fault` quality; one of several required kinds goes stale; a kind the plant never
bound is missing (must **not** block); uncalibrated sensor reporting a value.

**Planned tests.**
- `safety_017_missing_required_blocks` (property): for each required kind,
  remove it and assert refusal.
- `safety_017_missing_advisory_does_not_block` (property) — the converse.
- `safety_017_non_ok_quality_blocks_control` (unit): `Uncalibrated`, `Suspect`,
  and `Fault` are all unusable for control.
- `safety_017_unbound_kind_is_irrelevant` (unit).
- SCEN-099, SCEN-105.

**Enforced since M6 (2026-08-31)**, on both the offline path
(`safety_017_missing_required_blocks`) and the edge's own gate, which reads the
`required` role from the plant's bindings. The converse,
`safety_017_missing_advisory_does_not_block`, is asserted alongside it.

---

## SAFETY-018 — A plant with no actuator binding has no actuation path

**Statement.** A monitoring-only plant cannot be watered by any path: no command
may be issued for it, no offline policy may enable automation for it, and the API
refuses the attempt distinguishably from a safety refusal.

**Rationale.** Monitoring-only is the common case in a real home, not a degraded
state. Modelling it as "has a pump but it is disabled" produces a plant sitting in
a permanent lockout and a UI offering watering controls for hardware that does not
exist — which is how an operator comes to believe water is possible when it is
not.

**Enforcing component.** Edge, at validation and at the API boundary.

**Persisted state required.** Absence of a row in `actuator_bindings` — the
`[0..1]` cardinality is the mechanism.

**Distinguishable refusal.** `POST /plants/{id}/water` returns **422**
`no_actuator_bound`, not 409 (which means "refused by safety") and not 500. The
UI renders no watering controls at all rather than disabled ones.

**Failure scenarios covered.** Watering a monitoring-only plant via API; enabling
connected or offline automation without an actuator; removing an actuator binding
while automation is enabled; a policy naming an actuator the device never
declared; **applying a species preset carrying dose and cooldown defaults to a
plant with no actuator** (M5-018) — which must succeed and create measurement
policies, while creating no actuator binding and no actuation path.

**Planned tests.** All present and green as of M5 (2026-08-29):
- `api::plants::tests::safety_018_no_actuator_no_command` — `POST /water` on a
  monitoring-only plant.
- `api::offline_policy::tests::safety_018_automation_rejected_without_actuator`
  — authoring is refused, and `validate::tests::safety_018_a_policy_without_an_actuator_is_rejected`
  refuses it in the shared validator whether the policy is enabled or not.
- `api::plants::tests::safety_018_api_returns_422_not_409` — the distinction is
  the point, and the same test proves no `force`, `override`, or `bypass` field
  is honoured.
- `offline_policy::tests::safety_018_a_plant_with_no_actuator_cannot_have_an_offline_policy`
  and `binding::tests::*` — a binding naming an undeclared capability is refused.
- `api::plants::tests::safety_018_preset_creates_no_actuation_path` — applying a
  preset to a monitoring-only plant succeeds, writes its measurement policies,
  writes no `actuator_bindings` row, and leaves the plant unwaterable.
- `recommend::tests::safety_018_a_plant_with_no_actuator_is_never_advised_to_water`.
- SCEN-106, as `binding::tests::scen_106_a_monitoring_only_plant_binds_normally`
  and `api::bindings::tests::scen_106_a_monitoring_only_plant_is_first_class`.

**Enforced.** M5 (validation and API); M12 adds the UI omission.

---

## SAFETY-019 — Policy activation is atomic

**Statement.** A policy update that is invalid, interrupted, or truncated leaves
the previously active policy in force. At every instant exactly one valid policy
is active, or none.

**Rationale.** The dangerous failure is a half-written policy taking effect — a
device acting on a dose field from the new policy and a cooldown from the old.
Power loss during a flash write is not exotic; it is the normal way an
unmaintained device eventually fails.

**Enforcing component.** Device: validate → stage → verify read-back → atomic
activate → acknowledge ([offline-autonomy.md](offline-autonomy.md) §7). Steps
before activation are non-destructive.

**Persisted state required.** Separate `policy_blob` and `policy_staging` regions,
each CRC-protected, plus an atomically flipped active pointer.

**Failure scenarios covered.** Power loss at each step; CRC failure on read-back;
policy exceeding a firmware hard limit; policy naming an undeclared capability;
`policy_version` at or below the applied version (retained-message replay after a
rollback); repeated redelivery of the same policy.

**Planned tests.**
- `safety_019_interrupted_activation_leaves_one_valid_policy` — **green since
  M2** (`device-simulator`, `tests/policy.rs`): the process is killed after each
  of the five steps and a reload from disk finds exactly one valid active policy
  whose checksum matches and whose version equals the acknowledged one.
  `safety_019_invalid_policy_keeps_previous` and
  `safety_019_lower_version_ignored` are green there too.
- `safety_019_invalid_policy_keeps_previous` (unit, one case per rejection reason).
- `safety_019_interrupted_activation_leaves_one_valid_policy` (property):
  interrupt at every step index; assert exactly one valid active policy after.
- `safety_019_lower_version_ignored` (unit).
- SCEN-095, SCEN-103.

**Becomes enforced.** M9 (firmware); modelled in M6 by the simulator.

---

## SAFETY-020 — Lost buffered history is reported as an explicit gap

**Statement.** When the device event buffer overflows, the loss is recorded as a
gap marker carrying the lost `device_seq` range and count, replayed on reconnect,
stored, and made visible. Audit events are never evicted to make room for
telemetry.

**Rationale.** A bounded buffer is honest; silently dropping from it is not. If an
autonomous dose disappears from history, the plant's record is wrong in the one
direction that matters — the operator and the budget both under-count water that
was actually delivered. Making the gap first-class keeps the ledger truthful even
when it is incomplete.

**Enforcing components.** Device (tiered ring, gap markers); Edge (`history_gaps`
table, API exposure, UI rendering).

**Persisted state required.** Device: ring with tier tags and eviction counters.
Edge: `history_gaps` ([ADR-004](../adr/004-sqlite-edge-persistence-model.md)).

**Failure scenarios covered.** Very long isolation; high-rate telemetry filling
the ring; an audit-event storm (repeated refusals); reboot with a partially full
ring; overflow during replay.

**Planned tests.**
- `safety_020_telemetry_never_evicts_audit` and
  `safety_020_overflow_emits_gap_marker` — **green since M2**
  (`device-simulator`): audit survives ten times the telemetry capacity, and the
  gap carries its range, count, and tier. The edge and UI halves remain M6/M12.
- `safety_020_telemetry_never_evicts_audit` (property): fill with mixed events;
  assert audit-tier survival.
- `safety_020_overflow_emits_gap_marker` (unit): assert range and count.
- `safety_020_gap_reaches_edge_and_api` (integration).
- SCEN-104.

**Becomes enforced.** M9 (device); M6 (simulator model); surfaced in M12.

---

## SAFETY-021 — Expected sleep is bounded and never masks an unexpected absence

**Statement.** A device is reported as `sleeping` only while it is inside a wake
window the **edge** computed from its own clock, and only after the device
announced the sleep. A device that is overdue past `overdue_at`, or that vanished
without an announcement, is reported as `isolated`. No configuration, and no
field of any device-supplied message, can extend a wake window indefinitely or
convert an unannounced absence into an expected one.

**Rationale.** A battery device is quiet almost all the time, so "quiet" stops
being evidence of anything ([ADR-018](../adr/018-battery-and-deep-sleep-device-mode.md)
§1). Without a bound, `sleeping` becomes the state a flat battery, a failed wake
timer, a crashed device, and a stolen node all sit in for ever, and the operator
loses the one indication that something has died. The invariant is what makes the
new state safe to add: it can only ever *defer* the offline indication, never
suppress it.

The direction of the two derivations is load-bearing. The window is computed by
the edge, so a device with a wrong clock cannot make itself look punctual — the
same reason SAFETY-005 uses `received_at`. And an unrecognised `reason` on an
offline status resolves to `connection_lost`, so uncertainty produces *absent*,
never *asleep* (SAFETY-012).

**Enforcing components.** Edge (`device` liveness derivation and the liveness
timer). The device's announcement and its `expected_wake_ms` are advisory inputs
and are never authoritative.

**Persisted state required.** Edge: `power_mode`, `wake_interval_seconds`,
`expected_wake_at`, `missed_wake_count` on `devices`. Derived per read, in the
same way and for the same reason as `stale` (PRD 040 state model).

**Failure scenarios covered.** A battery device that stops waking; a device that
announces sleep and is then unplugged; a device whose announced
`expected_wake_ms` is years in the future; a device that announces sleep with an
unrecognised reason; a device that misses one wake and returns on the next.

**The reported state is derived, not read back.** `connectivity_mode` is stored,
because the liveness timer needs somewhere to record the transition it makes, the
event it emits, and the counter it increments. What is *reported* re-checks
`overdue_at` against the edge clock on every read. The distinction is the
invariant's whole margin of safety: a stored state needs a writer, and a writer
that dies leaves a device permanently asleep — which is exactly the failure the
invariant exists to prevent, reintroduced by the mechanism meant to enforce it.
So the timer is what makes the transition durable and observable, and the
derivation is what makes it true in the meantime.

**Tests.**
- `safety_021_overdue_sleeper_becomes_isolated` (unit,
  `edge-controller::device::connectivity`): a device past `overdue_at` derives
  `isolated`, not `sleeping`, and carries no `expected_wake_at`.
- `safety_021_an_incomplete_sleep_window_is_never_reachable` (unit): a
  `sleeping` row missing either half of its window derives `isolated`, never a
  reachable state (SAFETY-012).
- `safety_021_device_wake_time_is_advisory` (unit,
  `edge-controller::device::status`): an announced `expected_wake_ms` of
  `u64::MAX` does not extend the edge-computed window.
- `safety_021_unannounced_absence_is_never_sleeping` (unit): an LWT with
  `connection_lost`, an offline status with an unrecognised `reason`, a sleep
  claim without a battery declaration, and a sleep claim with an out-of-range
  interval all derive `isolated`.
- `safety_021_sleep_window_detected_by_timer` (`edge-controller::device::health`):
  the liveness timer transitions an overdue sleeper with no inbound message, per
  F-040-09.
- `safety_021_an_overdue_sleeper_reports_isolated_with_no_expected_wake` and
  `safety_021_an_overdue_sleeper_is_isolated_after_a_restart_with_no_timer`: the
  REST projection is `isolated` across a process restart with the timer never
  run.
- `safety_021_a_retained_sleep_redelivered_after_a_broker_restart_extends_nothing`
  (integration, live Mosquitto): a retained announcement replayed on re-subscribe
  moves neither the window nor `last_seen_at`.
- SCEN-111, SCEN-112.

**Becomes enforced.** M4 — the edge derivation and the liveness timer landed in
the dated 2026-08-28 post-M4 battery-compatibility correction and its 2026-08-28
post-M4 review, not in M5. Re-verified in M8 against a sleeping simulator (which
M5-021 still owes) and in M12's presentation.

---

## SAFETY-022 — A learned estimate may narrow a watering decision, never widen one

**Statement.** No value produced by a per-plant adaptive model may increase a
dose above the plant's configured `AutomationPolicy::dose_ml`, add to a rolling
budget, shorten a cooldown, relax a staleness threshold, satisfy a required
measurement, clear or defer a lockout, or reach a device's `OfflinePolicy`. Its
only permitted effects are to propose a **smaller** dose inside
`[min_effective_ml, dose_ml]`, to supply advisory timing, and to populate
explanations. The clamp that enforces the bound is applied in exactly one place,
**before** the safety gate runs.

**Rationale.** An inference is not an observation, and the difference is the
whole reason this invariant exists. Every other safety input in the system is
something a sensor measured or a person authored; a learned coefficient is
something the system concluded, and a system permitted to act on its own
conclusions without a ceiling can be wrong in an unbounded direction. Bounding
it to "less than the operator asked for" makes the worst realistic failure
under-watering — visible, recoverable, and already surfaced — rather than a
flooded floor.

It is also what keeps the existing invariants meaningful. `safety_gate`,
`budget::dose_fits`, the cooldown, the cycle dose limit, and the firmware
ceiling all run on the clamped value and cannot tell where it came from, so
adding a model does not add a second path through any of them
([ADR-019](../adr/019-per-plant-adaptive-water-model.md) §5).

**Enforcing components.** Edge domain (`rhizo_domain::hydration::clamp_proposal`)
and the control loop that is its only caller. The device is not involved: no
learned value crosses the wire, and an isolated device keeps evaluating the
statically provisioned policy ADR-015 requires.

**Persisted state required.** Edge: `plants.adaptive_mode`,
`plant_hydration_model`, and `plant_adaptive_decisions` recording the proposal,
the clamped value, and which was applied. Nothing on the device.

**Failure scenarios covered.** A model that proposes more than the configured
dose; a model that proposes a dose crossing the rolling 24-hour cap; a corrupt
or undecodable model row; a model that is confident about a plant whose probe
has drifted; a cold-start plant in `adaptive` mode; an epoch reset that would
otherwise let superseded observations back into an estimate.

**The clamp precedes the gate, and that ordering is the invariant.** A clamp
applied after the gate would leave a window in which an unclamped volume existed
inside the decision path, and a gate taught to recognise adaptive doses would be
a gate with two paths through it. One clamp, before everything, is what makes
"the model cannot widen anything" checkable by reading a single function.

**Planned tests.**
- `safety_022_adaptive_dose_never_exceeds_the_static_dose` (unit,
  `rhizo-domain::hydration`): a proposal above `dose_ml` clamps to it, for every
  generated policy and history.
- `safety_022_adaptive_dose_cannot_cross_the_rolling_cap` (unit,
  `rhizo-domain::irrigation`): a clamped dose is offered to the unchanged
  `budget::dose_fits`, and the 24-hour total never exceeds `max_daily_ml`.
- `safety_022_adaptive_dose_cannot_shorten_a_cooldown` (unit): no model value
  reaches `cooldown_until`.
- `safety_022_a_model_cannot_clear_a_lockout` (unit): an active lockout is
  unaffected by any model state, including a reset epoch.
- `safety_022_a_missing_or_corrupt_model_falls_back_to_the_static_policy`
  (`edge-controller::hydration`): an undecodable model row produces today's
  decision and one warning.
- Property: for arbitrary observation histories, the issued volume is finite and
  `<= automation.dose_ml`.
- Scenarios registered by M15-014.

**Becomes enforced.** M15 — specifically M15-012, the only issue in that
milestone permitted to change a volume that reaches a pump. Until then the model
is inference only, and every mode below `adaptive` is inert by construction.

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
