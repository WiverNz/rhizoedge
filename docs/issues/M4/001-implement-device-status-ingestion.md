# Issue M4-001 — Implement device status ingestion

**Milestone:** M4 · **PRD:** [PRD 040](../../prd/040-device-registry-and-health.md) · **Depends on:** M3-016

## Context

Protocol section 5.5. Retained status is how a subscriber learns device state
without waiting for the next heartbeat.

## Goal

Ingest status messages into the device registry.

## Scope

- Handle `device.status` in the pipeline
- Record status, firmware and protocol version, `boot_id`, `clock_synced`, `applied_config_version`, uptime, heap, RSSI
- Update `last_seen_at`
- Record `online`/`offline` transitions as device events

## Non-goals

- LWT specifics (M4-002).
- Auto-registration (M4-003).

## Dependencies

- M3-016

## Implementation notes

Status resolution must be **order-insensitive**: take the status from the
message with the greater `received_at`. A late-arriving LWT must not be able to
mark a live device dead (failure-model 1.4).

Only log a transition at INFO; a heartbeat that confirms the existing state is
not news.

## Acceptance criteria

- [ ] A status message updates the registry.
- [ ] An `online` transition is recorded as an event.
- [ ] A retained status is applied on subscribe.
- [ ] An out-of-order older status does **not** overwrite a newer one.
- [ ] Only transitions are logged at INFO.

## Verification

```bash
cargo test -p edge-controller device::status
cargo test --test integration retained_status
```

## Tests required

- Status update.
- Transition events.
- Out-of-order resolution.

## Documentation impact

- None.

## Files likely affected

```text
crates/edge-controller/src/pipeline/status.rs
crates/storage/src/repo/device.rs
```
