# Issue M2-018 — Implement the offline event buffer and replay

**Milestone:** M2 · **PRD:** [PRD 020](../../prd/020-device-simulator.md) · **Depends on:** M2-017

## Context

[ADR-014](../../adr/014-failure-and-retry-policy.md) specifies a bounded ring with
tiered retention and explicit gap reporting. The simulator models the
device-side mechanics before autonomous event production is activated by
M6-019. Tests may inject typed events into the ring; M6 later supplies real
autonomous outcomes through the same buffer API.

## Goal

Buffer history while isolated and replay it idempotently on reconnection.

## Scope

- Bounded ring in the state file with `audit` and `telemetry` tiers
- Audit events never evicted to make room for telemetry
- `event_id` generated once at buffering time; **never** regenerated on replay
- `device_seq` monotonic within `boot_id`
- Eviction emits a `history.gap` event with range, count, and lost tier
- Replay in `device_seq` order, in batches, `complete: true` on the last
- Events retained until the edge acknowledges them

## Non-goals

- Edge-side reconciliation (M3-016, M6-020).
- Offline evaluation or autonomous event decisions (M6-019).

## Dependencies

- M2-017

## Implementation notes

Generating `event_id` at buffering time rather than at publish time is the whole
mechanism. A device that regenerates on replay defeats deduplication and creates
duplicate watering history — SAFETY-016's central failure.

Size the ring so a realistic isolation (hours) overflows telemetry but not audit.
SCEN-104 needs overflow to be reachable in a test without waiting for days of
virtual time.

## Acceptance criteria

- [x] Events buffer while disconnected and replay on reconnect.
- [x] `event_id` is byte-identical across repeated replays.
- [x] Audit events survive a telemetry flood.
- [x] Overflow emits a `history.gap` with correct range and count.
- [x] The final batch sets `complete: true`.
- [x] Unacknowledged events are retained and replayed again.
- [x] Replaying three times produces one logical event per `event_id`.

## Verification

```bash
cargo test -p device-simulator --lib buffer::
cargo test -p device-simulator --test replay
```

Capacities: 64 audit, 256 telemetry, 32 events per replay batch. A realistic
isolation overflows telemetry without touching audit, which is what
`a_long_isolation_overflows_telemetry_but_not_audit` asserts directly.

**Acknowledgement.** `Device::acknowledge_events(through_seq)` discards what the
edge has confirmed; until it is called, events are retained and replayed again,
so an edge that crashes mid-reconciliation loses nothing. Protocol §5.4 requires
the acknowledgement but v1 defines **no acknowledgement topic** — recorded in
mqtt-v1.md §5.4 and left for M6-020, which owns edge-side reconciliation. Until
then the device simply retains and replays, which is the conservative side.

**Negative control**, run and reverted: making `replay_events` regenerate
`event_id` fails six tests across both suites — `safety_016_replaying_three_times_reuses_the_same_event_ids`,
`event_ids_survive_a_restart_unchanged`,
`unacknowledged_events_are_replayed_again_and_acknowledged_ones_are_not`,
`safety_020_a_telemetry_flood_evicts_telemetry_and_reports_a_gap`,
`replaying_three_times_yields_one_logical_event_per_id`, and
`a_run_of_losses_is_one_marker_with_a_stable_id_not_a_flood_of_them`.

## Tests required

- SCEN-100, SCEN-101, SCEN-104.
- Tier retention property test.

## Documentation impact

- `docs/protocol/mqtt-v1.md` §5.4: v1 defines no acknowledgement topic, so the
  retain-until-acknowledged requirement has no mechanism yet. Recorded there and
  assigned to M6-020.

## Files likely affected

```text
crates/device-simulator/src/buffer.rs
```
