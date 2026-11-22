# Rhizo Edge — Roadmap

**Implementation status:** M0–M8 complete; M9 IN PROGRESS with preserved acceptance gaps; M10 READY to start. No new hardware functionality is claimed by this planning update.

## Strategy — permanent change, 2026-11-22

Deliver a working end-to-end prototype quickly, connect the actual hardware,
and improve it through supervised use with real plants. Build the simplest
functional vertical slice first, retain essential electrical and watering
protections, and defer deeper validation. Reuse the Rust Edge Controller,
MQTT, SQLite, thresholds, command lifecycle, and existing firmware foundation.
No architecture redesign or new milestone is needed.

This decision supersedes earlier all-criteria milestone sequencing, blanket
verification gates, and the November M9 freeze's restriction on starting M10.
ADRs, historical reports, evidence and incomplete M9 checkboxes remain intact;
changing execution priority is not a passing test. Existing safety behavior and
protocol contracts remain binding. Battery/offline autonomy and later ADR
features remain supported directions, not prerequisites for this prototype.

## 1. Milestone table

| ID | Name | Objective | Depends on | Issues | Status |
|---|---|---|---|---|---|
| M0 | Foundation and Engineering Baseline | A clean Rust repository whose tooling, lint, test, container, and observability baseline every later milestone inherits | — | 13 | **DONE** |
| M1 | Domain Model and MQTT Protocol | The shared wire contract, typed measurement kinds, capability and offline-policy payloads, and the pure domain and policy crates | M0 | 19 | **DONE** |
| M2 | Device Simulator | A host device indistinguishable from firmware at the protocol/mechanics level, including offline-policy persistence, isolation/replay mechanics, fault injection, and virtual time; policy evaluation activates in M6 | M1 | 19 | **DONE** |
| M3 | Edge Ingestion and SQLite | Reliable MQTT consumption with durable deduplication and crash-safe persistence | M1, M2 | 18 | **DONE** |
| M4 | Device Registry and Health | Device lifecycle, staleness, sensor health, config drift, and the first REST surface | M3 | 13 | **DONE** |
| M5 | Plant Model and Recommendations | Plants, **bindings, per-measurement thresholds**, offline-policy authoring, species presets, trends, and an explainable recommendation engine — **issuing no commands** | M4 | 22 | **DONE** |
| M6 | Irrigation Control and Safety | The state machine, the safety gate, the command lifecycle, the **offline evaluator and reconciliation**, and every non-hardware SAFETY invariant | M5, M2 | 24 | **DONE** |
| M7 | Cloud API and PostgreSQL | Optional idempotent history sync that cannot affect local safety | M6 | 15 | **DONE** |
| M8 | End-to-End Test Environment | The whole software system reproducible and verifiable with one command, no hardware | M7 | 18 | **DONE** |
| M9 | ESP32 firmware foundation | Reuse demonstrated firmware; resolve relevant gaps when integrating hardware | M8 | 22 | IN PROGRESS |
| M10 | Real soil sensor | SEN0601 over RS485 through ESP32-C3, MQTT, SQLite and API | Existing M9 sensor/config/telemetry interfaces; not M9-022 | 13 | READY |
| M11 | Real pump and essential safety | Bounded DFR0523 watering and first supervised plant experiment | Existing firmware interfaces; real M10 readings before closed-loop watering | 14 | READY |
| M12 | Minimal Rust interface | Measurements, thresholds, manual water/stop and fault display | Existing M6 REST API; not M11 completion | 19 | READY |
| M13 | Multi-plant home | Scale after experience with one plant | M12 when needed | 17 | DEFERRED |
| M14 | Field readiness | Field/cloud-scale planning only when needed | M13 when needed | 10 | DEFERRED |
| M15 | Adaptive water model | Learn from actual usage later | M13 | 14 | DEFERRED |
| M16 | Verified watering | Additional physical delivery witness later | M11 | 16 | DEFERRED |

