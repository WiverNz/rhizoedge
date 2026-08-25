# Issue M2-007 — Implement the persistent device state file

**Milestone:** M2 · **PRD:** [PRD 020](../../prd/020-device-simulator.md) · **Depends on:** M2-001

## Context

PRD 020's data model mirrors what NVS holds on real hardware, deliberately:
it makes restart behaviour comparable between the simulator and the firmware,
which is what M9-014's conformance test relies on.

## Goal

Persist the state that must survive a simulator restart.

## Scope

- A JSON state file with boot count, applied config version, daily totals, command ring, in-flight dose, pending results
- Loaded at start, written on every change
- A 16-entry command dedup ring with outcomes
- Corrupt file: start fresh with a new `boot_id`, log WARN
- `--state-file` to select the path

## Non-goals

- Real NVS (M9-004).

## Dependencies

- M2-001

## Implementation notes

Corrupt state must not prevent startup — but it must not be trusted either.
Starting fresh and logging loudly is the correct behaviour; silently continuing
with half-parsed state is not.

Writes must be atomic (write to a temp file and rename), otherwise
`--fault restart-mid-dose` can produce a truncated file and the test then
exercises the corrupt-file path instead of the interrupted-dose path.

## Acceptance criteria

- [ ] State survives a restart.
- [ ] The command ring persists and deduplicates across restarts.
- [ ] `delivered_today_ml` persists and resets on a day boundary.
- [ ] A corrupt file starts fresh with a WARN and a new `boot_id`.
- [ ] Writes are atomic — a kill during a write leaves a valid file.
- [ ] The ring evicts at 16 entries.

## Verification

```bash
cargo test -p device-simulator state::
```

## Tests required

- Round trip.
- Ring eviction.
- Corrupt-file recovery.
- Atomicity under a simulated kill.
- Daily rollover.

## Documentation impact

- None.

## Files likely affected

```text
crates/device-simulator/src/state.rs
```
