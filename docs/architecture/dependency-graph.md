# Dependency Graph

Execution order for Rhizo Edge. The purpose of this document is narrow and
practical: **an implementation session should be able to determine which issue
is safe to execute next without rediscovering the architecture.**

Authoritative sources, in order of precedence:

1. Each issue's `Dependencies` section — normative.
2. This document — the readable summary, generated from and verified against
   those sections.
3. [ROADMAP.md](../../ROADMAP.md) — milestone-level view.

If this document and an issue disagree, the issue wins and this document is a
bug.

---

## 1. Milestone dependency graph

```text
                            M0  Foundation
                             │
                             ▼
                            M1  Domain + MQTT contract
                             │
                  ┌──────────┴──────────┐
                  ▼                     │
                 M2  Simulator          │
                  │                     │
                  └──────────┬──────────┘
                             ▼
                            M3  Ingestion + SQLite
                             ▼
                            M4  Registry + Health
                             ▼
                            M5  Plant model + Recommendations
                             ▼
                            M6  Irrigation + Safety
                             │
              ┌──────────────┴───────────────┐
              ▼                              ▼
             M7  Cloud + PostgreSQL         M12 Rust UI
              ▼                              │
             M8  End-to-end environment      │
              ▼                              │
             M9  ESP32 firmware              │
              ▼                              │
             M10 Real soil sensor            │
              ▼                              │
             M11 Real pump + hardware ───────┤
                                             ▼
                                            M13 Multi-plant home
                                             ▼
                                            M14 Field readiness (docs only)
```

### Reading the two branches after M6

**M12 depends on M6, not the reverse.** The UI is a Tauri desktop application
([ADR-009](../adr/009-ui-architecture-and-rust-web-stack.md)) and is deliberately
**not** part of the M8 software-only acceptance environment. M8 must stay
headless and CI-runnable.

M12's first issue (`M12-001`) depends on `M6-022`, so the UI becomes buildable
as soon as irrigation control exists. In practice it is scheduled after M11 so
the operator sees the full picture including real hardware, but nothing forces
that order. M13 requires both branches.

### Milestone prerequisites in full

| Milestone | Requires | Because |
|---|---|---|
| M0 | — | root of the graph |
| M1 | M0 | needs the workspace, toolchain, and lint policy |
| M2 | M1 | implements the protocol and calls the shared validator |
| M3 | M1, M2 | needs the contract types and a device producing traffic |
| M4 | M3 | extends the ingestion pipeline and storage |
| M5 | M4 | needs staleness and sensor health as inputs |
| M6 | M5, M2 | needs plant state; needs a device that refuses like firmware |
| M7 | M6 | needs watering events worth syncing |
| M8 | M7 | needs the last component of the topology |
| M9 | M8 | needs a proven software system to compare firmware against |
| M10 | M9 | needs the firmware trait boundaries |
| M11 | M10 | needs real soil readings to close the control loop |
| M12 | M6 | needs state and actions worth showing |
| M13 | M12, M11 | scales a working, hardware-verified, observable system |
| M14 | M13 | verifies reservations against a mature codebase |

---

## 2. Issue-level dependencies

Only dependencies that **materially affect execution order** are shown. Trivial
"the previous issue in the file" chains are omitted where they add no
information.

Notation: `A → B` means A must complete before B begins.
`A + B → C` means both are prerequisites.

### M0 — Foundation

```text
M0-001 (repo skeleton)
   ├──→ M0-002 (workspace) ──┬──→ M0-003 (toolchain 1.98.0 + lints) ──┐
   │                          ├──→ M0-004 (error conventions) ──┐      │
   │                          ├──→ M0-006 (telemetry)           │      │
   │                          ├──→ M0-007 (backoff)             │      │
   │                          └──→ M0-010 (testkit TestClock)   │      │
   │                                                             ▼      │
   │                          M0-004 + M0-002 ──→ M0-005 (edge config) │
   │                                                                    │
   ├──→ M0-008 (mosquitto auth + ACLs) ──→ M0-009 (compose) ───────────┤
   └──→ M0-011 (docs validator) ───────────────────────────────────────┤
                                                                        ▼
                    M0-003 + M0-009 + M0-011 ──→ M0-012 (CI workflow)
                                                        │
                                              all ──→ M0-013 (verification)
```

