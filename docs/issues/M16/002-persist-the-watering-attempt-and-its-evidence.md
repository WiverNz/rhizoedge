# Issue M16-002 — Persist the watering attempt and its evidence

**Milestone:** M16 · **PRD:** [PRD 160](../../prd/160-verified-watering.md) · **Depends on:** M16-001

## Context

`commands` records what was asked; `watering_events` records the claim that
water reached the plant, and is created only for `completed`
(`creates_watering_event`). Neither can hold the evidence for an attempt that
delivered nothing — which is exactly the attempt an operator most needs to see.

## Goal

`watering_deliveries`, `delivery_observations`, and `actuator_health`, keyed so
that a replay can never create a second attempt.

## Scope

- `migrations/edge/0003_verified_watering.sql`.
- `watering_deliveries` keyed by `command_id`, carrying the six doses, evidence
  level, outcome, unknown reason, timings, `settle_ok`, witness health,
  calibration version and date, firmware version, reconciliation status, and
  `credited_ml`.
- `delivery_observations` — bounded raw witness samples per attempt.
- `actuator_health` — one row per `(device_id, actuator_id)`.
- `crates/storage/src/repo/delivery.rs`, every statement compile-time checked.
- Retention: bound `delivery_observations` by per-attempt and total row count;
  **`watering_deliveries` is audit data and is never pruned**.

## Non-goals

- Writing any row from a real result. M16-010.
- Adding columns to `watering_events`. Its meaning is unchanged, and a no-flow
  attempt must not create one.

## Dependencies

- M16-001

## Implementation notes

`command_id` as the primary key is the whole idempotency story (F-160-20). An
`INSERT … ON CONFLICT(command_id) DO UPDATE` is what makes a replayed result
update one row; an autoincrement id with a `command_id` index would allow two
attempt rows for one command, which is the bug this key exists to prevent. Test
it by replaying the same result ten times.

`credited_ml` is **stored**, not recomputed on read. The rolling cap is derived
from rows precisely so it survives a logic change; a charge that silently
re-derives under new rules would rewrite history.

Extend `ledger_tables_are_not_in_retention_source` to include
`watering_deliveries`. It is the same class of table as `watering_events` and
for the same reason.

If M15-002 has already landed, this is `0003` onto `0002`. If M16 runs first,
this is the migration that ends the single-baseline regime, and
`canonical_baseline_contains_the_final_schema` must be converted to a
forward-migration assertion here instead — the test fires deliberately, as the
prompt to ask whether the first release has happened. By M16 it has.

## Acceptance criteria

- [ ] The migration applies onto the current baseline and onto an empty database.
- [ ] Earlier migrations are byte-identical.
- [ ] `watering_deliveries` is keyed by `command_id`; ten replays produce one row.
- [ ] `credited_ml` is stored, not recomputed.
- [ ] `delivery_observations` is bounded per attempt and in total.
- [ ] `watering_deliveries`, `watering_events`, `commands`, and `device_events`
      appear in no `DELETE FROM` in the retention source.
- [ ] Every statement in `repo::delivery` is compile-time checked.

## Verification

```bash
cargo test -p rhizo-storage migrate::
cargo test -p rhizo-storage delivery::
cargo test -p edge-controller retention::
```

## Tests required

- Migration preserves every existing row.
- Ten replays of one result yield one attempt row and one charge.
- Observation pruning is deterministic and never crosses an attempt it was not
  asked to prune.
- `ledger_tables_are_not_in_retention_source` covers the new audit table.

## Documentation impact

- `docs/adr/004-sqlite-edge-persistence-model.md`: the new tables and their
  retention class.
- `docs/testing/local-development.md` §9, if the migration regime changes here.

## Files likely affected

```text
migrations/edge/0003_verified_watering.sql
crates/storage/src/migrate.rs
crates/storage/src/repo/mod.rs
crates/storage/src/repo/delivery.rs
crates/storage/src/repo/retention.rs
```
