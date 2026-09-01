# Issue M15-002 — Persist hydration epochs, observations, and model state

**Milestone:** M15 · **PRD:** [PRD 150](../../prd/150-per-plant-adaptive-water-model.md) · **Depends on:** M15-001

## Context

The model must survive the 90-day raw-measurement prune, a restart, and a
process crash, and it must be rebuildable from what it persisted. That means a
durable **derived-observation ledger** distinct from both the raw measurements
and the authoritative watering ledger.

This is also the issue that ends the single-migration regime.
`canonical_baseline_contains_the_final_schema` asserts `MIGRATOR.iter().count()
== 1` and fails the moment a second migration file appears — deliberately, as
the prompt to ask whether the first release has happened. By M15 it has
(M13-013 ships release CI), so the answer is yes and the baseline stops
absorbing changes here.

## Goal

Add `migrations/edge/0002_adaptive_model.sql`, the repository module, and the
migration-regime change, with no estimator and no behaviour change.

## Scope

- `0002_adaptive_model.sql` creating `plant_hydration_epochs`,
  `plant_drying_segments`, `plant_dose_responses`, `plant_hydration_model`,
  `plant_adaptive_decisions`, and `plants.adaptive_mode TEXT NOT NULL DEFAULT
  'disabled'`.
- `crates/storage/src/repo/hydration.rs` with `sqlx::query!`-checked statements.
- Rewriting `canonical_baseline_contains_the_final_schema` into a
  forward-migration assertion: migrations apply in order from an empty database,
  the resulting table set is exact, and `0001_initial.sql` is unmodified.
- A bounded per-`(plant, epoch)` observation cap in
  `repo::retention::run_batch`, dropping oldest-and-lowest-weighted first.

## Non-goals

- Writing any observation. M15-003 and M15-005.
- Reading any observation into an estimate. M15-004 onward.
- Touching `watering_events`, `commands`, or `device_events` — they are the
  ledger and retention never sees them.

## Dependencies

- M15-001

## Implementation notes

**Do not edit `0001_initial.sql`.** An existing deployment has applied it, and
`sqlx` compares checksums; a modified baseline makes every M13 installation
refuse to start. That is the whole reason the test exists.

`plants.adaptive_mode` defaults to `'disabled'`, enforced in the column default
**and** in `repo::plant::create`, the same belt-and-braces
`auto_watering_enabled` gets and for the same reason: no caller can forget it.

Every observation row carries `status` (`accepted` / `rejected_outlier` /
`superseded`) rather than being deleted. A rejected observation is evidence
about the estimator, and an epoch opened by mistake is only diagnosable if its
observations still exist.

Index for the access pattern the estimators actually have:
`(plant_id, epoch, status, ended_at DESC)` on segments and
`(plant_id, epoch, status, dosed_at DESC)` on responses.

## Acceptance criteria

- [ ] `0002_adaptive_model.sql` applies cleanly onto a database already carrying
      `0001_initial.sql`, and onto an empty one.
- [ ] `0001_initial.sql` is byte-identical to its pre-M15 content.
- [ ] The rewritten baseline test asserts the exact ordered migration list and
      the exact table set.
- [ ] `adaptive_mode` defaults to `disabled` in both the schema and
      `repo::plant::create`.
- [ ] Retention bounds the two observation tables and touches no ledger table.
- [ ] Every statement in `repo::hydration` is compile-time checked.

## Verification

```bash
cargo test -p rhizo-storage migrate::
cargo test -p rhizo-storage hydration::
cargo test -p edge-controller retention::
```

## Tests required

- Migration from a `0001`-only database preserves every existing row.
- `ledger_tables_are_not_in_retention_source` still passes.
- The observation cap prunes deterministically and never crosses an epoch it was
  not asked to prune.
- A plant created through the repository has `adaptive_mode = 'disabled'`.

## Documentation impact

- `docs/adr/004-sqlite-edge-persistence-model.md`: the baseline is closed and
  forward migrations begin.
- `docs/testing/local-development.md` §9: the "delete and re-create your
  database" advice needs the post-baseline wording.
- `CLAUDE.md` §7: the "there is one migration" paragraph becomes historical.

## Files likely affected

```text
migrations/edge/0002_adaptive_model.sql
crates/storage/src/migrate.rs
crates/storage/src/repo/mod.rs
crates/storage/src/repo/hydration.rs
crates/storage/src/repo/retention.rs
crates/storage/src/repo/plant.rs
```
