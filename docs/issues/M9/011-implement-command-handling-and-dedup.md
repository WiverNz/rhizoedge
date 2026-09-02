# Issue M9-011 — Implement command handling with the shared validator and dedup ring

**Milestone:** M9 · **PRD:** [PRD 090](../../prd/090-esp32-rust-firmware.md) · **Depends on:** M9-010, M9-004

## Context

SAFETY-001, -002, -007 on real firmware. Like the simulator, **the only
actuation path calls `validate_water_command`** — there is no second
implementation of the rules.

## Goal

Handle commands with hardware-grade safety.

## Scope

- Dispatch on the three exact `commands/*` topics of protocol §3
- **Every water command through `validate_water_command`**
- 16-entry NVS dedup ring; a repeat republishes the stored result and does **not** actuate
- `(command_id, started_at, requested_ml)` written to NVS **before** actuation
- **If the NVS write fails, abort the dose** and report `failed`
- `delivered_today_ml` in NVS enforcing the device daily cap
- A result published for every command; retained in NVS and retried until the
  **edge** acknowledges it with `command.result.ack` (protocol §5.14), never
  retired on the broker's publish ack
- **Decide, document, and test the pending-result ledger's saturation
  behaviour** — see the dedicated section below. This is a design decision the
  issue requires you to make explicitly, not a detail to settle inside a ring
  implementation.

## Non-goals

- The real pump (M11-001).

## Dependencies

- M9-010
- M9-004

## Implementation notes

The NVS-write-failure rule is worth stating plainly: **if the device cannot
record that it is about to pump, it must not pump.** Otherwise an interrupted
dose becomes undetectable.

Results are ledger data and are retried until acknowledged, unlike telemetry.
An unpublishable result is persisted and republished after the next boot — and
so is a *published but unacknowledged* one, which is the case that makes this a
ledger rather than a slot.

### The pending-result ledger's saturation behaviour — decide this deliberately

Since the post-M6 correction a result is retired only by `command.result.ack`
(protocol §5.14), so the device holds a **bounded durable ledger** of
unacknowledged results, not a single `pending_result`. A device watering while
the edge is down accumulates entries, and every bounded structure eventually
saturates.

**The invariant this issue must satisfy** ([ADR-014](../../adr/014-failure-and-retry-policy.md)
§Device-side pending-result ledger, F-090-17…19):

> If the pending command-result ledger is full, the firmware MUST fail closed,
> and MUST NOT silently discard an unacknowledged watering result in a way that
> can under-count delivered water.

**Do not copy the simulator.** `device-simulator` sets
`PENDING_RESULT_LIMIT = 32` and evicts the oldest entry. That is fine there — a
host has no flash-endurance limit, its autonomous doses carry the same volumes
through a second path as `watering.offline_autonomous` audit events, and its job
is to exercise the protocol rather than to keep a plant alive. **None of those
hold on an ESP32.** The constant is not a specification and the analysis behind
it does not transfer.

**Do not reach for the event buffer's answer either.** M9-017 evicts the oldest
audit event and records a `history.gap`, and that is correct *there*: a gap tells
the edge it is missing a **record**, which is a thing the edge can see and reason
about (SAFETY-020). An evicted `command.result` removes a **quantity the edge's
rolling budget is derived from**, and the edge sees nothing at all — it simply
never learns about water that reached the plant, and the 24-hour cap is under-fed.
Under-counting is the direction that waters again too soon.

Six things to settle and record in the M9 report:

1. **Whether new actuation is refused while the ledger is saturated**, and with
   which refusal reason. Refusing is the obvious fail-closed reading — a device
   that cannot record what it delivered should not deliver more — but say so
   explicitly, and name the reason the edge will see.
2. **How already-delivered water stays attributable and accounted for** once the
   ledger is full, including whether a compacted or aggregated form (a volume
   total the edge can reconcile, say) preserves the accounting when individual
   entries cannot be kept.
3. **What durable fault, gap, or event is emitted**, so saturation is visible to
   the edge and to an operator rather than being an invisible steady state.
4. **Recovery** as acknowledgements free space, and that recovery loses and
   double-counts nothing.
5. **Reboot and NVS persistence at saturation** — the full state survives power
   loss, and a power cycle exactly at the boundary neither drops nor duplicates
   an entry.
6. Whether **any eviction of an unacknowledged result** is adopted at all, and
   if so the explicit argument that it is *safety-equivalent* to retaining the
   entry. Absent that argument, the answer is that the firmware does not evict.

Saturation is reachable on the host with fake adapters, so all of this is
testable in M9-014's conformance layer and the `app/` host tests without a
board.

The device daily cap counts everything — manual, automatic, calibration — unlike
the edge's cap which excludes manual.

## Acceptance criteria

- [x] A valid command actuates and reports `completed`.
- [x] A duplicate `command_id` republishes the stored result and does **not** actuate.
- [x] The ring survives a power cycle.
- [x] NVS is written before actuation.
- [x] **A failed NVS write aborts the dose.**
- [x] The device daily cap is enforced independently of the edge.
- [x] A result is published for every command including rejections.
- [x] A published result is retained until `command.result.ack` names it, and is
      **not** retired by the broker's publish ack.
- [x] The pending-result ledger's capacity and saturation behaviour are decided
      and written down in the M9 report, not left implicit.
- [x] **A saturated ledger fails closed**: no unacknowledged watering result is
      silently discarded in a way that can under-count delivered water.
- [x] Whether new actuation is refused while saturated is stated, with its
      refusal reason, and the tests match the stated choice.
- [x] Saturation emits a durable fault or event; already-delivered water remains
      attributable while it lasts.
- [x] The ledger survives a reboot at saturation with no entry dropped or
      duplicated.
- [x] Acknowledgement frees space and restores normal operation cleanly.
- [x] If eviction of an unacknowledged result is adopted, its safety equivalence
      is argued explicitly in the report; otherwise the firmware does not evict.
- [x] `grep -c validate_water_command` shows exactly one call site.

## Verification

```bash
cd firmware/esp32-node && cargo test command::
cargo test safety_001 safety_002 safety_007
grep -rn 'validate_water_command' firmware/esp32-node/src | wc -l
```

## Tests required

- Each verdict path.
- Dedup across a simulated power cycle.
- **NVS failure aborts the dose.**
- Daily cap enforcement.
- Result retry and persistence, including that only `command.result.ack` retires
  an entry.
- **Ledger saturation**: fill it, assert the stated fail-closed behaviour,
  power-cycle at the boundary, then drain it with acknowledgements and assert
  nothing was lost or double-counted.

## Documentation impact

- [PRD 090](../../prd/090-esp32-rust-firmware.md) Open question 5 — resolve it,
  recording the capacity and the saturation behaviour actually chosen.
- [ADR-014](../../adr/014-failure-and-retry-policy.md) §Device-side
  pending-result ledger — record the decision against its six points.
- The M9 report — including the safety-equivalence argument if any eviction of
  an unacknowledged result is adopted.

## Files likely affected

```text
firmware/esp32-node/src/app/command.rs
firmware/esp32-node/src/app/ledger.rs
firmware/esp32-node/src/safety/mod.rs
```