**Parallelisable after M0-002:** M0-003, M0-004, M0-006, M0-007, M0-010 are
independent of one another. M0-008 and M0-011 need only M0-001 and can start
immediately alongside M0-002.

**Critical path:** `M0-001 → M0-002 → M0-003 → M0-012 → M0-013`.

### M1 — Domain and MQTT contract

```text
M0-002 + M0-003 ──→ M1-001 (no_std crate skeleton)
   ├──→ M1-002 (DeviceId) ──┬──→ M1-005 (topic grammar)
   ├──→ M1-003 (time/ids)   │
   │                         ▼
   └── M1-002 + M1-003 ──→ M1-004 (envelope)
                              ├──→ M1-006 (telemetry payloads)
                              ├──→ M1-007 (status/config payloads)
                              └──→ M1-008 (command payloads)
                                        ▼
                                   M1-009 (validate_water_command)   ◄── the gate
   M1-006 + M1-007 + M1-008 ──→ M1-010 (fixture corpus)
   M1-001 + M0-012 ──────────→ M1-011 (no_std CI verification)
   M1-006 + M0-010 ──────────→ M1-012 (domain crate + Clock trait)
   M1-012 + M0-003 ──────────→ M1-013 (clippy clock ban)
                       all ──→ M1-019 (verification)
```

**The two issues everything downstream leans on:**

- **`M1-009`** — `validate_water_command`. The simulator (M2-008) and the
  firmware (M9-011) both call it and nothing else. If it is wrong, every
  simulator-based safety test in M6 is wrong too.
- **`M1-012`** — the domain crate and `Clock` trait. Every pure decision function
  from M5 and M6 depends on its purity constraint.

### M2 — Simulator

```text
M1-019 + M0-006 ──→ M2-001 (skeleton)
   ├──→ M2-002 (MQTT + LWT) ──┬──→ M2-003 (status/config) ──┐
   │                            └──→ M2-012 (ACL isolation)  │
   ├──→ M2-004 (soil model) ──→ M2-005 (weight/tank/EC) ─────┤
   ├──→ M2-007 (persistent state) ───────────────┐            │
   └──→ M2-009 (control API) ─────────┐          │            ▼
                                       │          │   M2-003 + M2-005 ──→ M2-006
                                       │          │                    (telemetry)
                                       │          │            │
                            M2-006 + M2-007 ──────┴────────────┴──→ M2-008
                                                              (command handling)
                                                                     │
                          M2-008 ──→ M2-010 (retention guards)  │
             M2-008 + M2-009 ──→ M2-013 (fault injection) ◄──────────┘
             M2-004 + M2-009 ──→ M2-014 (virtual time)
             M2-006 + M1-010 ──→ M2-011 (fixture drift check)
             M2-003 ──→ M2-015 (capabilities)
             M2-008 + M2-015 ──→ M2-016 (atomic policy store)
             M2-016 + M2-007 + M2-013 + M2-014 ──→ M2-017
                                      (isolation + offline runtime state; no evaluation)
             M2-017 ──→ M2-018 (bounded event buffer + replay mechanics)
                          all ──→ M2-019 (verification)
```

**`M2-008` is the milestone's centre of gravity.** It is the only actuation path
and must have exactly one call site of `validate_water_command`. It needs both
telemetry (M2-006, to observe effects) and persistent state (M2-007, for the
dedup ring and the interrupted-dose record).

### M3 — Ingestion and storage

