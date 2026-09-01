# Operational History — what is durably recorded, and what is derivable from it

A record of which operational facts the edge keeps, where each lives, and which
future questions can be answered from them without a schema redesign.

**This document describes no product.** Fleet views, an attention queue, an ROI
dashboard, technician workflows, organisations, and roles are all out of scope
and none is planned here. The narrow purpose is to make sure a fact that is
**cheap to record today and impossible to reconstruct tomorrow** is not being
thrown away — and, just as importantly, to say plainly which facts do *not* need
recording because they can be reconstructed or re-entered later.

**No derived business value is stored.** No "money saved", no "hours avoided",
no efficiency score. Those need external baseline data the system does not have
and should not guess at (§4). What is stored is what happened.

---

## 1. The rule this document applies

A fact earns a durable row when losing it is **irreversible**. Two questions
decide it:

1. **Is it a transition, or is it configuration?** A transition happens once, at
   an instant, and cannot be re-derived afterwards — a lockout being set, a
   device going isolated, a dose settling. Configuration is a current state that
   an operator can re-enter at any time — a plant's name, a species, a
   threshold, a site assignment.
2. **Does something already record it?** A second record of the same fact is
   schema churn, not durability.

Configuration that is wrong today can be fixed tomorrow. A transition that was
not recorded is gone.

---

## 2. Where each fact lives

### Identities and relationships

| Fact | Where | Durable? |
|---|---|---|
| Device identity, firmware, protocol version | `devices` | ✅ current state |
| Device capabilities as declared | `device_capabilities` | ✅ current state |
| Plant identity, species, pot volume, soil | `plants`, **soft-deleted** (`deleted_at`) | ✅ — a deleted plant's history stays attributable |
| Sensor bindings, roles, points | `sensor_bindings` | ⚠️ current state only |
| Actuator binding | `actuator_bindings` | ⚠️ current state only |
| Per-plant thresholds, offline policy | `measurement_policies`, `offline_policies` | ⚠️ current state only |
| Site / location / organisation | **not modelled** | — see §3 |

The ⚠️ rows are deliberate. A binding change loses the *previous* binding, but
every row that matters carries its own attribution at the time it was written:
`watering_events.device_id`, `commands.device_id`, and `measurements.device_id`
all record which hardware was actually involved. So "which device watered this
plant in March" is answerable from the events themselves and does not depend on
the binding table's history. See §3 for what this does not cover.

### Watering, dose, and outcome

| Fact | Where |
|---|---|
| Requested dose | `commands.requested_ml`, `watering_events.requested_ml` |
| Safety-authorised dose | the same value — the edge gate refuses rather than reducing, so today authorised == commanded (Verified Watering, PRD 160, splits the ladder into six) |
| Commanded dose, issue time, TTL | `commands` (`requested_ml`, `issued_at`, `expires_at`, `published_at`) |
| Device-side clamp | `command_results.result_json` (`clamped`, `delivered_ml`) |
| Delivered volume | `watering_events.delivered_ml` |
| Final command outcome and why | `commands.status` + `commands.reason`, `settled_at` |
| Automatic vs recommended vs manual vs detected | `commands.mode`, `watering_events.mode` |
| Offline autonomous doses | `watering_events.origin = 'offline_autonomous'` |
| Manual watering nobody commanded | `watering_events.mode = 'detected'` (from `detect::detect_manual_watering`) |
| Budget charged | derived from rows, never a counter (SAFETY-006) |

`watering_events` and `commands` are **ledger tables**: retention never deletes
them, and `ledger_tables_are_not_in_retention_source` fails if anyone tries.

### Safety blocks

| Fact | Where |
|---|---|
| Current block and reason | `plants.lockout_reason`, `lockout_since`, `lockout_until` |
| Who cleared it and when | `plants.lockout_cleared_by`, `lockout_cleared_at` |
| **Every block and unblock, with reason** | `plant_events` kinds `lockout_set` / `lockout_cleared` |
| Device-side refusal reason | `command_results.result_json.reason`, `commands.reason` |
| Threshold warnings and criticals | `plant_events`, severity `warning` / `critical` |
| Irrigation state transitions | `plant_events` kind `irrigation_state_changed` |
| Plant state transitions | `plant_events` kind `plant_state_changed` |
| Forward clock step | `plant_events` kind `clock_step` |

The `lockout_set` / `lockout_cleared` rows are **new** (see §5). Everything above
them already existed.

### Device health, faults, and connectivity

| Fact | Where |
|---|---|
| Device faults and their category | `device_events` (`kind`, `severity`, `detail_json`) |
| Leak, tank, pump faults reported by a device | `device_events`, plus the refusal reason on the command |
| No-delivery detection | `LockoutReason::NoDeliveryDetected`, now in `plant_events` |
| Sensor stuck / unhealthy | `sensor_stuck_state`, `device_events` |
| Isolation periods, with duration | `device_isolation_periods` (`started_at`, `ended_at`, `duration_ms`) |
| Isolated / reconciled transitions | `device_events` `device.isolated` / `device.reconciled` |
| Connectivity and power mode, wake windows | `devices` (`connectivity_mode`, `power_mode`, `expected_wake_at`, `overdue_at`, `missed_wake_count`) |
| Lost buffered device history | `history_gaps` (SAFETY-020) |
| Battery voltage and charge | `measurements`, kinds `battery_voltage` / `battery_percent` |

`device_events`, `device_isolation_periods`, and `history_gaps` are never pruned.
`measurements` **are**, at 90 days — see §3.

### Decisions and their reasons

