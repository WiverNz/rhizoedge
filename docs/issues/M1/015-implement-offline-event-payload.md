# Issue M1-015 — Implement the offline event and replay payload types

**Milestone:** M1 · **PRD:** [PRD 010](../../prd/010-domain-and-mqtt-protocol.md) · **Depends on:** M1-008

## Context

A device isolated from the edge buffers what happened and replays it on
reconnection ([mqtt-v1.md](../../protocol/mqtt-v1.md) §5.4). The stability of
`event_id` across replays is what makes reconciliation idempotent (SAFETY-016).

## Goal

Implement the `device.events` payload types for buffered history replay.

## Scope

- `DeviceEventBatch` with `replay`, `complete`, and `events[]`
- `BufferedEvent` with `event_id`, `device_seq`, `tier`, `kind`, `monotonic_ms`, optional `device_time_ms`, `detail`
- `EventTier`: `Audit` | `Telemetry`
- Event kinds including `watering.offline_autonomous`, `offline.refused`, `history.gap`, `policy.activated`, `lockout.set`, `lockout.cleared`
- `detail` as a typed enum per kind, not an opaque map

## Non-goals

- Buffering or replay logic (M2-018, M9-017).
- Edge-side ingestion (M3-016).

## Dependencies

- M1-008

## Implementation notes

`monotonic_ms` is always present and always meaningful; `device_time_ms` is
`Option` and is `None` whenever the clock was unsynced. Making the wall clock the
optional one — the reverse of the telemetry envelope — reflects what an isolated
device actually knows.

Keep `detail` typed. A `serde_json::Value` would be the easy choice and would
push every consumer into runtime string matching, which is precisely what
[ADR-017](../../adr/017-extensible-measurement-model.md) rejects for
measurements; the same reasoning applies here.

`complete` is what tells the edge it may release the plant from `Uncertain`
(SAFETY-016). Getting it wrong in either direction is a safety bug: never set,
and the plant never waters again; set too early, and the edge doses on top of an
autonomous dose it has not yet read.

## Acceptance criteria

- [ ] All event kinds round-trip with their typed detail.
- [ ] `monotonic_ms` is required; `device_time_ms` is `Option` and `null` decodes to `None`.
- [ ] `history.gap` detail carries `from_seq`, `to_seq`, `lost_count`, `lost_tier`.
- [ ] An unrecognised event kind decodes to a conservative `Unknown` variant rather than failing.
- [ ] `complete` defaults to `false` when absent.
- [ ] `detail` is a typed enum, not a free-form map.

## Verification

```bash
cargo test -p rhizo-mqtt-contract events::
```

## Tests required

- Round trip per event kind.
- Optionality of `device_time_ms`.
- Unknown-kind tolerance.
- Gap detail contents.

## Documentation impact

- None.

## Files likely affected

```text
crates/mqtt-contract/src/payload/events.rs
```