```text
M0-013 + M1-019 ──→ M3-001 (edge binary + supervisor)
   ├──→ M3-002 (storage crate + pool) ──→ M3-003 (schema + migrations) ──→ M3-004
   │                    │                          │                     (.sqlx cache)
   │                    └────────┬─────────────────┘
   │                             ▼
   │                    M3-008 (dedup transaction)   ◄── SAFETY-001 / -010 mechanism
   │                             │
   └──→ M3-005 (MQTT ingress)    │
             │                   │
             ▼                   │
   M3-005 + M1-004 ──→ M3-006 (decode + received_at)
             │                   │
   M3-006 + M3-003 ──→ M3-007 (quarantine) ──→ M3-013 (classification, + M0-004)
                                 │
             M3-008 + M1-006 ──→ M3-009 (persist measurements + events)
                                      ├──→ M3-010 (latest-sample cache)
                                      ├──→ M3-011 (metrics, + M0-006) ──→ M3-012
                                      ├──→ M3-014 (graceful shutdown)      (cardinality)
                                      └──→ M3-015 (retention)
                                 all ──→ M3-018 (verification)
```

**`M3-008` is the safety-critical issue of this milestone.** The
deduplicate-and-persist transaction is the mechanism behind SAFETY-001 and
SAFETY-010; everything in M6 assumes it is correct.

**`M3-006` fixes `received_at` as authoritative.** Using device time here would
silently break SAFETY-005 three milestones later.

### M4 — Registry and health

```text
M3-018 ──→ M4-001 (apply ingested status to registry)
   ├──→ M4-002 (LWT)
   ├──→ M4-003 (auto-registration — device only, never a plant)
   ├──→ M4-004 (staleness + liveness timer)      ◄── SAFETY-005 input
   ├──→ M4-005 (sensor health)
   ├──→ M4-006 (config drift)
   └── M4-001 + M3-005 ──→ M4-007 (health endpoints)

   M4-004 + M4-005 + M4-006 ──→ M4-008 (device REST) ──→ M4-009 (API server + CORS)
   M4-004 + M3-011 ──────────→ M4-010 (device metrics)
                        all ──→ M4-013 (verification)
```

M4-002 through M4-007 are largely parallelisable once M4-001 lands.

### M5 — Plant model and recommendations

```text
M4-013 ──→ M5-001 (plant/profile repositories)
M1-012 ──→ M5-003 (profile validation)   ── independent of M5-001
M1-012 ──→ M5-005 (moisture trend)       ── independent of M5-001
   M5-001 + M4-009 ──→ M5-002 (plant endpoints)
   M5-001 + M5-003 ──→ M5-004 (profile endpoints)

M5-005 ──┬──→ M5-006 (dry duration) ──┐
         ├──→ M5-007 (manual watering detection)
         ├──→ M5-008 (stuck sensor)   │
         ├──→ M5-011 (EC trend)       │
         └── M5-005 + M5-006 ──→ M5-009 (recommendation engine)
                                    ├──→ M5-010 (plant state)
                                    └── M5-009 + M5-010 ──→ M5-012 (endpoints + tick)
                                                       all ──→ M5-017 (verification)
```

**M5-003 and M5-005 depend only on `M1-012`**, so they can be executed in
parallel with the repository and endpoint work. They are the pure-domain half of
the milestone.

### M6 — Irrigation and safety

```text
M5-017 ──→ M6-001 (IrrigationInputs / IrrigationDecision)
              ▼
           M6-002 (safety gate — exhaustive, no catch-all)
              ├──→ M6-003 (leak + tank checks)
              └──→ M6-004 (sensor validity) ── + M4-004 ──→ M6-005 (staleness + manual exception)
                                                                  │
                              M6-003 + M6-005 ──→ M6-006 (state machine)
                                                        │
                              M6-006 + M3-003 ──→ M6-007 (rolling 24h window)
                                                        ├──→ M6-015 (clock step)
                                                        ▼
                                                  M6-008 (command persistence)
                                                        ▼
                                                  M6-009 (publication)
                            ├──────────────────────────┼──────────────────────┐
                            ▼                          ▼                      ▼
                     M6-010 (results)          M6-011 (retry, + M0-007)  M6-013 (config
                       ├──→ M6-012 (restart              │               publication,
                       │    reconciliation)              │                + M4-006)
                       └──→ M6-017 (no delivery)         ▼
                                          M6-011 + M6-003 ──→ M6-016 (watering endpoints)
                     M6-006 + M3-011 ──→ M6-014 (control metrics)

        M6-017 + M6-012 + M6-015 ──→ M6-018 (safety property tests)
                                all ──→ M6-022 (verification)
```

