# CLAUDE.md — Rhizo Edge

Working notes for Claude Code sessions on this repository.

---

## 1. Where the project is right now

**Planning is complete. M0 through M6 are implemented and green. M7 is READY and
has not started.**

| | State |
|---|---|
| Planning artefacts | ✅ complete; revised by the 2026-08-26 architecture pass |
| M0 milestone | ✅ **DONE** — report in [docs/reports/M0.md](docs/reports/M0.md) |
| Architecture pass | ✅ done — offline autonomy, per-plant policy, extensible measurements (see §11) |
| M1 | 19 issues, **DONE** — report in [docs/reports/M1.md](docs/reports/M1.md) |
| M2 | 19 issues, **DONE** — report in [docs/reports/M2.md](docs/reports/M2.md) |
| Protocol seam cleanup | ✅ done (2026-08-28) — exact device subscriptions, `event.ack`, sealed gap markers; report in [docs/reports/M2.md](docs/reports/M2.md) §Amendment |
| M3 | 18 issues, **DONE** — report in [docs/reports/M3.md](docs/reports/M3.md) |
| M4 | 13 issues, **DONE** — report in [docs/reports/M4.md](docs/reports/M4.md) |
| Battery pass | ✅ done (2026-08-28) — ADR-018 (see §12), then the post-M4 correction and its independent review; both dated in [docs/reports/M4.md](docs/reports/M4.md) |
| M5 | 22 issues, **DONE** — report in [docs/reports/M5.md](docs/reports/M5.md) |
| M6 | 24 issues, **DONE** — report in [docs/reports/M6.md](docs/reports/M6.md) |
| Post-M6 correction | ✅ done (2026-08-31) — durable `command.result.ack`, offline-dose attribution by name, and the M6 report's test-count evidence; see [docs/reports/M6.md](docs/reports/M6.md) §Post-M6 corrections and §13 below |
| M7-001 | ⬜ **next** — create the cloud-api binary and PostgreSQL service |

**This section goes stale fastest. Verify it before trusting it:**

```bash
git log --oneline -3
git status --short
ls Cargo.toml rust-toolchain.toml clippy.toml rustfmt.toml
cargo build --workspace
```

The broker-backed simulator tests need Mosquitto and a `.env`:

```bash
docker compose -f deploy/docker-compose.yml up -d mosquitto
RHIZO_REQUIRE_BROKER=1 cargo test --workspace --all-features
```

Without `RHIZO_REQUIRE_BROKER` they print a loud skip and pass, so a fresh clone
is green. **With** it — as CI sets — a missing broker is a failure, because a
suite that can silently skip its own subject eventually proves nothing.

**`Metrics::new()` is a process-wide singleton.** It caches in a `OnceLock`, so
every caller in a test binary shares one set of gauges and counters. A test that
sets a gauge and reads it back is racing every other test that touches it —
`api::health` did, and failed about one full run in three. Take
`api::health::gauge_lock()` before asserting on a shared metric, or assert on a
delta rather than an absolute.

**Never quote a bare workspace test total as evidence.** 46 tests are
broker-gated, `cargo test` captures their skip messages, and they count as
passed either way — so the workspace total is *identical* with the broker
stopped and with it running under `RHIZO_REQUIRE_BROKER=1`. Measured, not
assumed: 1 101 both times. Quote the environment and the per-suite counts, or
the number says nothing. This is what the post-M6 pass corrected in the M6
report.

Then **update this table in the same change** that moves the project on. A
CLAUDE.md that lies about the current position is worse than none.

---

## 2. What this project is

An offline-first Rust platform for soil monitoring and fail-safe irrigation:
ESP32 devices → MQTT → a Rust edge controller that owns watering decisions
→ SQLite → optional cloud. A Tauri desktop UI talks only to the edge's REST API.

Three principles drive nearly every design decision:

- **Edge-first.** The cloud is an append-only sink, disabled by default, and can
  vanish for a week without changing a single watering decision.
- **Safety-first.** Missing, stale, invalid, or contradictory input means *do not
  water* plus a visible lockout — never *water anyway*.
- **Offline-capable at every layer.** Three distinct outage modes, not one: cloud
  offline, site offline, and **device isolated**. An isolated device that was
  explicitly provisioned with a validated policy keeps the plant alive on its own
  ([connectivity-modes.md](docs/architecture/connectivity-modes.md)).

Read [README.md](README.md) once for the full picture.

---

## 3. Read these before writing code

In this order. Do not skip to the issue file.

1. **[ROADMAP.md](ROADMAP.md)** — milestones, exit criteria, conventions,
   definition of "done"
2. **[docs/architecture/dependency-graph.md](docs/architecture/dependency-graph.md)** —
   which issue is safe to execute next
3. **The PRD for the current milestone** (`docs/prd/NNN-*.md`) — what to build
4. **The issue file** (`docs/issues/M<n>/NNN-*.md`) — step-by-step scope and
   acceptance criteria

