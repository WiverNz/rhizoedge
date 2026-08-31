# Issue M7-008 — Implement the outbox cap with value-tiered pruning

**Milestone:** M7 · **PRD:** [PRD 070](../../prd/070-cloud-sync-and-storage.md) · **Depends on:** M7-006

## Context

Failure-model 4.5: during a prolonged outage the outbox must not grow without
bound — but **history is nice, and the ledger of what the machine did to a
living plant is not optional**. The tier column encodes that distinction.

## Goal

Bound outbox growth while preserving high-value events.

## Scope

- `outbox_max_rows` (default 500 000)
- At the cap, prune `value_tier = 'low'` oldest first
- **Never prune `value_tier = 'high'`**
- `value_tier` assigned at the single outbox write site, defaulting to `high`
- Pruning emits an alert-level log and increments `cloud_events_dropped_total`

## Non-goals

- Compressing or aggregating pruned data.

## Dependencies

- M7-006

## Implementation notes

Tiering: measurements are `low`; watering events, commands, lockouts, and
device faults are `high`. Defaulting to `high` means a new event kind is
preserved unless someone deliberately marks it disposable — the safe default is
to keep.

Assert the never-prune property with a test that fills the outbox well past the
cap and confirms every high-tier row survives.

## Acceptance criteria

- [x] Growth stops at the cap **while there is low-tier history to drop**. Under
      high-tier-only pressure the outbox exceeds the cap and nothing is pruned —
      preservation wins, and the original wording of this criterion was the
      stronger claim the code correctly does not make. See the post-M7 note below.
- [x] Low-tier events are pruned oldest first, whether `pending` or
      `quarantined`. Both count as pressure, so both must be prunable.
- [x] **Every high-tier event survives** filling the outbox to twice the cap.
- [x] `value_tier` defaults to `high`.
- [x] Pruning logs at alert level and increments the counter.
- [x] It is assigned at exactly one write site.

## Post-M7 correction — 2026-08-31

Two defects behind the ticks above, both found by asking what the tests actually
covered rather than what they were named. Full write-up in
[docs/reports/M7.md](../../reports/M7.md) §Post-M7 outbox-retention correction.

- **Low-tier `quarantined` rows were counted as pressure but were not prunable.**
  The cap counted `status!='synced'` and deleted only `status='pending'`, so each
  quarantined low-tier row permanently inflated the excess — evicting one extra
  live row apiece — while nothing could ever remove it. Counted and prunable are
  now the same set, differing only by tier.
- **"Growth stops at the cap" was never true for high-tier pressure**, and could
  not be without deleting the ledger. Now stated as it is, and asserted by
  `the_cap_yields_to_preservation_under_high_tier_pressure` so nobody later
  "fixes" the cap into a data-loss bug.

F-070-29 (synced rows pruned after 24 h) was implemented but had **no test at
all**: the retention test asserted `processed` and `measurements` and never
looked at `pending_cloud_events`. It now has five, including the boundary.

## Verification

```bash
cargo test -p edge-controller outbox::prune
cargo test -p edge-controller --lib cloud::drain
cargo test -p rhizo-storage --lib outbox
cargo test --test integration outbox_cap
```

## Tests required

- **High-tier preservation under extreme growth (SCEN-065).**
- Low-tier oldest-first pruning.
- Default tier.
- Counter and log.

## Documentation impact

- None.

## Files likely affected

```text
crates/edge-controller/src/cloud/prune.rs
crates/storage/src/repo/outbox.rs
```