**Execution order within M6 is nearly linear and should be respected.** The gate
(M6-002) precedes the machine (M6-006), which precedes the command lifecycle
(M6-008 → M6-009 → M6-010). Implementing the command path before the gate would
mean writing an actuation path that has to be retrofitted with safety, which is
the ordering this design exists to avoid.

**`M6-008 → M6-009` is normative and directional:** the command row is committed
*before* the publish. Reversing it permits a pump to run with no record.

### M7 — Cloud

```text
M6-022 ──→ M7-001 (cloud binary + postgres) ──→ M7-002 (schema) ──→ M7-003 (ingestion)
                                                                        ├──→ M7-004
                                                                        │  (projections)
                                                    M7-003 + M3-013 ──→ M7-005
                                                                     (cloud client)
   M7-005 + M0-007 ──→ M7-006 (outbox drain)
        ├──→ M7-007 (batch adaptation)
        ├──→ M7-008 (cap + value-tier pruning) ──→ M7-009 (metrics)
        │                                       └── + M6-022 ──→ M7-014 (event emission)
        └── M7-006 + M6-018 ──→ M7-010 (cloud independence differential test)

   M7-004 ──┬──→ M7-012 (reprojection)
            ├──→ M7-013 (read endpoints)
            └── + M7-005 ──→ M7-011 (time round-trip)
                       all ──→ M7-015 (verification)
```

**`M7-010` is the milestone's headline test** and depends on `M6-018`, because
the differential comparison needs the deterministic property-test infrastructure.

### M8 — End-to-end environment

```text
M7-015 ──→ M8-001 (Dockerfiles) ──→ M8-002 (compose topology) ──→ M8-003 (test overlay)
                                                                        ▼
                                                              M8-004 (time-scale check)
                                                                        ▼
                                                              M8-005 (scenario runner)
                                                                        ▼
                                                              M8-006 (baseline scenarios)
     ┌──────────────┬──────────────┬──────────────┬──────────────┬──────┘
     ▼              ▼              ▼              ▼              ▼
  M8-007         M8-008         M8-009         M8-010         M8-011
  (MQTT)        (device)      (lockouts)     (restart)       (cloud)
                                                                │
                              M8-006 + M8-011 ──→ M8-012 (first demo)
                                                     ├──→ M8-013 (mutation verification)
                                                     └──→ M8-014 (e2e CI job)
                                                 all ──→ M8-017 (verification)
```

**M8-007 through M8-011 are fully parallelisable** once the runner (M8-005) and
the baseline scenarios (M8-006) exist. This is the widest parallel section in
the whole plan.

### M9 — ESP32 firmware

```text
M8-017 ──→ M9-001 (verify + correct ADR-007 toolchain)   ◄── do this first, on real hardware
   ├──→ M9-002 (firmware CI job)
   └──→ M9-003 (workspace skeleton)
           ├──→ M9-004 (NVS + identity) ──┬──→ M9-008 (wifi) ──→ M9-009 (mqtt + lwt + time sync)
           │                               ├──→ M9-006 (serial provisioning)   │
           │                               │                                    ▼
           └──→ M9-005 (traits + fakes) ──→ M9-007 (boot-safe pump)      M9-010 (telemetry)
                                                    │                            │
                              M9-010 + M9-004 ──────┴──────────────────→ M9-011 (commands
                                                                          + dedup ring)
                                                            ├──→ M9-012 (config)
                                        M9-011 + M9-007 ──→ M9-013 (interrupted dose)
                                        M9-011 + M9-013 ──→ M9-014 (conformance test)
                                                       all ──→ M9-019 (verification)
```