| Fact | Where |
|---|---|
| Recommendation, dose, confidence, structured reasons, blocking lockout | `plant_recommendations` (`decision`, `recommended_ml`, `confidence`, `reasons_json`, `blocked_by`, `evaluated_at`) |
| Held doses for a sleeping device | `command_intents`, including `refusal_reason` |
| Reconciliation progress | `replay_progress` |

Recommendations are written **on change of answer**, not per tick, so the series
is a transition log rather than a sampling of a constant.

---

## 3. What is deliberately not recorded, and why that is safe

**Site, location, zone, organisation, customer.** Not modelled, and not added
here. This is *configuration*, not history: assigning a device to "Greenhouse 2,
bench 4" a year from now loses nothing, because the assignment is a present-tense
fact about where hardware is, and an operator can supply it retroactively. The
identities a site would group — `device_id`, `plant_id`, `actuator_id` — are
already stable primary keys, so a later grouping layer attaches to them without
touching anything below it. A zone entity is already reserved in
[PRD 140](../prd/140-field-readiness.md) and M14-004.

**Binding history.** Superseded bindings are not retained. The events that matter
carry their own `device_id` at the time of writing (§2), so historical
attribution does not depend on it. The residual loss is narrow: "this plant was
served by device A until March, then B" is not reconstructible from the binding
table alone. If it becomes needed, [PRD 150](../prd/150-per-plant-adaptive-water-model.md)
already introduces `plant_hydration_epochs`, whose whole purpose is to record
exactly that transition — so the fix has a home and should not be duplicated here.

**Raw measurements beyond 90 days.** Pruned by retention (ADR-004). Long-horizon
questions — battery degradation over a year, seasonal drying — are the subject of
M13-010's hourly downsampling, which keeps average, min, max, and count. Nothing
should be added here in anticipation of it.

**Derived operational or financial value.** No stored "money saved", "labour
hours avoided", "efficiency". These are not facts the system observes; they are
arithmetic over facts plus a baseline that lives outside the system entirely
(§4). Storing them would freeze one interpretation into the schema and make it
unfixable.

**Anything a future product layer would own.** Attention queues, technician
assignment, SLAs, billing, roles. None of them needs a fact recorded today that
this document does not already list.

---

## 4. Which future metrics are derivable, and which are not

Derivable **now**, from the tables above, with no schema change:

| Question | Derived from |
|---|---|
| Number of watering operations, by mode | `watering_events.mode`, `commands.mode` |
| Automatic vs manual vs detected split | the same |
| Watering failures and their kind | `commands.status` + `reason`, `command_results.result_json` |
| Volume delivered, per plant, per period | `watering_events.delivered_ml` |
| Number of safety blocks, by reason | `plant_events` `lockout_set`, `detail_json.reason` |
| Time a plant spent blocked | `lockout_set` → `lockout_cleared` pairs |
| Interventions: how often a person cleared a block | `lockout_cleared` with `cleared_by != 'auto'` |
| Maintenance incidents | `lockout_set` where the reason is `leak`, `pump_fault`, `no_delivery_detected`, `tank_low`; plus `device_events` at `warning`/`critical` |
| Device downtime | `device_isolation_periods.duration_ms`; `devices.missed_wake_count` |
| Lost device history | `history_gaps` |
| Sensor faults | `sensor_stuck_state`, `device_events` |
| Why the system recommended what it did | `plant_recommendations.reasons_json` |

Derivable **after Verified Watering** ([PRD 160](../prd/160-verified-watering.md)):

| Question | Needs |
|---|---|
| Verified water usage, as opposed to commanded | `watering_deliveries.measured_ml` |
| Share of operations physically verified | `watering_deliveries.evidence_level` |
| Requested-versus-delivered error | the six-dose ladder |
| Unknown outcomes | `DeliveryOutcome::OutcomeUnknown` |
| No-flow incidents caught on the first dose | `DeliveryOutcome::NoFlow` |

**Not derivable, ever, from this system alone.** Anything comparing what happened
against what *would* have happened:

- avoided or reduced manual checks
- labour hours saved
- plants saved
- cost per plant, or any monetary figure

Each needs an **external baseline** that only the operator has: how often someone
walked the site before, what an hour costs, what a plant is worth. The system's
side of that calculation is already complete — it can say how many interventions
occurred, how many blocks, how much water, and how much downtime. The other side
must be supplied, and should be supplied *at report time*, not stored as a field.
This is the boundary: **the edge records what happened; a baseline for what
otherwise would have happened is business data and lives elsewhere.**

---

## 5. Change history

**2026-09-01 — lockout transitions became durable locally.** `set_lockout` wrote
the current state to `plants` and emitted a cloud event, and nothing else.
`outbox::emit` writes no row at all while cloud sync is disabled — the default,
and the local-first case — so a deployment that never enabled the cloud kept no
record of a leak lockout once it had cleared. The reason, the duration, and the
fact that a person intervened were all unrecoverable.

It now also writes a `plant_events` row in the same transaction, the way
`plant::set_state` already did for plant-state changes: `lockout_set` at
`warning`, `lockout_cleared` at `info`, carrying the reason, the prior reason on
a clear, the hold deadline, and who cleared it. The prior reason is read inside
the transaction before the update, because afterwards it is gone.

The row is written **only when the lockout actually changed**. Two callers
re-assert a lockout unconditionally — a forward clock step locks every plant, and
a `clock_unsynced` rejection locks its plant on every occurrence — so an ungated
insert would turn one incident into a stream of identical rows and make a block
count meaningless.

No migration, no new table, no new API, no change to any safety rule: the
lockout lifecycle, its explicit-clear semantics, and every gate are untouched.
