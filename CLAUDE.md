# CLAUDE.md — Rhizo Edge

Working notes for Claude Code sessions on this repository.

---

## 1. Where the project is right now

**Planning is complete. M0 and M1 are implemented and green. M2 has not started.**

| | State |
|---|---|
| Planning artefacts | ✅ complete; revised by the 2026-08-26 architecture pass |
| M0 milestone | ✅ **DONE** — implemented, verified, committed (`8fba4e7`) |
| Architecture pass | ✅ done — offline autonomy, per-plant policy, extensible measurements (see §11) |
| M2-001 | ⬜ **next** — create the device-simulator binary skeleton |
| M1 | 19 issues, **DONE** |

**This section goes stale fastest. Verify it before trusting it:**

```bash
git log --oneline -3
git status --short
ls Cargo.toml rust-toolchain.toml clippy.toml rustfmt.toml
cargo build --workspace
```

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
(`docs/adr/`). Seventeen of them, each with Context / Decision / Alternatives /
Consequences / Risks. Read the relevant one rather than re-deciding.

The three newest are the ones most likely to surprise you if you learned this
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
├── adr/               ADR-001…014 — why each decision was made
├── prd/               PRD 000…140 — one per milestone, 17 fixed sections
├── protocol/          mqtt-v1.md (normative), http boundaries, versioning
├── testing/           strategy, 72 scenarios, simulator, HIL, local dev
└── issues/M0…M14/     204 implementation issues

tools/docscheck/       planning-artefact validator (Rust, no dependencies)

Cargo.toml             root workspace (M0-002)
crates/                nine crate stubs, empty but building:
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
- **`rhizo-mqtt-contract` is `no_std`.** It is the only crate shared with the
  firmware. A `std`-only dependency there breaks the ESP32 build invisibly.
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
  control.
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

**`auto_watering_enabled` defaults to `false`.** If a plant never waters in a
test, check that first — it is intended behaviour, not a bug.

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

Twelve numbered invariants in
[docs/architecture/safety-invariants.md](docs/architecture/safety-invariants.md),
each with named tests and an enforcement milestone. Most become enforced in M6.

The ones most easily broken by an innocent-looking change:

- **SAFETY-005** — staleness must use the **edge** `received_at`, never the
  device timestamp. A device with a wrong clock would otherwise make stale data
  look fresh.
- **SAFETY-006** — the 24-hour cap is **rolling and derived from rows**, not a
  counter and not a calendar day. A counter would reset on restart; a calendar
  day permits two allowances around midnight.
- **SAFETY-012** — `None` and `Unknown` map to a lockout, never to permission.
  No `unwrap_or_default()` on a safety input, no `_ =>` arm on a safety match.
- **SAFETY-001 / -010** — the dedup marker and the message's effects share one
  SQLite transaction. Splitting them reintroduces duplicate watering on crash.

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