**`M9-001` must genuinely be first.** It executes ADR-007's commands on a real
machine and corrects them. Every other issue in the milestone assumes a working
toolchain, and discovering it does not work halfway through is expensive.

**`M9-014`** is the conformance test that validates the entire simulator-first
strategy retrospectively.

### M10 — Real soil sensor

```text
M9-019 ──→ M10-001 (adapter selection)
   ├──→ M10-002 (Modbus RTU) ──→ M10-003 (register maps as data) ──→ M10-004 (modbus soil)
   └──→ M10-005 (analog soil)                                              │
                    └──────────────┬───────────────────────────────────────┘
                                   ▼
                            M10-006 (calibration — uncalibrated publishes null)
                                   ▼
                            M10-007 (error handling + health)
                                   ├──→ M10-008 (config schema, + M6-013)
                                   └──→ M10-009 (metrics + events) ──→ M10-010
                                                              (gravimetric validation)
                                                          all ──→ M10-011 (verification)
```

**M10-005 (analogue) depends only on M10-001**, so it can be done first as a
cheaper path to a working real sensor while the Modbus chain is built.

### M11 — Real pump and safety hardware

```text
M10-011 ──┬──→ M11-001 (pump driver) ──→ M11-002 (independent run guard) ──→ M11-003
          │           └──→ M11-004 (calibration command)                     (fault)
          ├──→ M11-005 (tank sensor) ──┐
          └──→ M11-006 (leak sensor) ──┴──→ M11-008 (hardware config schema)
                     M11-006 + M11-002 ──→ M11-007 (leak interrupt during dose)

  ── HIL gate sequence, each stage gating the next ──
  M11-002 + M11-008 ──→ M11-009 (HIL-1 boot safety, multimeter, no water)
                              ▼
              M11-009 + M11-004 ──→ M11-010 (HIL-3 calibration, measuring cup)
                              ▼
                        M11-011 (HIL-4 command safety, measure the cup)
                              ▼
              M11-011 + M11-007 ──→ M11-012 (HIL-5 lockouts)
                              ▼
                        M11-013 (HIL-6 full cycle)
                         all ──→ M11-014 (verification)
```

**The HIL chain M11-009 → M11-013 is strictly sequential and must not be
parallelised.** A stage failure sends you back to the bench, not forward with a
note. M11-009 involves no water at all; water enters the system only at M11-010.

### M12 — Rust UI

```text
M6-022 ──→ M12-001 (Tauri + Leptos workspace, no Node)
              ▼
           M12-002 (API client + shared DTOs, 409 → Refused)
     ┌────────┼────────┬────────┬────────┐
     ▼        ▼        ▼        ▼        ▼
  M12-003  M12-004  M12-005  M12-008  M12-009
 (overview) (plant)  (device) (profile) (events/sync)
     │        │
     │        ├──→ M12-006 (watering actions — no override control)
     │        └──→ M12-007 (charts)
     └──→ M12-010 (connection state) ──→ M12-011 (packaging) ──→ M12-012 (CI)
                                                          all ──→ M12-017 (verification)
```

M12-003 through M12-009 are largely parallelisable after M12-002.

### M13 — Multi-plant home

```text
M12-017 ──→ M13-001 (multi-device operation + cross-plant isolation)
   ├──→ M13-002 (provisioning tool)
   ├──→ M13-003 (reservoir entity) ──→ M13-004 (shared reservoir lockout)
   ├──→ M13-005 (grouping/filtering)
   ├──→ M13-006 (cross-device cap validation)
   ├──→ M13-007 (notifications) ──→ M13-008 (backup/restore) ──→ M13-009 (systemd)
   └──→ M13-010 (downsampling)

   M13-004 + M13-007 ──→ M13-011 (multi-device scenarios)
   M13-011 + M12-017 ──→ M13-012 (UI at scale)
                   all ──→ M13-016 (verification)
```

