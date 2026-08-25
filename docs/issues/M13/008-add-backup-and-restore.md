# Issue M13-008 — Add backup and restore tooling

**Milestone:** M13 · **PRD:** [PRD 130](../../prd/130-multi-plant-home.md) · **Depends on:** M13-007

## Context

ADR-004 risk: SD card wear on a Pi is the most likely long-term hardware
failure. A backup that has never been restored is not a backup.

## Goal

Make the deployment recoverable.

## Scope

- `rhizo-backup create` with a WAL checkpoint and a consistent copy
- `restore` with verification
- `verify` comparing row counts and a watering-history checksum
- Scheduled backup with retention
- **A restore failure is itself notification-worthy**

## Non-goals

- Off-site backup.

## Dependencies

- M13-007

## Implementation notes

`verify` is the part that makes this real. Comparing row counts and a
checksum over the watering history proves the backup contains the ledger, which
is the data that cannot be regenerated.

Alerting on backup failure closes the loop: a silently failing nightly backup is
the classic way to discover you have none.

## Acceptance criteria

- [ ] `create` produces a consistent backup while the system runs.
- [ ] `restore` reproduces identical row counts.
- [ ] `verify` detects a corrupted backup.
- [ ] The watering-history checksum matches after restore.
- [ ] Scheduled backups run with retention.
- [ ] A backup failure raises a notification.

## Verification

```bash
cargo run -p rhizo-backup -- create
cargo run -p rhizo-backup -- verify <file>
cargo run -p rhizo-backup -- restore <file>
```

## Tests required

- Consistency under concurrent writes.
- **Restore fidelity.**
- Corruption detection.

## Documentation impact

- Backup and restore procedure.

## Files likely affected

```text
crates/backup/src/main.rs
```
