# Issue M4-001 — Apply ingested device status to the registry

**Milestone:** M4 · **PRD:** [PRD 040](../../prd/040-device-registry-and-health.md) · **Depends on:** M3-018

## Context

Protocol section 5.5. Retained status is how a subscriber learns device state
without waiting for the next heartbeat.

## Goal

Apply already-ingested status messages to the device registry and publish the
Edge time response.

## Scope

- Extend the existing M3 `device.status` pipeline branch after validation and
  durable raw-status persistence; do not add another MQTT consumer or decoder
- Record status, firmware and protocol version, `boot_id`, `clock_synced`, `applied_config_version`, uptime, heap, RSSI
- Update `last_seen_at`
- Record `online`/`offline` transitions as device events
- **On every status received, publish `edge.time` to that device** — live, `retain=false`, QoS 1 (F-040-17)

## Non-goals

- MQTT subscription, envelope decoding, identity validation, `received_at`
  stamping, quarantine, and raw durable status persistence (M3).
- LWT specifics (M4-002).
- Auto-registration (M4-003).

## Dependencies

- M3-018

## Implementation notes

Status resolution must be **order-insensitive**: take the status from the
message with the greater `received_at`. A late-arriving LWT must not be able to
mark a live device dead (failure-model 1.4).

Registry projection changes apply only after the existing M3 persistence path
accepts a new status effect. F-040-17 is intentionally per validated receipt:
even an exact transport duplicate still triggers a fresh, non-retained
`edge.time` response without reapplying the registry effect.

Only log a transition at INFO; a heartbeat that confirms the existing state is
not news.

## Acceptance criteria

- [ ] A status message updates the registry.
- [ ] An `online` transition is recorded as an event.
- [ ] A retained status is applied on subscribe.
- [ ] An out-of-order older status does **not** overwrite a newer one.
- [ ] Only transitions are logged at INFO.
- [ ] Receiving a status triggers an `edge.time` publish to that device.
- [ ] The `time` publish is **never** retained, asserted by a fresh-subscriber test.

## Verification

```bash
cargo test -p edge-controller device::status
cargo test --test integration retained_status
```

## Tests required

- Status update.
- Transition events.
- Out-of-order resolution.
- SCEN-073 time sync on connect enables commands.
- SCEN-074 no time sync refuses commands while monitoring continues.
- SCEN-076 stale `edge.time` is ignored.
- SCEN-079 a replayed `edge.time` cannot hold a device synchronised.

## Documentation impact

- None.

## Files likely affected

```text
crates/edge-controller/src/pipeline/status.rs
crates/storage/src/repo/device.rs
```