### M14 — Field readiness (documentation only)

```text
M13-016 ──→ M14-001 (verify reservations against code)
   ├──→ M14-002 (connectivity assumption breaks) ──┬──→ M14-003 (v2 protocol requirements)
   │                                                └──→ M14-006 (security requirements)
   └──→ M14-004 (zone + multi-depth model) ──→ M14-005 (weather boundary)
                                          all ──→ M14-009 (verification)
```

---

## 2b. Issues added by the 2026-08-26 architecture pass

Offline autonomy, the binding/policy model, and the extensible measurement model
added 35 issues. They are appended at the end of their milestones, so numeric
order remains a valid execution order.

```text
M1  014 offline policy payload ─┬─► 016 rhizo-policy crate ─► 018 policy no_std check
    015 offline event payload ──┴─► 017 extended fixture corpus
                                              └──► 019 M1 verification

M2  015 declare capabilities ──► 016 policy store ──► 017 isolation/runtime state
                                                          └──► 018 event buffer
    (policy evaluation and autonomous scheduling are deliberately absent)

M3  016 ingest replayed events ──► 017 record history gaps

M4  011 ingest capabilities ──► 012 expose connectivity mode

M5  013 bindings ──► 014 measurement policies ──┬─► 015 threshold evaluation
                                                 └─► 016 offline policy authoring

M2-017 mechanics ──► M6  019 shared evaluator + simulator integration ──┐
    020 reconciliation ─────┼─► 021 offline safety property tests
    (018 existing safety) ──┘

M6-019 + M6-021 ──► M8  015 isolation scenarios ──► 016 reconciliation scenarios

M9  015 NVS policy store ──► 016 offline evaluator ──┬─► 017 event buffer
                                                      └─► 018 monotonic budget

M12 013 binding editor ──► 014 threshold config
    015 connectivity views ──► 016 offline history

M13 013 release CI ──► 014 MSRV matrix
    015 observability profile

M14 007 Helm planning        008 future actuator model
```

**The chain that matters most** runs `M1-016` (the shared `rhizo-policy` crate)
→ `M2-017` (simulator mechanics only) → `M6-019` (the evaluator plus its sole
simulator call site) → `M9-016` (the firmware call site). If that crate is wrong,
both consumers are wrong in the same way; neither consumer may implement a
second evaluator.

---

## 3. Cross-milestone dependencies

Issues that reach **backwards across a milestone boundary**. These are the ones
most easily missed, because the milestone table alone does not show them.