**286 issues** are retained; no new issues or milestones are introduced.
READY means implementation can start, not that wiring is verified or watering
is authorized. IN PROGRESS means unfinished work remains. DEFERRED means outside
the current prototype path, not passed. DONE requires the current scoped exit
criteria to be demonstrated; explicitly deferred work stays open in the backlog.
M9 stays IN PROGRESS rather than being retroactively declared DONE.

## 2. Shortest path to the first plant watering

Execute the following in order. Sensor work starts immediately; pump and UI
implementation may proceed independently once their own interfaces are available.

1. **Real sensor path:** M10-001 → M10-002 → M10-003 → M10-004.
   Select a real adapter behind the existing sensor trait; implement only the
   Modbus operations and register map needed for SEN0601 first. Keep pump power
   disconnected. Inspect actual connectors, wiring, polarity, supply and RS485
   isolation before power. Start with one sensor; add other addresses later.
2. **Usable measurements:** M10-006 → M10-007 → M10-008 → M10-009 → M10-013.
   Configure units/scaling and calibration provenance, get plausible repeatable
   readings in the intended soil, and demonstrate real telemetry in SQLite/API.
   Missing, invalid or stale soil readings must block automatic watering. Reuse existing Edge
   bindings and thresholds; fix an actual integration defect if found instead
   of enforcing an artificial zero-Edge-diff rule.
3. **Bounded pump path:** M11-001…M11-008. Implement the DFR0523 adapter, an
   independent run guard, faults, calibration, and the existing required tank/leak
   inputs. Reuse shared validators, caps, cooldown and durable accounting.
   Do not substitute fake healthy inputs for absent hardware. Resolve M9 runtime
   gaps only where they affect this always-on commanded-water path.
4. **Minimal interface:** M12-001 → M12-002 → M12-003/M12-004 → M12-006,
   plus M12-008 → M12-013 → M12-014 and M12-010 → M12-019.
   One plant view is enough: current measurements and age, editable thresholds,
   dose/frequency/volume limits, manual water, stop, and clear fault/refusal state.
   Use the existing REST API and Rust UI stack; no cloud or packaging project.
5. **Cup before plant:** M11-009…M11-012. With pump disabled first, check
   startup/reset/watchdog and loss-of-control-signal safety on the actual driver.
   Then calibrate small doses into a cup; demonstrate stop, maximum runtime,
   volume cap/cooldown, invalid or missing inputs, communication loss and safe
   restart without duplicate watering. Keep a reachable power disconnect.
6. **First real plant:** M11-013 → M11-014. Use a small configured dose under
   human supervision, see measurements and a threshold decision, deliver water,
   stop, and observe the result through the interface. Record a short result
   with limits and any remaining fault. No week/month waiting period gates this.
   Improve autonomous operation incrementally from actual use; no unattended
   watering until essential fail-safe behavior is demonstrated.

### Prototype exit criteria

A real plant is monitored and watered through SEN0601 → ESP32-C3/RS485 → MQTT →
Rust Edge → SQLite/threshold decision → DFR0523, with a minimal usable interface.
Measurements are observable; thresholds and watering limits are configurable.
Duration, per-dose and cumulative volume, and frequency are bounded; calibration
makes time-derived volume a conservative estimate, not a measured-delivery claim.
Faults fail closed. Stop and power disconnect work, and ordinary sensor/network
loss and restart recover without uncontrolled or duplicate watering.

Use an always-on, externally powered, single-plant configuration initially;
offline autonomous watering remains disabled. Loss of communication must not
start new watering or defeat the local duration guard. An interrupted or unknown
dose is conservatively accounted for before another dose is permitted.

**Prototype tested** means this bounded configuration and these observed behaviors
worked. It does not mean **production validated**, all historical HIL passed, or
unattended operation approved. Keep existing safety invariants; claim only the
specific physical coverage obtained.

### Current milestone entry and exit

