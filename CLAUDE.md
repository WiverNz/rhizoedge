# CLAUDE.md — Rhizo Edge

Working notes for Claude Code sessions on this repository.

---

## 1. Where the project is right now

**Planning is complete. M0 through M4 are implemented and green. M5 is READY and
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
| M5-001 | ⬜ **next** — add plant and profile repositories |

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
crates/                nine implemented/building workspace crates:
                       mqtt-contract, domain, storage, telemetry, cloud-client,
                       testkit, edge-controller, device-simulator, cloud-api
deploy/ migrations/ scripts/ test/            skeleton, .gitkeep only
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

**The device subscribes to seven *exact* topics, never a wildcard.** An earlier
revision of protocol §3 specified `commands/+`, which also matches
`commands/result` — the device's own output — and MQTT 3.1.1 has no "no local"
option. The rule had to be "receive it but never act on it", which is a property
of the dispatch code rather than of the wire. It is now
`Topic::device_subscriptions` returning `[String; 7]`, and
`Topic::device_command_filter` is gone. The cost: **adding a command kind means
adding a subscription**, in the same change as the topic itself.

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
test, check that first — it is intended behaviour, not a bug.

**The simulator never waters by itself, and that is M2's boundary, not a bug.**
An enabled, valid, activated offline policy on a bone-dry isolated plant is
completely inert until M6-019 adds the one shared
`rhizo_policy::evaluate_offline` and the simulator's single call site.
`tests/single_actuation_path.rs` fails if an evaluator, a decision type, or a
dose scheduler appears in `crates/device-simulator/src` before then.

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
  retention change, and the device still subscribes to exactly seven exact
  topics. Holding a command is an Edge-side mechanism with no wire representation.
- **Command TTL and `edge.time`.** Unchanged, because the command is minted at
  the wake. SAFETY-002 is untouched.
- **M4.** Its completed report carries a dated battery-compatibility correction
  and a dated independent review of that correction. The registry model is no
  longer deferred to **M5-020**, which is wholly superseded, and **SAFETY-021 is
  enforced in M4**, not M5. M4 was not reopened as a milestone — the same
  treatment M0 got in the 2026-08-26 pass — because M5 is the first milestone
  still open. That is why M5, a plant milestone, contains device issues: M5-019's
  remaining contract scope and M5-021's simulator.