| Issue | Depends on | Reason |
|---|---|---|
| M1-011 | M0-012 | extends the CI workflow with the `no_std` job |
| M1-012 | M0-010 | `TestClock` from testkit implements the `Clock` trait |
| M2-001 | M0-006 | uses `rhizo-telemetry` for logging |
| M2-011 | M1-010 | diffs captured output against the fixture corpus |
| M2-012 | M0-008 | needs the Mosquitto ACL configuration |
| M3-001 | M0-013 | needs the whole M0 baseline, not just the workspace |
| M3-006 | M1-004 | decodes the envelope type |
| M3-009 | M1-006 | applies telemetry payload range validation |
| M3-011 | M0-006 | registers metrics in the shared registry |
| M3-013 | M0-004 | implements the `Classify` trait from the conventions |
| M4-007 | M3-005 | readiness depends on the MQTT `Subscribed` state |
| M4-010 | M3-011 | extends the metric set |
| M5-002 | M4-009 | needs the API server and CORS configuration |
| M5-003, M5-005 | M1-012 | pure domain functions; do **not** need M5-001 |
| M6-005 | M4-004 | reuses the staleness threshold computation |
| M6-007 | M3-003 | queries `watering_events` for the rolling window |
| M6-011 | M0-007 | uses the shared backoff utility |
| M6-013 | M4-006 | pairs publication with drift detection |
| M6-014 | M3-011 | extends the metric set |
| M7-005 | M3-013 | `CloudError` implements `Classify` |
| M7-006 | M0-007 | uses the shared backoff utility |
| M7-010 | M6-018 | needs deterministic property-test infrastructure |
| M7-014 | M6-022 | emits events from every M6 state change |
| M10-008 | M6-013 | extends the config publication path |
| M12-001 | M6-022 | needs state and actions worth showing |
| M1-016 | M1-012 | `rhizo-domain` links `rhizo-policy` to validate and predict |
| M1-018 | M1-011 | extends the existing `no_std` CI job |
| M2-015 | M2-003 | capabilities ride in the status payload |
| M3-016 | M3-008 | replay reuses the dedup transaction |
| M4-011 | M4-001 | capabilities arrive in `device.status` |
| M5-013 | M4-011 | a binding may only name a **declared** capability |
| M6-019 | M6-002, M6-006, M2-017 | implements the shared offline gate and activates the prepared simulator seam |
| M6-020 | M3-016 | reconciliation consumes ingested replay |
| M8-015 | M6-019, M6-021 | full isolation scenarios require the shared evaluator and its safety tests, not M2 alone |
| M8-016 | M6-019, M6-020, M6-021 | reconciliation scenarios require evaluator, reconciliation, and offline safety work |
| M9-016 | M9-011 | offline dosing routes through the existing actuation gate |
| M12-013 | M12-008 | extends the profile editor surface |
| M13-014 | M13-013 | the MSRV matrix sits alongside release CI |
| M13-012 | M12-017 | extends the completed UI |

---

## 4. Critical path

The longest chain from an empty repository to the M8 software-only demo:

```text
M0-001 → M0-002 → M0-003 → M0-012 → M0-013
       → M1-001 → M1-004 → M1-008 → M1-009 → M1-019
       → M2-001 → M2-002 → M2-003 → M2-006 → M2-008 → M2-019
       → M3-001 → M3-002 → M3-003 → M3-008 → M3-009 → M3-018
       → M4-001 → M4-008 → M4-013
       → M5-001 → M5-005 → M5-009 → M5-012 → M5-017
       → M6-001 → M6-002 → M6-004 → M6-005 → M6-006 → M6-007
                → M6-008 → M6-009 → M6-010 → M6-012 → M6-018 → M6-022
       → M7-001 → M7-002 → M7-003 → M7-005 → M7-006 → M7-015
       → M8-001 → M8-002 → M8-003 → M8-004 → M8-005 → M8-006 → M8-012 → M8-017
```

Roughly 60 issues on the critical path out of 146 in M0–M8. The remainder are
parallelisable, concentrated in M4 (health checks), M8 (scenarios), and the pure
domain functions of M5.

---

## 5. How to choose the next issue

```text
1. Take the lowest-numbered issue in the current milestone whose
   Dependencies are all complete.
2. If several qualify, prefer the one on the critical path (§4).
3. If the milestone's verification issue is the only one left, run it —
   and do not mark the milestone DONE until its exit criteria in
   ROADMAP.md §2 are demonstrably met.
```

Issue numbering within a milestone is **already topologically sorted**: for
every issue, its dependencies have lower numbers within the same milestone, or
belong to an earlier milestone. Executing M*n*-001 through M*n*-0*k* in numeric
order is always valid. The graph above exists to reveal where that order can be
safely *widened* into parallel work, not to replace it.

---

## 6. Consistency

This document is verified against the issue files by `rhizo-docscheck`
(`cargo run -p rhizo-docscheck`), which asserts:

- every issue ID referenced here exists;
- every dependency listed in an issue file refers to an existing issue;
- the dependency relation is acyclic;
- within a milestone, dependencies point to lower-numbered issues or to earlier
  milestones — i.e. the numbering is a valid topological order;
- every milestone in ROADMAP.md has a matching issue directory.