- **M9:** reuse the demonstrated toolchain, provisioning, Wi-Fi/MQTT, fake
  telemetry, shared traits and host tests. The [frozen inventory](docs/reports/M9-acceptance-freeze.md)
  remains the evidence source, not an M10 entry gate. Boot safety, runtime guard,
  actual sensor health, interrupted-dose handling and conservative accounting
  matter before real watering. Sleep campaigns, full conformance and unrelated
  audit/CI gaps do not block sensor implementation.
- **M10:** entry is the existing sensor/config/telemetry interfaces. Exit is
  one real SEN0601 path, configured scaling/calibration, useful readings, and
  missing/invalid/stale soil rejection for automatic watering through the local system. No cloud,
  analogue sensor, four-week drift or energy campaign is required.
- **M11:** driver and input implementation can start now. Physical actuation
  waits for verified electrical/control safety; closed-loop watering waits for
  valid real inputs. Exit is essential cup checks and one supervised plant
  cycle, with reliable stop and conservative limits.
- **M12:** start against existing REST endpoints now. Exit is the minimal
  interface above on the development host, including refusal/disconnection
  handling. Charts, installers, fleet and cloud views are deferred.

### Deferred hardening and expansion backlog

Reuse these issues; do not generate a new validation project:

| Existing work | Disposition / when to revisit |
|---|---|
| M9-022 and unresolved items in the frozen inventory | Preserve all evidence; address immediate watering-path defects during integration; remaining exhaustive conformance, mutation/CI, replay/audit and endurance coverage after prototype usage |
| M9-019…M9-021 battery/sleep coverage | Before enabling battery/deep-sleep operation; not for the always-on prototype |
| M10-005 | Analogue adapter only if a real need arises |
| M10-010 | Gravimetric accuracy and multi-week drift after initial soil observations |
| M10-011 | Full stabilization distribution across temperatures before duty-cycled deployment; basic startup settling still checked now |
| M10-012 | Power profiling and battery lifetime claims later |
| Extended HIL-1…HIL-7 campaigns | Repeated reset/power-cut matrices, endurance and exhaustive fault injection later; essential driver behavior remains a pre-actuation gate |
| M12-005, M12-007, M12-009, M12-011, M12-012, M12-015…M12-018 | Rich device/history/cloud views, charts, packaging and multi-platform CI after usable controls |
| M13…M16 | Fleet/scalability, field/Helm, adaptive models and new delivery witnesses after real usage establishes need |
| Additional cloud and production reliability work | Existing optional cloud stays available but disabled for the prototype; no new cloud dependency |

Use available hardware. Do not require a new sensor, driver, battery, scale,
oscope or interlock merely to finish a checklist. Confirm any actually missing
safe power wiring, connectors or means to measure a small calibration dose before
pump use. A concrete electrical or uncontrolled-watering risk still blocks that
specific operation until resolved.

### Completed milestone record

The following M0–M8 descriptions retain their delivered scope; they are not
instructions to repeat historical verification on every feature.

### M0 — Foundation and Engineering Baseline · DONE

**Objective.** Establish workspace, toolchain, lint policy, configuration,
observability, containers, and CI so that every later milestone adds behaviour
rather than scaffolding.

**Deliverables.** Three-workspace layout with `firmware` and `ui` excluded ·
`rust-toolchain.toml` pinned to **1.98.0** · `unwrap_used`/`expect_used` denied
in libraries · `rhizo-telemetry` (tracing + Prometheus registry) · layered
edge configuration with fail-fast validation and secret redaction · full-jitter
backoff utility · `rhizo-testkit` with `TestClock` · Mosquitto with
authentication and `%u` ACLs · Compose skeleton · CI gate · documentation
validator.

**Exit criteria.** On a **fresh clone**: `cargo fmt --check`, `cargo clippy -D
warnings`, `cargo test --workspace`, `docker compose config`, and the docs
validator all pass. Mosquitto refuses anonymous connections and cross-device
publishes. Invalid configuration exits non-zero naming the key. `Debug` on the
config prints `[redacted]`.

**Invariants.** None enforced. Enables SAFETY-007 (workspace layout for the
shared contract crate) and SAFETY-012 (fail-fast posture, deny-unwrap).