When a decision seems arbitrary, the reason is in an ADR
(`docs/adr/`). Eighteen of them, each with Context / Decision / Alternatives /
Consequences / Risks. Read the relevant one rather than re-deciding.

The four newest are the ones most likely to surprise you if you learned this
project from its earlier documents:

- **[ADR-015](docs/adr/015-device-offline-autonomy.md)** — a device provisioned
  with a validated policy **may** water while isolated. This amends ADR-003 and
  ADR-006, which previously said the device had no irrigation intelligence.
- **[ADR-016](docs/adr/016-plant-binding-and-policy-model.md)** — configuration
  is per plant: bindings, roles, per-measurement thresholds. `PlantProfile` is now
  only a template. The actuator is **optional**.
- **[ADR-017](docs/adr/017-extensible-measurement-model.md)** — one batched
  telemetry topic carrying typed `MeasurementKind` samples, and a narrow
  typed-kind `measurements` table.
- **[ADR-018](docs/adr/018-battery-and-deep-sleep-device-mode.md)** — a battery
  device sleeps between samples and is **not offline**. Sleep is announced and
  bounded by an Edge-computed wake window; a command for a sleeping device is
  held as an Edge-side **intent** and minted at the next wake. Nothing on the
  wire changed. See §12.

---

## 4. Repository layout

```text
README.md              what the project is
ROADMAP.md             milestones, conventions, definition of done
CLAUDE.md              this file
.gitattributes         LF everywhere (see §8)
.env.example           secret template; .env is gitignored

docs/
├── README.md          documentation index — start here to navigate
├── Rhizo_Edge_*.md    historical source material (see §7)
├── architecture/      9 docs: overview, components, data flow, deployment,
│                      safety invariants, failure model, time, config, deps
├── adr/               ADR-001…018 — why each decision was made
├── prd/               PRD 000…140 — one per milestone, 17 fixed sections
├── protocol/          mqtt-v1.md (normative), http boundaries, versioning
├── testing/           strategy, 80 scenarios, simulator, HIL, local dev
├── hardware/          home-node-hardware-guide.md — BOM, enclosure, wiring,
│                      power, assembly order (practical, NOT normative)
└── issues/M0…M14/     256 implementation issues

tools/docscheck/       planning-artefact validator (Rust, no dependencies)

Cargo.toml             root workspace (M0-002)
crates/                ten implemented/building workspace crates:
                       mqtt-contract, policy, domain, storage, telemetry,
                       cloud-client, testkit, edge-controller, device-simulator,
                       cloud-api
migrations/edge/       0001_initial.sql (the M5 pre-release baseline) and
                       0002_irrigation_control.sql (M6: command intents, the
                       pre-dose baselines, the lockout audit fields)
crates/domain/data/    presets.v1.json — the embedded species catalogue,
                       compiled in with include_str! (M5-017). Twenty-two
                       curated entries; every value carries its provenance,
                       and nothing here names a device, sensor, or schedule.
deploy/ scripts/ test/                        compose, helper scripts, fixtures
```

`firmware/esp32-node` (M9) and `ui/rhizo-ui` (M12) will be **separate
workspaces**, excluded from the root, so `cargo test --workspace` never attempts
a cross build. See ADR-001.

Each crate's responsibilities and its explicit prohibitions are in
[docs/architecture/component-model.md](docs/architecture/component-model.md) —
read the relevant section before adding code to a crate.

`docs/hardware/home-node-hardware-guide.md` is the odd one out: a procurement and
assembly guide with parts, prices, and ratings, useful from M9 onward and
irrelevant before it. **Nothing in it is normative.** Its numbers are starting
points its own §20 lists as needing measurement, so never derive a constant, a
threshold, or a firmware default from it — required behaviour lives in the ADRs,
PRDs, safety invariants, and the MQTT contract. It names boards; ADR-007 governs
what that means in code.

---

## 5. Hard constraints — do not violate without an ADR

- **Rust only.** No Go, no Node.js, no TypeScript. The Tauri UI uses the
  Cargo/Trunk workflow specifically to avoid `npm`. A `package.json` appearing
  anywhere is a defect.
- **MSRV is Rust 1.98.0**; `rust-toolchain.toml` currently pins 1.98.0. The pin
  may move to a newer stable as a deliberate change, but **no change may silently
  raise the MSRV**, and nothing goes below 1.98.0. The firmware workspace may pin
  a different ESP-compatible toolchain (ADR-007); the host workspace is never
  downgraded to match it.
- **`rhizo-domain` is pure.** No I/O, no `Utc::now()`. Clippy enforces the clock
  ban from M1-013 onward.
- **`rhizo-mqtt-contract` and `rhizo-policy` are `no_std`.** They are the two
  firmware-facing shared crates. A `std`-only dependency in either breaks the
  ESP32 build invisibly; verify both bare-metal targets in the project gate.
