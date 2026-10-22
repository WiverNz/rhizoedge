# ADR-021 — Independent plant lockouts: latches, holds, and derived conditions

## Status

Accepted — 2026-10-22. Implemented on the edge in five stages between
2026-10-21 and 2026-10-22 (commits `c7dfa76`, `2efa186`, `f1901ad`,
`5fdbc23`, and the stage-6 change that records this ADR); the history is in
[docs/reports/M9.md](../reports/M9.md) §14 and §15.

**Amends [ADR-004](004-sqlite-edge-persistence-model.md)**: `plant_lockouts` is
the authoritative lockout state, and `plants.lockout_reason` /
`plants.lockout_since` became a display summary that no decision reads.

**Amends [ADR-006](006-irrigation-state-machine-ownership.md)**: the safety gate
reads recorded lockouts as two lists rather than one sticky reason, its step
order changed, and `Locked(reason)` no longer clears itself — a latch clears
only by an operator, a hold only by time, and a derived condition only by
re-evaluation.

**Amends [PRD 060](../prd/060-irrigation-control-and-safety.md)** F-060-40 …
F-060-43 and F-060-51, which now describe the model below.

Changes no wire format, no firmware, no `no_std` crate, and no cloud event kind.

## Context

Until 2026-10-21 a plant had one lockout: the `plants.lockout_reason` column,
which every writer replaced. A single slot cannot hold two independent facts,
and SAFETY-003 ("clearing requires the leak signal to be absent *and* an
explicit operator reset") depends on not losing either of them. Tracing every
writer found six ways a latched alarm could disappear with no operator:

| Path | How the latch was lost |
|---|---|
| A | A forward clock step (F-060-51) wrote a held `uncertain` over every plant; when the hold expired the latch was gone |
| B | A detected leak overwrote `pump_fault` / `no_delivery_detected` / `max_doses_reached`; the operator's leak reset then lifted a fault nobody reset |
| C | A silent leak sensor (`LeakState::Unknown → Uncertain`) was answered before the sticky lockout and persisted over `leak` (the CI failure in End-to-end run 39663877363; fixed first, §14) |
| D | A device's `clock_unsynced` refusal, settled in the result transaction, wrote over the latch |
| E | The automatic clear compared a stale read against a newer write and removed a reason raised in between |
| F | A command issued or published while a lockout was being recorded reached the device |

C was a gate-order defect and was fixed in place. A, B, D and E are structural:
any fix inside one column has to choose which fact to drop. F is a race between
the decision and the MQTT publish.

## Decision

### 1. Three classes, and only two are stored

`irrigation::types::lockout_class` is an exhaustive match, so a new
`LockoutReason` does not compile until it is classified:

| Class | Reasons | Ends | Stored |
|---|---|---|---|
| **Latch** | `leak`, `pump_fault`, `no_delivery_detected`, `max_doses_reached`, `unknown` | only by an operator clearing that one reason (F-060-41) | yes |
| **Hold** | `uncertain` (forward clock step, F-060-51), `clock_unsynced` (a device refused a command for its clock) | only when `hold_until` passes on the edge clock; never by a request | yes |
| **Derived** | `stale_data`, `sensor_fault`, `tank_low`, `daily_limit`, `no_actuator`, and the `uncertain` of an unknown leak, tank, or reconciliation | when the next evaluation no longer derives it (F-060-40) | **never** |

A derived condition is a function of the current inputs; storing it would only
create a second copy that can disagree with them. `unknown` stands for any
stored name this edge does not recognise and is a latch: an unexplainable row
blocks until a person clears it by its exact stored name (SAFETY-012).

### 2. One row per `(plant_id, reason)`

```sql
CREATE TABLE plant_lockouts (
    plant_id   TEXT NOT NULL REFERENCES plants(plant_id),
    reason     TEXT NOT NULL,
    kind       TEXT NOT NULL CHECK (kind IN ('latch', 'hold')),
    since      INTEGER NOT NULL,
    hold_until INTEGER,
    CHECK ((kind = 'hold') = (hold_until IS NOT NULL)),
    PRIMARY KEY (plant_id, reason)
);
```

`rhizo_storage::repo::lockout` is the only writer:

- `latch` inserts once and keeps the original `since`; it refuses a hold or
  derived reason, so a latch can never be stored as something that expires.
- `hold` inserts or **lengthens**, never shortens, and accepts only
  `uncertain` and `clock_unsynced`.
- `clear_latch` deletes exactly one named latch and never a hold.
- `active` returns every latch and every hold whose deadline is ahead, for live
  plants only. `active_all` is the same for every plant, for the overview.

No writer can displace another's row, so paths A, B and D cannot occur: a clock
step adds a hold *beside* a leak, a leak is latched *beside* another fault, and a
`clock_unsynced` refusal adds a hold inside the settling transaction.

### 3. The gate reads both lists, and its order

`IrrigationInputs` carries `latched: &[LockoutReason]` and `holds: &[Hold]`,
filled only from `repo::lockout::active` by `control::inputs::records`. The
gate, in order:

0. no actuator binding → `NoActuator` (SAFETY-018);
1. leak `Detected` → `Leak`;
3. any latch, or any hold with `now < until` → the best-ranked one;
2. leak `Unknown` → `Uncertain`;
4. … the derived checks (reconciliation, tank, soil, budget), unchanged.

Step 3 runs before step 2 so that a silent leak sensor cannot replace a latched
`leak` with an auto-clearing reason (path C). Every early return refuses, so
the order decides only which reason is *reported*. Which recorded reason is
reported is `display_rank` — latches, then holds, then derived — a fixed total
order, so the answer cannot depend on how many records exist or the order they
were written in (`gate::recorded`, including a property test over every
permutation).

### 4. Writers

- **The control pass** latches a `Lock` whose class is `Latch` and that is not
  already recorded. It records nothing else, and **the automatic clear is
  gone** (path E): a derived condition simply stops being derived.
- **The clock step** records `hold("uncertain", now + cooldown)` for every
  plant.
- **The result settle** records `hold("clock_unsynced", now + command TTL)` in
  the same transaction that settles the command.
- **The operator** calls `POST /plants/{id}/lockout/clear {reason}`, which runs
  `clear_latch` (§5).

### 5. Atomic operations

Each of these is one SQLite transaction begun `BEGIN IMMEDIATE`, so it holds the
single write lock from its first statement; no transaction waits on MQTT, so
none can deadlock.

| Operation | Inside the transaction |
|---|---|
| `clear_latch` | for `leak`, re-read the bound leak stream and require an `ok`, positively dry, fresh reading; delete the one row; write `lockout_cleared` + outbox `lockout.cleared` |
| `command_repo::issue` | refuse (`IssueOutcome::Blocked`, nothing written) if the plant has any latch or active hold at `issued_at`; otherwise insert the command |
| `command_repo::authorize_publish` | before **every** publish attempt, retries included: the command must still be `issued` and its plant unlocked; otherwise settle it `failed` / `blocked_by_lockout` and move the plant to `recheck` in the same transaction |
| `latch` / `hold` | insert or lengthen the row and write `lockout_set` + outbox `lockout.set` only when something changed |

Neither command check is a second gate: both read the records the gate reads
and can only refuse.

### 6. The summary column

`plants.lockout_reason` is written by `command::set_summary` with the reason the
gate reported, for display and for the existing `lockout` API field.
`the_summary_column_decides_nothing` writes a lie into it and asserts no
decision changes. The summary emits `lockout_set` / `lockout_cleared` only for
what no record covers — a derived block, or a hold that expired — so each block
is counted once. The `lockouts_total{reason}` counter follows the summary;
`plants_locked_out` counts plants the gate refuses.

### 7. Migration

`migrations/edge/0002_plant_lockouts.sql` is the first forward migration;
`0001` is unchanged. The backfill is fail-closed and idempotent
(`ON CONFLICT DO NOTHING`):

- a known explicit-clear reason → latch;
- an unrecognised or blank reason → latch (blank stored as `unknown`), cleared
  by its exact name;
- a known hold or derived reason with a `lockout_until` → hold, expired or not
  (the SQL cannot see the edge clock; the reader decides);
- a known derived reason with no deadline → dropped, because the next pass
  re-derives it from live inputs;
- a missing `since` falls back to the plant's `created_at`.

Every migrated row writes a `lockout_migrated` `plant_events` row, `warning`
for an unrecognised name. The automatic pre-migration backup runs first.

### 8. The API

`GET /plants/{id}`, `GET /plants`, and `GET /overview` carry `lockouts[]` in
display order with `reason`, `kind`, `since`, `hold_until`,
`operator_clearable`, and `recognised`. `lockout` remains as the one-reason
summary. `lockout/clear` answers 200 with `cleared` and `remaining[]`, 409
`not_locked_out` (with `details.lockouts`) for a reason that is not a recorded
latch, and 409 `hold_not_clearable` for a hold. `wrong_reason` is retired
([http-api-boundaries.md](../protocol/http-api-boundaries.md)).

## Alternatives considered

**Keep the column and make writers conditional** (a clock step skips a latched
plant, a leak does not replace another latch). Rejected: each rule drops one of
two true facts. Skipping the clock step leaves an operator who resets the leak
inside the cooldown with a plant the step should still be holding.

**A bitmask or JSON list in the `plants` row.** Rejected: every writer becomes a
read-modify-write of one cell, which reintroduces path E as a lost update, and
neither gives a per-reason `since`, deadline, or audit row.

**Store derived conditions too.** Rejected: a stored copy of "the tank is low"
can disagree with the tank, and then something has to decide which wins.

**Fold `0002` into the baseline.** Rejected: it would have required recreating
databases and discarding the alarms they hold.

**Recall a published command when a lockout appears.** Not possible: MQTT has
no recall, and the device holds no notion of a plant lockout. See Risks.

## Consequences

- An alarm can only be removed by the one thing allowed to end it. Paths A–E are
  closed by construction rather than by care in each writer.
- Clearing is per reason. An operator with two faults clears two times, and the
  response says what remains.
- An unrecognised stored reason is visible (`recognised: false`) and blocks
  until cleared by name.
- A command refused at publish time is `failed` with `blocked_by_lockout` and its
  volume stays charged to the rolling window, as for `publish_failed` — which
  errs towards less water.
- Scenario resets and any test that hard-deletes a plant must delete
  `plant_lockouts` first; production only soft-deletes.

## Risks

- **Path F is narrowed, not closed.** Between `authorize_publish` committing and
  the broker taking the message, a newly recorded lockout does not stop that
  message; an accepted QoS 1 message may also be retransmitted after a
  reconnect. Bounded by the command TTL (the device refuses an expired command),
  by the device's own leak and tank refusal (protocol §5.8), and by the next
  decision, which refuses at once. Pinned as a test,
  `a_latch_after_authorisation_is_the_window_that_remains`.
- **The device does not know about plant latches.** `no_delivery_detected`,
  `pump_fault`, and `max_doses_reached` are edge-side facts; a device whose own
  checks pass will execute a command that escaped through path F, and an
  isolated device waters from its offline policy without them (ADR-015). A leak
  is the exception that the device also refuses on its own.
- **Device diagnostics are not plant-scoped.** `POST /devices/{id}/commands/calibrate`
  runs the pump and passes the device's §5.8 gate (leak, tank, hard limits) but
  no plant lockout, so a recorded `no_delivery_detected` or `pump_fault` does not
  refuse it. Open question; behaviour unchanged.
- **`clearable` means two things.** In `GET /plants/{id}`'s `lockout` it means
  "clears by itself"; in `POST /water`'s 409 body it means "needs an operator".
  `lockouts[]` avoids the word (`operator_clearable`). Pre-existing; unchanged.
- **No edge writer raises `pump_fault` today.** It exists as a latch class, is
  migrated, displayed, and clearable, but the device's pump refusal
  (`pump_unavailable`) is not latched by the edge. Pre-existing; unchanged.

## Follow-up

- [PRD 060](../prd/060-irrigation-control-and-safety.md) F-060-40 … 43, 51.
- [safety-invariants.md](../architecture/safety-invariants.md) SAFETY-003.
- [docs/reports/M9.md](../reports/M9.md) §15.
- SCEN-046 in [failure-scenarios.md](../testing/failure-scenarios.md).
- Decisions outstanding: the calibration question and the `clearable` field
  above.