**PRD.** [000](docs/prd/000-platform-foundation.md)

---

### M1 — Domain Model and MQTT Protocol · DONE

**Objective.** Define the contract that cannot be changed cheaply once devices
exist in pots.

**Deliverables.** `rhizo-mqtt-contract` (`no_std` + `alloc`) implementing
[mqtt-v1.md](docs/protocol/mqtt-v1.md) · `DeviceId` grammar · `UtcMillis` ·
envelope with identity checking · topic grammar · **all MQTT v1 message and
payload types defined by the normative protocol** (§5.2–§5.12: batched typed
telemetry, actuator state, device events, status with declared capabilities, LWT,
config, the three command kinds, command results, the offline policy, and
`edge.time`) · the `MeasurementKind` enum with its `const fn spec()` · the
`TimeSyncState` strict-acceptance helper · **`validate_water_command`, the single
shared actuation gate** · protocol fixture corpus · `rhizo-domain` skeleton with
the `Clock` trait · `no_std` CI verification · clippy ban on direct clock access
in the domain.

**Exit criteria.** Every clause of mqtt-v1.md §2–§10 implemented or explicitly
noted, checked against the §11 conformance checklist. `validate_water_command`'s
ordering matches §5.8 exactly. `TimeSyncState` accepts only a **strictly** newer
`edge_time_ms` (§5.12). The contract crate builds for a bare-metal target with
default features off. `Utc::now()` in `rhizo-domain` fails clippy. Every fixture
behaves as documented: each valid file decodes into its **concrete payload type**
and survives a re-encode without losing a stated field, and each invalid file
fails with its **named** typed variant.

**Invariants.** Delivers the mechanism for SAFETY-002, SAFETY-007, SAFETY-012;
enforcement tested in M6.

**PRD.** [010](docs/prd/010-domain-and-mqtt-protocol.md)

---

### M2 — Device Simulator · DONE

**Objective.** A reference device that makes M3–M8 achievable without hardware —
and that is **never more permissive than firmware**.

**Deliverables.** Full protocol conformance including LWT and retained status ·
**declared sensor and actuator capabilities**, so the edge assumes nothing ·
batched typed measurements across the kinds the simulated hardware exposes · soil
model with absorption lag, probe overshoot, and drainage · weight rising
immediately while VWC lags · tank, leak, EC models · **actuation exclusively
through `validate_water_command`** · **wall clock maintained solely from
`edge.time`**, with strict duplicate rejection · NVS-equivalent persistent state ·
**a persisted, versioned offline policy applied atomically** · isolation and
reconnect state plus persisted monotonic evaluator inputs, but **no offline
decision or autonomous dosing until M6-019 installs the single shared
`rhizo_policy::evaluate_offline` call site** · **a bounded offline event buffer with
ordered replay and explicit gap reporting** · control API · thirteen injectable
faults · accelerated virtual time.

**Exit criteria — met, with evidence in
[docs/reports/M2.md](docs/reports/M2.md).** Runs standalone against a bare broker
(`docker compose --profile devices up mosquitto device-simulator`; telemetry read
back with `mosquitto_sub`). Exactly one call site of `validate_water_command` and
**no offline evaluator or autonomous-dose call site yet**, both checked
structurally rather than asserted.
`requested_ml: 10000` published directly to the broker delivers 80 ml — the
compile-time limit — and the reservoir confirms it. No retained messages on
command, telemetry, event, or `time` topics, verified after a full command cycle
by a fresh subscriber, with both negative controls run and reverted.
A replayed `edge.time` never extends `clock_synced`. An isolated simulator keeps
sampling and buffering through two simulated days on a bone-dry plant with an
enabled policy and never actuates. ACL isolation holds, and an over-permissive
ACL pattern fails the test. A full commanded cycle completes in **1.4 s** at
scale 600.

**Invariants.** Makes SAFETY-002, SAFETY-007, SAFETY-011, policy activation, and
buffer/replay mechanics testable before hardware exists — each now with a green
test in `crates/device-simulator`. M6-019/M6-021 provide the first executable
autonomous-device evaluation for SAFETY-013…017.

