# Issue M2-006 — Implement telemetry publication

**Milestone:** M2 · **PRD:** [PRD 020](../../prd/020-device-simulator.md) · **Depends on:** M2-003, M2-005

## Context

Protocol sections 5.1-5.4. Telemetry is never retained — a retained sample
would be served to every new subscriber as though current, which is an actively
dangerous stale reading.

## Goal

Publish the four telemetry kinds on schedule with correct envelopes.

## Scope

- Soil, weight, tank, and pump telemetry per `--sensors`
- Envelope with UUIDv7 `message_id`, fresh `boot_id`, monotonic `sequence`
- QoS 1, **retain false**
- Publication on the configured interval in virtual time
- A 16-sample ring across disconnects; older samples dropped
- Immediate publication on a leak state change

## Non-goals

- Command handling (M2-008).

## Dependencies

- M2-003
- M2-005

## Implementation notes

`--sensors` controls which topics are published at all, so the
missing-sensor lockout paths (SAFETY-004, SAFETY-005) can be exercised by
omission rather than by an injected fault. This is how SCEN-043 works.

The telemetry ring is capped at 16 deliberately: a device is not a ledger for
samples, and unbounded buffering would exhaust RAM on real hardware.

Leak changes bypass the schedule — an hour-late leak notification is useless.

## Acceptance criteria

- [ ] All four telemetry kinds publish with valid envelopes.
- [ ] `retain` is false on every telemetry publish.
- [ ] `sequence` increases monotonically within a boot.
- [ ] `boot_id` changes on restart.
- [ ] `--sensors soil` publishes only soil telemetry.
- [ ] After a 10-minute disconnect, at most 16 buffered samples are sent.
- [ ] A leak change publishes immediately.

## Verification

```bash
mosquitto_sub -h localhost -u rhizo-edge -P "$P" -t 'rhizo/v1/#' -v --retained-only  # no telemetry
cargo test -p device-simulator telemetry::
```

## Tests required

- Envelope validity per kind.
- Sequence monotonicity.
- Ring cap after a disconnect.
- Integration: no retained telemetry.

## Documentation impact

- None.

## Files likely affected

```text
crates/device-simulator/src/telemetry.rs
```