- **One actuation gate.** `validate_water_command` lives in the contract crate;
  the simulator and the firmware each call it from exactly one place. A second
  implementation of the rules makes every simulator-based safety test worthless.
- **One offline evaluator.** `rhizo_policy::evaluate_offline` is shared the same
  way, for the same reason, and is also called from exactly one place per
  consumer. It is pure and takes elapsed time as a parameter — it cannot read a
  clock.
- **Offline autonomy is opt-in and policy-driven.** No policy, an invalid policy,
  or a missing required measurement all mean **no actuation**. A device never
  invents a threshold.
- **The actuator is optional.** A plant with no `ActuatorBinding` is a normal,
  fully supported monitoring plant, not a degraded one. `POST /water` on it
  returns 422, not 409.
- **The UI has no MQTT dependency**, so `UI → MQTT pump command` does not
  compile. Keep it that way.
- **No override / force / bypass parameter** on any watering endpoint or UI
  control. From ADR-018 this extends to **no wake, expedite, or cancel control**
  for a sleeping device: the first two have no mechanism and the third is a
  deliberate open question.
- **Power is never a safety input.** Battery voltage, charge state, and solar
  availability are telemetry. They may raise an alert; they grant and refuse
  nothing, and none of them appears in `IrrigationInputs` (ADR-018 §7).
- **An announced sleep may only defer the offline indication, never suppress
  it.** A device past its Edge-computed wake window is `isolated`, not
  `sleeping` (SAFETY-021).
- **Persist before publish.** A command row is committed before the MQTT publish;
  a publish retry reuses the same `command_id` and never generates a new one.

---

## 6. Working discipline

Per issue:

```text
read the issue and its dependencies → implement → cargo fmt → clippy -D warnings
→ cargo test → run the issue's own Verification commands
→ update docs if behaviour changed → tick the acceptance criteria
```