**PRD.** [020](docs/prd/020-device-simulator.md)

---

### M3 — Edge Ingestion and SQLite · DONE

**Objective.** Consume telemetry reliably and persist it crash-safely.

**Deliverables.** MQTT ingress with reconnect **and re-subscription** · decoding
with edge-stamped `received_at` · bounded, rate-limited quarantine ·
**the deduplicate-and-persist transaction** · per-field validation that keeps
good fields · full SQLite schema and migrations · `.sqlx` offline cache ·
ingestion metrics with a cardinality guard · failure classification · graceful
shutdown and startup restoration · retention that never prunes the ledger.

**Exit criteria.** Duplicate `message_id` produces one row. Edge restart
preserves history. **Broker restart results in re-subscription**, verified by
telemetry resuming. A message with one bad field stores the rest. SIGTERM exits
0; a task panic exits non-zero.

**Invariants.** Delivers the mechanism for SAFETY-001 and SAFETY-010; establishes
`received_at` authority that SAFETY-005 depends on.

**PRD.** [030](docs/prd/030-edge-ingestion-and-storage.md)

---

### M4 — Device Registry and Health · DONE

**Objective.** Know whether a device is there, and how fresh its data is.

**Deliverables.** Status ingestion and LWT handling, order-insensitive ·
**auto-registration that creates a device and never a plant** · staleness
derived from `received_at` with a liveness **timer** · sensor health
distinguishing absent from unhealthy · config drift detection · health endpoints
excluding cloud reachability · device REST endpoints · API server with CORS off
by default.

**Exit criteria.** Killing the simulator marks it offline. Restart yields a new
`boot_id` without a spurious regression flag. Stopping telemetry while connected
produces a stale indication **from the timer**. An unknown device registers with
no plant. `/health/ready` is 200 with the cloud stopped, 503 with the broker
stopped. No endpoint changes `device_id`.

**Exit criteria added by the dated post-M4 battery correction.** A device inside
its announced window reports `sleeping`, not `offline`; one past `overdue_at`
reports `isolated`, with the durable transition made **by the liveness timer with
no inbound message** and the reported value derived on every read regardless of
the timer. An announced wake time far in the future extends nothing. A retained
announcement redelivered after a broker or edge restart moves neither the window
nor the missed-wake count. An always-on device's liveness behaviour is unchanged.

**Invariants.** Provides SAFETY-005's inputs — and takes no power field into that
threshold; applies SAFETY-012 to onboarding; enforces **SAFETY-021** (expected
sleep is bounded and never masks an unexpected absence).

**PRD.** [040](docs/prd/040-device-registry-and-health.md)

**Evidence.** [M4 milestone report](docs/reports/M4.md).

---

### M5 — Plant Model and Recommendations · DONE

**Objective.** Turn telemetry into an explainable recommendation — while still
issuing no commands, so the logic can be validated against a real plant for a
week before anything can pump.

**Deliverables.** Plant CRUD with `auto_watering_enabled` defaulting false ·
**explicit `SensorBinding[]` with `control`/`required`/`advisory` roles, bound to
declared device capabilities** · **an optional `ActuatorBinding` (0..1), whose
absence is a normal monitoring plant, not a degraded one** · **per-measurement
policies: target band, warning and critical thresholds, staleness, hysteresis,
confirmation duration** · **threshold evaluation raising alerts that never cause
watering** · **authoring and validating the per-plant offline policy the device
may later act on** · profile validation that **rejects rather than clamps**, with
the profile demoted to a template that pre-populates policies · least-squares
moisture trend returning `None` on sparse data · dry-duration tracking with gap
handling · manual-watering detection with command attribution · stuck-sensor
detection · rule-based recommendation with typed reasons · plant state
derivation · EC trend and warning · evaluation tick and endpoints ·
**a versioned, embedded, curated species preset catalogue with per-value
provenance, and its application into ordinary per-plant policies** ·
**the remaining battery measurement/config wire surface and a simulator that
sleeps** (M5-019, M5-021). The sleep-status wire surface and the whole sleep-aware
liveness model are already delivered and enforced in M4; M5 owns only what can
*produce* that behaviour and the battery telemetry and desired-config fields.

