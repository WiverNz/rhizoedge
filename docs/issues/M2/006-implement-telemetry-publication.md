# Issue M2-006 — Implement telemetry publication

**Milestone:** M2 · **PRD:** [PRD 020](../../prd/020-device-simulator.md) · **Depends on:** M2-003, M2-005

## Context

Protocol sections 5.1-5.4. Telemetry is never retained — a retained sample
would be served to every new subscriber as though current, which is an actively
dangerous stale reading.

## Goal

Publish one typed measurement batch per sampling cycle and separate actuator
state changes with correct envelopes.

## Scope

- One `telemetry.batch` on `rhizo/v1/devices/{id}/telemetry` per sampling cycle,
  containing every `MeasurementSample` taken in that cycle
- A separate `actuator.state` publication on
  `rhizo/v1/devices/{id}/actuator` when actuator state changes
- `--sensors` controls which typed measurement samples are present in the batch
- Envelope with UUIDv7 `message_id`, fresh `boot_id`, monotonic `sequence`
- QoS 1, **retain false**
- Publication on the configured interval in virtual time
- A 16-sample ring across disconnects; older samples dropped
- A leak state change triggers an immediate sampling cycle and therefore one
  complete batch, never a separate measurement topic

## Non-goals

- Command handling (M2-008).

## Dependencies

- M2-003
- M2-005

## Implementation notes

`--sensors` controls which samples appear in the batch, so the
missing-sensor lockout paths (SAFETY-004, SAFETY-005) can be exercised by
omission rather than by an injected fault. This is how SCEN-043 works.

The telemetry ring is capped at 16 deliberately: a device is not a ledger for
samples, and unbounded buffering would exhaust RAM on real hardware.

Leak changes bypass the schedule — an hour-late leak notification is useless.

## Acceptance criteria

- [x] Each sampling cycle produces exactly one valid `telemetry.batch` envelope.
- [x] The batch contains all and only the typed `MeasurementSample` values
      produced by enabled sensors in that cycle.
- [x] Actuator changes publish a separate valid `actuator.state` envelope.
- [x] `retain` is false on every telemetry publish.
- [x] `sequence` increases monotonically within a boot.
- [x] `boot_id` changes on restart.
- [x] `--sensors soil` produces a batch containing soil samples and no samples
      from disabled sensors.
- [x] After a 10-minute disconnect, at most 16 buffered samples are sent.
- [x] A leak change publishes immediately.

## Verification

```bash
mosquitto_sub -h localhost -u rhizo-edge -P "$P" -t 'rhizo/v1/#' -v --retained-only  # no telemetry
cargo test -p device-simulator telemetry::
```

## Tests required

- Literal batch shape and envelope validity.
- Actuator-state shape and publication-on-change.
- Sequence monotonicity.
- Ring cap after a disconnect.
- Integration: no retained telemetry.

## Documentation impact

- None.

## Files likely affected

```text
crates/device-simulator/src/telemetry.rs
```
