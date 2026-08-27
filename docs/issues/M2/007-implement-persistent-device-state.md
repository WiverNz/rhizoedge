# Issue M2-007 — Implement the persistent device state file

**Milestone:** M2 · **PRD:** [PRD 020](../../prd/020-device-simulator.md) · **Depends on:** M2-001

## Context

PRD 020's data model mirrors what NVS holds on real hardware, deliberately:
it makes restart behaviour comparable between the simulator and the firmware,
which is what M9-014's conformance test relies on.

## Goal

Persist the state that must survive a simulator restart.

## Scope

- A JSON state file with boot count, applied config version, daily hard-limit
  totals, command ring, in-flight dose, pending results, policy active/staging
  checksum and version state, applied policy versions, offline runtime
  budget/cooldown/confirmation/dose-count state, and the bounded replay buffer
  with gap/ack metadata
- Loaded at start, written on every change
- A 16-entry command dedup ring with outcomes
- Corrupt safety-critical state: start only in diagnostic/monitoring mode with an
  observable persistent-state fault and actuation disabled
- `--state-file` to select the path

## Non-goals

- Real NVS (M9-004).

## Dependencies

- M2-001

## Implementation notes

Corrupt state need not prevent monitoring and diagnostics, but it must fail
closed. The simulator MUST NOT replace corrupted safety state with fresh
deduplication, budget, cooldown, policy, or in-flight defaults. It refuses
actuation commands that depend on that state, keeps offline policy inactive,
and reports the persistent-state fault. If non-safety physical-model state is
separately recoverable, that portion alone may reset. This matches the future
firmware contract: corruption cannot make either implementation more permissive.

Writes must be atomic (write to a temp file and rename), otherwise
`--fault restart-mid-dose` can produce a truncated file and the test then
exercises the corrupt-file path instead of the interrupted-dose path.

## Acceptance criteria

- [x] State survives a restart.
- [x] The command ring persists and deduplicates across restarts.
- [x] `delivered_today_ml` persists and resets on a day boundary.
- [x] Corrupt safety-critical state raises an observable persistent-state fault,
      permits monitoring/diagnostics, and disables all pump actuation.
- [x] Corruption cannot clear dedup/in-flight uncertainty, activate a policy,
      replenish a budget, shorten a cooldown, or restore actuation permission.
- [x] Commands requiring persisted safety state are refused while faulted.
- [x] Writes are atomic — a kill during a write leaves a valid file.
- [x] The ring evicts at 16 entries.

## Verification

```bash
cargo test -p device-simulator state::
```

## Tests required

- Round trip.
- Ring eviction.
- Corrupt-file fail-closed startup and explicit fault reporting.
- Property/integration test proving arbitrary corruption cannot restore
  actuation permission or make budget/cooldown state more permissive.
- Atomicity under a simulated kill.
- Daily rollover.

## Documentation impact

- PRD 020 §Data model: the state file carries a **whole-file checksum**, not only
  a checksum on the policy blob. A property test showed that one flipped digit
  in `delivered_today_ml` is still valid JSON and would otherwise be applied
  silently.

## Files likely affected

```text
crates/device-simulator/src/state.rs
```