**Exit criteria.** Drying produces `WaterRecommended` with a non-empty reason
list and **zero MQTT commands published**. A plant with no `ActuatorBinding`
returns **422**, not 409, from every actuation path, and still receives telemetry,
thresholds, and alerts. A binding to a capability the device never declared is
rejected. A critical temperature raises an alert and waters nothing. A profile
with `dose_ml = 200` is rejected with 422 naming the firmware limit. A moisture
step following a command creates no second event. A plant created from a species
preset has ordinary, fully editable `MeasurementPolicy` rows, still has
`auto_watering_enabled = false`, and **no catalogue entry contains an interval or
schedule** — a preset stores preferences, never a timer.
A **simulated** battery device drives the M4 liveness model end to end: it sleeps
and shows as `sleeping`, `miss-wake:2` leaves it `isolated`, and
`sleep-without-announcing` fires the Last Will and also yields `isolated`.
`--power-mode always_on` behaviour is unchanged and the whole M2 suite stays
green.

> **Corrected 2026-08-29, at the M5 gate.** This criterion originally also
> required `missed_wake_count == 2` after `miss-wake:2`. It is unreachable, and
> the reason is a property of M4 rather than a gap in M5: a *missed* wake
> announces nothing, so it opens no new window, and the liveness timer counts
> **at most one miss per window** — the rule M4's own
> `each_sleep_cycle_can_miss_at_most_once` asserts. Two consecutive missed wakes
> therefore leave the device `isolated` with `missed_wake_count == 1`. The
> alternative would be a device that wakes far enough to re-announce a sleep it
> is not really taking, or a counter that runs away on a single window; neither
> is better than an honest one. `isolated` — the safety-relevant half — is
> unaffected and is asserted end to end against a real broker.

**Invariants.** Enforces SAFETY-018 (no actuator binding ⇒ no actuation path);
re-verifies **SAFETY-021**, which M4 enforces, against a device that can actually
sleep. Otherwise builds the gate's inputs without actuating.

**PRD.** [050](docs/prd/050-plant-model-and-recommendations.md)

**Evidence.** [M5 milestone report](docs/reports/M5.md).

---

### M6 — Irrigation Control and Safety · DONE

**Objective.** The milestone where software can move water. The most
safety-critical in the project.

**Deliverables.** `IrrigationInputs` with `Option` for every absent-able input ·
**the safety gate, exhaustive with no catch-all arm** · leak/tank/validity/
staleness checks · the pure, total state machine · the rolling 24-hour window
derived from rows · command persistence **before** publication · publication
with `retain = false` · result handling that never invents a watering event ·
**retry with the same `command_id`, never a new one** · restart reconciliation ·
config publication · control metrics · clock-step detection · watering and
lockout endpoints with **no override parameter** · no-delivery detection ·
**`rhizo_policy::evaluate_offline`, the single shared offline evaluator — pure,
`no_std`, taking elapsed time as a parameter and reading no clock** ·
**reconciliation of offline actions on reconnect, idempotent by `event_id`, with
the plant held `Uncertain` and no dose issued until replay completes** ·
**durable command intents for sleeping devices, minted into ordinary commands at
the next wake with the full gate re-run against current inputs** · the full
property-test suite, including the offline safety properties.

**Exit criteria.** `cargo test safety_` fully green. `PROPTEST_CASES=10000 cargo
test safety_006` passes. `POST /water` during a leak returns 409 with nothing
published. Restart after publish produces no second command and one watering
event. Replaying a buffered offline batch three times out of order produces one
`watering_event` per `event_id` and one budget charge. No `_ =>` arm on any
safety match. A `POST /water` to a sleeping device **publishes nothing**, returns
`pending_for_device_wake` with no `command_id`, survives an edge restart, and
delivers exactly one command whose `issued_at` is the wake instant; a leak raised
while the device slept refuses it at delivery.
**A `command.result` acknowledged to a device survives an edge
crash between receipt and commit** — a broker PUBACK is not proof of durable
commit, and M6 may not enable real watering until that is closed
([M6-010](docs/issues/M6/010-implement-command-result-handling.md), carried
forward from M3).