Project gate (grows as milestones land):

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo build -p rhizo-mqtt-contract --no-default-features --target thumbv7em-none-eabi
cargo build -p rhizo-policy --no-default-features --target thumbv7em-none-eabi
docker compose -f deploy/docker-compose.yml config
cargo run -p rhizo-docscheck
```

`cargo test safety_` is the project's answer to "are the invariants still
enforced?". Run it reflexively after touching `rhizo-domain`.

**A milestone is `DONE` only when its exit criteria in ROADMAP.md §2 are
demonstrably met** — never on the basis of closed issues alone. Update the status
column, the safety-invariant registry, and this file in the same change.

Issue numbering is a valid execution order: within a milestone, every issue's
dependencies have lower numbers or belong to an earlier milestone. Executing
`M<n>-001…0<k>` in order is always safe. The dependency graph shows where that
order can be *widened* into parallel work.

---

## 7. Things that will surprise you

**Write files with LF.** Python's `Path.write_text` translates `\n` → `\r\n` on
Windows. `.gitattributes` pins `eol=lf`, so CRLF files produce
"CRLF will be replaced by LF" on every `git add`. If you script a bulk edit, pass
`newline="\n"` or write bytes. Verify with:

```bash
git ls-files --eol | awk '{print $1,$2}' | sort -u   # expect only: i/lf w/lf
```

**The validator is a workspace member.** M0-011 adopted it, so the command is
`cargo run -p rhizo-docscheck`. The old `--manifest-path` form still resolves
and still works; the many issue files that quote it are not wrong, just older.
It stays dependency-free on purpose — it runs first in CI, with no network and
no registry access, before anything else compiles.

**The issue generators were deleted deliberately.** The 204 issue files were
produced by one-shot Python scripts that were then removed, because re-running
them would clobber corrections applied afterwards (four issue-ID swaps and
several reference fixes). **Never regenerate issues in bulk.** Edit them
individually.

**`docs/Rhizo_Edge_*_Prompt.md` and `PROJECT_PLAN.md` are historical inputs.**
The project plan has been normalised (no Go, no Arduino, Tauri UI), but the two
prompt files contain illustrative examples like `M1-003 → M1-004 → M2-002` that
are **not** real references. `docscheck` excludes them for that reason. Do not
"fix" them.

**Two things named `.gitkeep` and `README.md` exist at several levels.** The
documentation index is `docs/README.md`; the project overview is the root
`README.md`. They are different documents.

**Docker-based verification may need a Linux environment.** Compose, the
integration tests that need a broker, and the M8 scenario suite are all
exercised on Linux in CI. How a given developer reaches a Docker daemon is a
machine detail — keep it in your own untracked `CLAUDE.local.md`, not here.

**A denied MQTT publish still exits 0 under MQTT 3.1.1.** The broker ACKs and
discards. Only MQTT v5 carries reason code 0x87 back to the client, which is
why `scripts/verify-mosquitto-acls.sh` passes `-V 5` and asserts on output
rather than exit status. Any future ACL test must do the same or it will pass
unconditionally.

**The device subscribes to eight *exact* topics, never a wildcard.** An earlier
revision of protocol §3 specified `commands/+`, which also matches
`commands/result` — the device's own output — and MQTT 3.1.1 has no "no local"
option. The rule had to be "receive it but never act on it", which is a property
of the dispatch code rather than of the wire. It is now
`Topic::device_subscriptions` returning `[String; 8]`, and
`Topic::device_command_filter` is gone. The cost: **adding a command kind means
adding a subscription**, in the same change as the topic itself.

It was seven until the post-M6 correction added `commands/result/ack` (§13).
That topic is a *child* of `commands/result` and a distinct exact topic from it,
so subscribing to the acknowledgement still does not deliver the device its own
results — which is the whole property the exact form exists to guarantee, and is
asserted rather than assumed.

**A `history.gap` marker is sealed when it is first sent.** It accumulates
mutably while unsent — range widened, count raised — and
`EventBufferState::seal_gap` turns it into an immutable event immediately before
each replay, allocating its `device_seq` at that moment. Later losses open a new
marker. Both halves are load-bearing: the edge deduplicates on `event_id`, so a
marker that grew after publication would be discarded as a duplicate of the
smaller first version; and a sequence allocated at the first loss would sit below
events buffered afterwards, where a cumulative `event.ack` would bury a marker the
edge had never seen. `replay_events()` therefore does **not** include a pending
gap — a test that inspects the buffer directly must seal first, or reconnect.

**A QoS 1 PUBACK is hop by hop and proves nothing about the edge.** This is the
single most reusable fact in this repository, and both `event.ack` and
`command.result.ack` exist because of it. A device's PUBACK is written by the
*broker*, on receipt; the edge may not have read the message, may crash before
its transaction commits, and — the edge session is clean on purpose — will never
be offered it again. `set_manual_acks(true)` in `mqtt::ingress` makes the edge's
own PUBACK follow its commit, which is worth having and governs the
**broker-to-edge hop only**. An earlier comment there claimed it made a device's
retry depend on the edge's commit; it never could. Anything that must survive
that gap needs an application-level acknowledgement.

**A `command.result` is retained on the device until `command.result.ack` names
it** (protocol §5.14), not until the broker acks the publish, and it is
republished every `COMMAND_RESULT_RETRY_MS` (15 s) while it is unacknowledged —
on a timer, not only on reconnect, because the failure it covers is an edge that
crashes and restarts while the device's socket never drops. The edge publishes
the acknowledgement **after** its commit and **also for a duplicate result**: a
duplicate is a device retrying because the first acknowledgement was lost, so
silence would leave it retrying for ever. Per `command_id`, not cumulative —
results carry no `device_seq`-style total order. Do not "fix" a related problem
with `clean_session=false`: that moves durability into the broker, which holds
no application state and is explicitly replaceable.

**`PENDING_RESULT_LIMIT = 32` is a simulator constant, not a specification.**
The simulator bounds its unacknowledged-result ledger and evicts the oldest,
which is fine on a host: no flash-endurance limit, autonomous doses carry the
same volumes through a second path as `watering.offline_autonomous` audit
events, and its job is to exercise the protocol rather than keep a plant alive.
**None of that holds on an ESP32**, so M9 must decide the firmware's saturation
behaviour on its own terms — fail closed, never silently under-count delivered
water. The requirement is [ADR-014](docs/adr/014-failure-and-retry-policy.md)
§Device-side pending-result ledger and PRD 090 F-090-17…19; M9-011 decides,
M9-022 verifies. M9-017's event ring is *not* the precedent: a `history.gap`
reports a lost **record**, which the edge can see, while a dropped result removes
a **quantity the edge's budget is derived from**, which it cannot.

**A `command.result` is ledger data; a telemetry sample is not.** A lost sample
is fail-safe — it makes data look older, and stale data blocks watering — so
telemetry gets no acknowledgement and no retry, deliberately. A lost
`completed` result *under-counts* the SAFETY-006 budget, which is the direction
that waters again too soon. Do not generalise the result-durability machinery
to telemetry.

**A replayed `watering.offline_autonomous` names its own plant.**
`detail.plant_id` is the plant whose `OfflinePolicy` the device evaluated, and
`persist_replay` writes it onto the `watering_events` row in the same
transaction as the event. Binding-based attribution still exists but is now only
the **fallback**, for a device that predates the field or names a plant this
edge has never provisioned — the latter falls back rather than failing, because
`watering_events.plant_id` is a foreign key and a rejected replay wedges
reconciliation for ever. Resolving the plant from `actuator_bindings` at replay
time asks a question about the present and applies the answer to the past: move
a pump while a device is isolated and the dose lands in the wrong budget, which
leaves the plant that *was* watered free to be watered again.

**An `event.ack` beyond the highest sequence a device issued is refused whole,
not clamped.** Clamping would turn one corrupt field into "delete the entire
buffer". Same shape for a mismatched `boot_id`: ignored, not best-effort.

**`connectivity` is stored *and* derived, and the derived answer is the one that
counts.** `devices.connectivity_mode` exists so the liveness timer has somewhere
to record the transition it makes, the event it emits, and the counter it
increments. What `GET /api/v1/devices/{id}` reports re-checks `overdue_at`
against the edge clock on every read, so an overdue sleeper is `isolated` even
if the timer is stopped, wedged, or has not run since the process started. Read
the column back directly and you will see `sleeping` for a device that is
reported — correctly — as `isolated`; that is not a bug, it is the point. Use
`connectivity::from_projection`, never the raw column.

**There are two staleness formulas and picking the wrong one breaks SAFETY-005.**
`max_sample_age_seconds` is the control-freshness threshold: it takes the
telemetry cadence and nothing else, and it is what M6-005 must call.
`liveness_interval_seconds` is the connectivity cadence, is widened by a battery
device's declared `wake_interval_seconds`, and is capped at 3600 s for that
reason. `wake_interval_seconds` is device-declared and admits values up to
86 400, so letting it reach the control threshold would let a device advertise
itself a three-day freshness window. A battery device beyond the cap reads
`stale` while its connectivity reads `sleeping`; the two answer different
questions and are allowed to disagree.

**An absent `power` block and an unknown `power.mode` are not the same thing.**
Absence is what a v1 status written before ADR-018 carries: it declares nothing
and changes nothing the edge already knew. An unrecognised mode is an explicit
declaration that resolves to always-on and *retires* any battery state the device
had. And a Last Will declares neither — it is composed at connect and delivered
whenever the session drops, so it is evidence of an absence and never a
restatement of configuration.

**A sleeping device is not an offline device, and an offline device is never
shown as sleeping.** A battery device announces sleep with a retained
`status: "offline", reason: "sleeping"` and the Edge opens a wake window computed
from **its own** `received_at`. Past `overdue_at` the device is `isolated`. The
device's own `expected_wake_ms` is advisory and never extends the window; a Last
Will and an unrecognised `reason` both mean `isolated`. This is SAFETY-021, and
it is what stops the new state becoming a place where dead devices hide.

**A command for a sleeping device is an *intent*, not a command.** No
`command_id` exists until the device is awake, which is why persist-before-publish
and same-`command_id`-retry did not have to change, and why `commands` gains no
column. The gate re-runs in full at delivery, so this path is *stricter* than the
immediate one. Command TTL stayed at 120 s and needed no change — the latency
lives in `intent_expires_at`, which never reaches a device.

**Deep sleep is not a reboot, but only when it can prove it.** A timer wake with
a valid RTC-memory checksum credits the RTC counter's measured elapsed time;
every other reset reason, and any checksum failure, credits zero — which is
SAFETY-015's existing behaviour. Get this backwards and a corrupted RTC word
becomes free watering budget.

**`auto_watering_enabled` defaults to `false`.** If a plant never waters in a
test, check that first — it is intended behaviour, not a bug. The default is
enforced in `repo::plant::create`, not only in the API, so no caller can forget
it.

**Deleting a plant is a soft delete, and a profile in use can never be deleted.**
`plants.deleted_at` is what keeps `watering_events` and their attribution alive,
so every plant read filters on it — a plant that "vanished" is one you deleted.
`count_using_profile` counts **all** plant rows including soft-deleted ones,
because the foreign key does not know about `deleted_at`: reporting a profile as
free and then failing its delete with a constraint error would be worse than
refusing it plainly.

**The dry-duration accumulator folds every unobserved sample, not one per tick.**
`plant::analyse` reads the samples newer than `last_sample_at` and folds them in
order. Advancing one reading per *tick* would make the debounce a property of how
often the loop runs — a 30-second tick against a 5-minute cadence over-counts,
and a slow tick under-counts. It also means the freshness threshold has to be
larger than the sampling interval or every sample looks like a gap: at the
default 300-second cadence the threshold is 900 seconds, and a test that samples
every 30 minutes will see `dry_ms` stay at zero. That is the gap rule working,
not a bug.

**A sleeping simulator disconnects *cleanly*.** `Connection::disconnect_cleanly`
leaves without publishing anything, because a dropped socket makes the broker
publish the will — which overwrites the retained `sleeping` status with
`connection_lost` and turns every expected absence into an unexplained one. The
`sleep-without-announcing` fault is the one path that still drops the socket, and
that is its entire purpose. This was caught by the real-broker test, not the
in-process one.

**`missed_wake_count` reaches 1 after two missed wakes, not 2.** A missed wake
announces nothing, so it opens no new window, and M4's timer counts at most one
miss per window. `isolated` — the half that matters — is unaffected. See
[docs/reports/M5.md](docs/reports/M5.md) §Deviations before "fixing" it.

**`POST /plants/{id}/water` answers 422, 409, or 202, and never 501.** A plant
with no `ActuatorBinding` gets **422** `no_actuator_bound`, which SAFETY-018
requires to be distinguishable from a 409 safety refusal and from a 404. M6-016
replaced M5's 501 arm: a plant that does have one now runs the gate and answers
**409** with `{ reason, since, clearable, message }` or **202** with a
`command_id` — or, for a sleeping device, **202** with an `intent_id` and no
`command_id` at all.

**Battery measurement kinds are recognised, scalar, and deliberately not
control-eligible.** `MeasurementKind::is_power_telemetry` is what excludes them,
so a policy naming `battery_voltage` as its control measurement is refused by the
shared validator rather than by a reviewer noticing (ADR-018 §7).

**A preset kind with no matching binding produces no policy row.** Creating a
plant with `preset_id` before binding any sensor is therefore legal and configures
nothing — the kinds come back in `skipped_unbound_kinds`. Applying the preset
again after binding sensors is how they get configured, and needs `overwrite`
only if policies already exist.

**The simulator now waters by itself while isolated, and there is exactly one
call site.** M6-019 added `rhizo_policy::evaluate_offline` and the simulator's
single call in `src/offline.rs`. `tests/single_actuation_path.rs` asserts *one*
call site where it previously asserted zero, and still fails if a second
implementation of the offline rules appears. An autonomous dose reaches the pump
through the same `begin_dose` a command does; what it skips is
`validate_water_command` steps 2 and 3 — `clock_unsynced` and `expired` — because
those are properties of a command another machine issued at a wall-clock instant,
and an isolated device has neither. The volume ceilings still apply, from the
shared `bound_dose`.

**`evaluate_offline`'s `elapsed` is a delta, not a since-boot instant.** It is
what §5b's `credit_elapsed` produces, and it has to be: the cooldown is stored as
a *remaining duration*, so there is no instant to subtract it from. A reboot
credits zero, and zero advances nothing. offline-autonomy.md §4's old "since
boot" comment was corrected in the same change.

**Two functions, not one, on both evaluators.** `evaluate` answers *what to do*
and `next_state` answers *where that leaves the plant*; `evaluate_offline` and
`next_offline_state` are the same split. The caller persists the second in the
same transaction as the first's side effect, which is what F-060-14 asks for.

**The reconciliation hold is derived, not stored.** A plant is held while
`replay_progress` shows no committed contiguous prefix for the device's *current*
boot. It is not a column on `devices`, because `persist_status` rewrites
`connectivity_mode` on every heartbeat and a device replays while it is
heartbeating. An **empty** complete replay releases the plant — a device that was
never isolated has nothing to reconcile — while a *suffix with no prefix* keeps
the hold. Conflating the two froze every ordinary reconnection, and the real
simulator found it.

**`manual` water is outside the automatic rolling cap.** M6-007's query is
`mode IN ('automatic','recommended')`, so a manual dose does not spend the
budget — though it still resets the cooldown, and the device's own
`FIRMWARE_MAX_DAILY_ML` still bounds it. M5's `delivered_since` counted every
mode but `detected`; that is not the M6 rule.

**`IrrigationInputs` has six fields PRD 060's illustrative struct does not**, and
each is required by a normative requirement the struct predates: `dry_duration`,
`pre_dose_weight`, `required_inputs`, `active_lockout`, `lockout_held_until`, and
`reconciling`. The tuning constants arrive through `AutomationPolicy` rather than
a `profile` field, because a machine that read a profile at evaluation time would
make editing a template silently rewrite twelve plants' rules (ADR-016).

**A held dose is an intent, and `commands` gained no column.** The reviewer's
check that M6-022 was implemented correctly is that `command_intents.command_id`
is nullable. `intent_expires_at` is the edge's clock and never reaches a device;
the wire TTL is unchanged at 120 s.

**The PUBACK now follows the commit.** `ingress::options` sets
`set_manual_acks(true)` and the pipeline acknowledges on the success arm of
`process`. Do not "simplify" that back to automatic acknowledgement: it is the
M3 gap M6-010 was required to close, and a lost `command.result` under-counts the
SAFETY-006 budget.

**Two documented greps in the M2 issues count documentation, not code.**
`grep -c validate_water_command` matches a `use` statement and five doc
comments; `grep 'evaluate_offline'` matches six comments explaining the M2/M6
boundary. The call-expression forms are what the criteria mean, and
`tests/single_actuation_path.rs` checks them as tests. Both issues record the
correction.

**A device with no leak sensor cannot water at all.** The shared gate refuses
`LeakState::Unknown` (protocol §5.8 step 6), so `--sensors soil,tank` produces a
device that refuses every dose with `leak_unknown`. Fail-closed by design, and
the first thing to check when a simulated dose is refused.

**A protocol fixture's directory is part of its assertion.** Under
`test/fixtures/protocol/invalid/`, the directory name *is* the expected typed
failure (`policy_dose_above_hard_limit/`, `event_duplicate_id/`, …) and maps to a
case of `Expected` in `crates/mqtt-contract/tests/fixtures.rs`. A directory the
harness does not recognise fails the suite rather than being skipped, so a new
failure class costs one match arm. Valid fixtures are decoded as their **concrete
payload type**, never as `serde_json::Value` — the whole point is that renaming a
payload field turns the suite red. Run them with
`cargo test -p rhizo-mqtt-contract --test fixtures`; a bare `fixtures::` filter
matches nothing in an integration-test binary and passes vacuously.

---

## 8. Identifier conventions

| Kind | Form | Example |
|---|---|---|
| Milestone | `M<n>` | `M6` |
| Issue | `M<n>-NNN` | `M6-009` |
| ADR | `ADR-NNN` | `ADR-006` |
| PRD | `PRD NNN` | `PRD 060` |
| Safety invariant | `SAFETY-NNN` | `SAFETY-006` |
| Test scenario | `SCEN-NNN` | `SCEN-040` |
| Functional requirement | `F-NNN-NN` | `F-060-20` |

Safety tests are named `safety_NNN_<description>`. Every reference of these
forms is checked by `docscheck` — if you invent an ID, the validator will catch
it, so run it before finishing.

---

## 9. Safety invariants — the short version

Twenty-one numbered invariants in
[docs/architecture/safety-invariants.md](docs/architecture/safety-invariants.md),
each with named tests and an enforcement milestone. Most become enforced in M6.

The ones most easily broken by an innocent-looking change:

- **SAFETY-005** — staleness must use the **edge** `received_at`, never the
  device timestamp, and its threshold must come from the telemetry cadence, never
  from a power field. A device with a wrong clock would otherwise make stale data
  look fresh; a device declaring an 86 400-second wake interval would otherwise
  make three-day-old data look actionable.
- **SAFETY-006** — the 24-hour cap is **rolling and derived from rows**, not a
  counter and not a calendar day. A counter would reset on restart; a calendar
  day permits two allowances around midnight.
- **SAFETY-012** — `None` and `Unknown` map to a lockout, never to permission.
  No `unwrap_or_default()` on a safety input, no `_ =>` arm on a safety match.
- **SAFETY-001 / -010** — the dedup marker and the message's effects share one
  SQLite transaction. Splitting them reintroduces duplicate watering on crash.
- **SAFETY-021** — a device is `sleeping` only inside a window the **edge**
  computed, and `isolated` the moment it is overdue. Trusting the device's own
  `expected_wake_ms`, letting an unrecognised offline `reason` mean "asleep", or
  reporting the stored `connectivity_mode` without re-checking `overdue_at`, all
  turn the sleep state into a place where dead devices hide. The last of those is
  the subtle one: it makes the invariant depend on a writer, and a writer that
  dies leaves a device asleep for ever.

If you find yourself weakening one of these to make a test pass, the test is
probably right.

---

## 10. When something is genuinely undecided

Open questions are recorded in each PRD's "Open questions" section, and real
external risks in ROADMAP.md. The largest unresolved ones:

- Whether stock Rust 1.98.0 builds `riscv32imc-esp-espidf`, or whether the
  espup channel is required (resolved empirically by M9-001).
- ESP-IDF toolchain friction on Windows; documented fallback is building in a
  Linux container and flashing from the host.
- Accuracy of cheap capacitive probes, unknown until the gravimetric check in
  M10-010.

Everything else has been decided. If a choice feels open, search the ADRs before
re-opening it.

---

## 11. The 2026-08-26 architecture pass

Requirements expanded after M0 shipped and before M1 started. Three ADRs landed,
the MQTT v1 contract was revised in place, and 35 issues were added. If you
learned this project from documents written earlier, these are the corrections:

| Was | Is now |
|---|---|
| "Offline" meant the cloud was unreachable | Three modes: cloud offline, site offline, **device isolated** ([connectivity-modes.md](docs/architecture/connectivity-modes.md)) |
| The device has no irrigation intelligence | A device with a **validated persisted policy** may water while isolated ([ADR-015](docs/adr/015-device-offline-autonomy.md)) |
| One `PlantProfile` per plant | Bindings + roles + per-measurement thresholds; profile is a **template** ([ADR-016](docs/adr/016-plant-binding-and-policy-model.md)) |
| Every plant has a pump | The actuator is **optional**; monitoring-only is first-class (SAFETY-018) |
| Six hard-coded measurements, four telemetry topics | Typed `MeasurementKind` enum, **one batched telemetry topic** ([ADR-017](docs/adr/017-extensible-measurement-model.md)) |
| Wide `measurements` table | Narrow typed-kind table with `batch_id` and `origin` |
| 12 safety invariants | **20** — SAFETY-013…020 appended; the first twelve unchanged and never renumbered |
| Rust pinned at exactly 1.98.0 | **MSRV 1.98.0**; the pin may move forward deliberately |
| Devices sync SNTP from the internet | Devices take wall time from **the Edge over MQTT** (`edge.time`, never retained), so a site outage does not disable watering |

Two things did **not** change and are worth restating: M0 was not reopened, and
the cloud is still incapable of originating a command in any mode.

New crate: `rhizo-policy` (`no_std`, pure) holds the offline evaluator and is the
**second** crate shared with the firmware. `mqtt-contract ← policy ← domain`.

---

## 12. The 2026-08-28 battery and deep-sleep pass

Planning only; no runtime code was written. A battery-powered Wi-Fi node that
sleeps between samples became a supported deployment
([ADR-018](docs/adr/018-battery-and-deep-sleep-device-mode.md)). **14 issues were
added** and each affected milestone's verification issue was renumbered to stay
last — M5-022, M6-024, M8-018, M9-022, M10-013, M12-019, M13-017, M14-010.

| Was | Is now |
|---|---|
| A device is either connected or offline | Four reachability states: `connected`, `sleeping { expected_wake_at }`, `isolated`, `reconciling` |
| Silence means something is wrong | Silence inside an **announced, Edge-computed** window is normal; silence past it is `isolated` (SAFETY-021) |
| A command is persisted then published immediately | For a sleeping device: an **intent** is persisted and the command is minted at the next wake |
| Battery operation was an M14 topic | Real work in M5, M6, M9, M10, M12, M13; M14 keeps only solar and outdoor power |
| PRD 140 listed five "high" connectivity breakages | Four were the *sleep* problem and are resolved in v1; only the *radio* problem remains |
| 20 safety invariants | **21** — SAFETY-021 appended; the first twenty unchanged and never renumbered |
| 242 issues | **256** |

Three things did **not** change, and it is worth knowing why before you touch
this area:

- **The protocol.** Two `MeasurementKind` variants, optional `power` blocks, one
  offline `reason`. All additive within v1; no version bump, no new topic, no
  retention change, and the device still subscribed to exactly seven exact
  topics *at that date* — the post-M6 correction later took it to eight (§13).
  Holding a command is an Edge-side mechanism with no wire representation.
- **Command TTL and `edge.time`.** Unchanged, because the command is minted at
  the wake. SAFETY-002 is untouched.
- **M4.** Its completed report carries a dated battery-compatibility correction
  and a dated independent review of that correction. The registry model is no
  longer deferred to **M5-020**, which is wholly superseded, and **SAFETY-021 is
  enforced in M4**, not M5. M4 was not reopened as a milestone — the same
  treatment M0 got in the 2026-08-26 pass — because M5 is the first milestone
  still open. That is why M5, a plant milestone, contains device issues: M5-019's
  remaining contract scope and M5-021's simulator.

---

## 13. The 2026-08-31 post-M6 correction

A focused pass over two guarantees M6 **claimed** but did not hold, plus the
evidence that had been offered for them. M6 was not reopened as a milestone —
the treatment M0 got in the 2026-08-26 pass and M4 got in the battery pass —
because M7 had not started. Full write-up in
[docs/reports/M6.md](docs/reports/M6.md) §Post-M6 corrections.

| Was | Is now |
|---|---|
| Manual PUBACKs made a device's retry depend on the edge's commit | They govern the **broker-to-edge hop only**; QoS 1 is hop by hop, and a `command.result` could be lost by an edge that crashed before committing |
| A result is retired when the broker acks the publish | Retired only by `command.result.ack` (protocol §5.14), retried every 15 s until then, and persisted across reboot |
| A replayed autonomous dose was attributed from current `actuator_bindings` | It carries `detail.plant_id`, written in the same transaction as the event; bindings are the **fallback** |
| 7 exact device subscriptions | **8** — `commands/result/ack` added |
| "1 092 passed" quoted as the headline evidence | A bare total is *identical* with the broker stopped: 46 tests skip silently and count as passed. Quote the environment and per-suite counts |
| `edge-controller/integration` reported 20/20 | It was 18/20 against a broker carrying one leftover retained status; four assertions read `devices` without a `WHERE` clause. Scoped, and verified **against** the pollution rather than its absence |
| `api::health`'s two readiness tests passed | They raced on `metrics.connection`, which is **process-global** (`Metrics::new` caches in a `OnceLock`), failing about one full run in three. Serialised on a mutex — a per-test registry would test something the binary does not do |

Three things did **not** change:

- **Telemetry loss semantics.** A lost sample is fail-safe and still gets no
  acknowledgement and no retry. The durability machinery is for ledger data only.
- **`clean_session = true` on the edge.** Deliberately not the fix: it would move
  durability into the broker, which is the one participant holding no application
  state. `mqtt::ingress::options` now says so.
- **Every existing payload, retention rule, QoS, command TTL, and `edge.time`.**
  Both changes are additive within v1, and a device implementing the old rules
  still interoperates.