**Invariants enforced.** SAFETY-001, -002, -003, -004, -005, -006, -007 (via the
shared validator), -010, -012, and — against the simulator as the reference
device — SAFETY-013, -014, -015, -016, -017. SAFETY-019 and SAFETY-020 are
firmware-owned and land in M9.

**PRD.** [060](docs/prd/060-irrigation-control-and-safety.md)

---

### M7 — Cloud API and PostgreSQL · DONE

**Objective.** Optional history that cannot become load-bearing.

**Deliverables.** Cloud API with idempotent batch ingestion returning **per-event
results where `duplicate` is a success** · two-layer schema with
`UNIQUE (edge_id, event_id)` · order-insensitive projections · cloud client with
exhaustive classification · outbox drain fully decoupled from control · adaptive
batch sizing · **value-tiered cap that never prunes high-tier events** · sync
metrics · the differential independence test · time round-trip test ·
reprojection command · read endpoints with **no command or config-write route**.

**Exit criteria.** With the cloud stopped, every local function works and
`/health/ready` returns 200. Replay creates no duplicate rows.
`safety_009_decisions_identical_with_cloud_down` passes. `rhizo-domain` has no
cloud dependency.

**Invariants enforced.** SAFETY-008, SAFETY-009.

**PRD.** [070](docs/prd/070-cloud-sync-and-storage.md)

---

### M8 — End-to-End Test Environment · DONE

**Objective.** Make the whole software system reproducible in one command, and
prove the safety claims are detected rather than assumed.

**Deliverables.** Dockerfiles with correct signal handling · complete Compose
topology · test overlay with **restart policies disabled** · time-scale
agreement check · Rust scenario runner asserting on observable state with
failure dumps · fourteen e2e scenarios · isolation, reconciliation, and
**battery/sleep** scenarios · the eighteen-step first demo ·
**seven-mutation verification** · CI job.

**Exit criteria.** `docker compose up --build` works from a fresh clone. The
suite runs with one command, exits 0, under 10 minutes. `scenario_first_demo`
reproduces all eighteen steps. **Each of the seven mutations turns the suite
red**, including the one that publishes immediately to a sleeping device.

**Invariants.** Re-verifies SAFETY-001…-010 and -012 in the assembled system,
SAFETY-013…-020 in the isolation and reconciliation scenarios, and SAFETY-021 in
the battery/sleep scenarios — the set §4's *Re-verified* column names. It
enforces none of its own.

**PRD.** [080](docs/prd/080-end-to-end-test-environment.md) ·
**Report.** [docs/reports/M8.md](docs/reports/M8.md)

> **M8 is the software-only demo.** It requires no ESP32, no pump, no plant, and
> the M12 desktop UI is deliberately **not** part of it.

---

## 4. Safety invariant ownership

Full registry: [docs/architecture/safety-invariants.md](docs/architecture/safety-invariants.md).

| ID | Invariant | Enforcing component | Enforced at | Re-verified |
|---|---|---|---|---|
| SAFETY-001 | Duplicate command → single watering | Edge + Device | M6 (edge), M9 (device) | M8, M11 |
| SAFETY-002 | Expired command never executes | Device | M6 (sim), M9 (fw) | M8, M11 |
| SAFETY-003 | Leak disables all watering | Edge + Device | M6 | M8, M11 |
| SAFETY-004 | Tank below minimum disables watering | Edge + Device | M6 | M8, M11, M13 |
| SAFETY-005 | Stale/invalid moisture disables auto-watering | Edge | M6 | M8, M10 |
| SAFETY-006 | Rolling 24 h cap never exceeded | Edge | M6 | M8 |
| SAFETY-007 | Device hard maximum cannot be bypassed | Device firmware | M6 (validator), M9 (fw) | M8, M11 |
| SAFETY-008 | Cloud outage cannot disable monitoring | Edge | M7 | M8 |
| SAFETY-009 | Cloud outage cannot bypass safety | Edge (structural) | M7 | M8 |
| SAFETY-010 | Edge restart cannot replay a completed command | Edge | M6 | M8 |
| SAFETY-011 | Device restart converges to pump-off | Device | M9 | M11 |
| SAFETY-012 | Uncertainty defaults to no watering | Edge domain + device | M6 | M8 |
| SAFETY-013 | Autonomous action needs a validated persisted policy | Device | M6 (sim), M9 (fw) | M8 |
| SAFETY-014 | Offline doses obey the same caps and hard limits | Device + Edge | M6 | M8, M11 |
| SAFETY-015 | Clock uncertainty never grants budget or shortens cooldown | Device | M6 (sim), M9 (fw) | M8 |
| SAFETY-016 | Offline actions reconcile exactly once | Edge + Device | M6 (edge), M9 (device) | M8 |
| SAFETY-017 | Missing/stale required measurement blocks autonomous action | Device | M6 | M8 |
| SAFETY-018 | A plant with no actuator has no actuation path | Edge | M5 | M8, M12 |
| SAFETY-019 | Policy activation is atomic | Device | M9 | M8 |
| SAFETY-020 | Lost buffered history is reported as an explicit gap | Device + Edge | M9 | M8 |
| SAFETY-021 | Expected sleep is bounded and never masks an unexpected absence | Edge | M4 | M8, M12 |
| SAFETY-022 | A learned estimate may narrow a watering decision, never widen one | Edge domain | M15 | M15 |
| SAFETY-023 | An unknown delivery outcome is never credited as zero | Edge | M16 | M16 |
| SAFETY-024 | Water movement no command authorised is a fault | Device + Edge | M16 | M16 |

Every invariant names at least one automated test in the registry, and no
milestone is complete until the tests for the invariants it *enforces* exist and
pass. M6 and M7 met that bar on 2026-08-31; SAFETY-011, -019, and -020 wait on
M9, and SAFETY-022 … -024 on M15 and M16.

---


## 5. Planning conventions

Issue identifiers and links remain stable. Dependencies express required
functionality, not completion of every predecessor's historical validation.
Use [the dependency graph](docs/architecture/dependency-graph.md) and the current
issue. Partial prototype scope must not be reported as full deferred scope.
A checked criterion means demonstrated; deferred criteria stay unchecked.

Run relevant unit tests and a basic integration smoke for changed behavior.
Add tests for important behavior, regressions and realistic failures. Reuse
existing passing evidence; repeat expensive checks only for a relevant change,
failure or concrete unresolved risk. Documentation-only work needs docscheck and
a diff/link review, not Rust product builds or hardware campaigns. Follow
[testing policy](docs/testing/strategy.md). Keep summaries short: change, checks,
remaining work. No new extensive reports, matrices or speculative requirements.

## 6. Toolchain

Use existing Rust workspaces and pinned toolchains; host MSRV is 1.100.0.
See [firmware build instructions](firmware/README.md) and ADR-007 for the ESP
exception. Prefer proven commands and native Windows host Rust tools; use WSL2
for Docker/hardware as documented locally. Keep files LF.

## 7. Scope boundaries

No cloud control, forced watering bypass, nutrient inference from EC, new
architecture, Kubernetes deployment, or production reliability programme is
needed for the prototype. Preserve existing Rust/shared-policy/API boundaries.

## 8. Implementation starting point

```text
M10-001 — Define the real sensor adapter structure
```

Implement configuration selection for SEN0601/Modbus behind the existing
`SoilSensor` interface, retaining fakes for host tests and rejecting unknown
configuration. Then implement the bounded Modbus read path in M10-002.
No hardware power or pump actuation is needed to start this coding task.
