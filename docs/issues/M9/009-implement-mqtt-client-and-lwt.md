# Issue M9-009 — Implement the MQTT client, Last Will and Edge time sync

**Milestone:** M9 · **PRD:** [PRD 090](../../prd/090-esp32-rust-firmware.md) · **Depends on:** M9-008

## Context

Protocol section 8's device sequence: LWT before connect, `clean_session =
true`, retained online status, subscribe to own topics only.

The wall clock is established here too, because it arrives over this connection.
Section 5.12 defines `edge.time`: live, never retained, published by the Edge in
response to the device's own retained status and refreshed periodically. SAFETY-002
depends on it — a device that cannot evaluate TTL must refuse every water
command.

## Goal

Speak MQTT identically to the simulator, and hold a wall clock synchronised to
the Edge.

## Scope

- `esp-idf-svc` MQTT client, client id = `device_id`
- **LWT configured before connect**, retained, QoS 1
- `clean_session = true`
- Retained `status: online` on connect
- Subscribe to own `config`, `policy`, `time` and `commands/+` only
- Reconnect with backoff
- Apply `edge.time` only when `edge_time_ms >= last_applied_edge_time_ms`
- Record the monotonic instant of application; derive `clock_synced` from its age
  against `TIME_SYNC_MAX_AGE_SECONDS`
- `clock_synced` reported truthfully in status, meaning *synchronised to the Edge
  and not aged out*
- While `clock_synced` is false, republish status at most every 60 s so the edge
  has a retry trigger

## Non-goals

- Telemetry (M9-010).
- Commands (M9-011).

## Dependencies

- M9-008

## Implementation notes

`clean_session = true` is normative: a persistent session would have the
broker queue commands for an offline device, which SAFETY-002 exists to prevent.

Configuring the LWT after connect silently does nothing, and the omission is
invisible until a device dies in the field. Assert the ordering in a host test.

Report `clock_synced` honestly. A device that claimed synchronisation it did not
have would turn SAFETY-002 into a lie, and there is no way to detect that from
the edge.

The monotonically-non-decreasing acceptance rule is the piece here that is easy
to omit and expensive to miss: MQTT does not guarantee ordering across a
reconnect, so an older `edge.time` can arrive after a newer one, and applying it
would move the clock backwards and make expired commands look valid again.

## Acceptance criteria

- [ ] The device connects with per-device credentials.
- [ ] The LWT is set **before** connect, asserted by a host test.
- [ ] `clean_session` is true.
- [ ] Retained online status is published on connect.
- [ ] It subscribes to `config`, `policy`, `time` and `commands/+` only.
- [ ] It does **not** subscribe to `commands/result`.
- [ ] Killing power produces the LWT within the keepalive window.
- [ ] Applying an `edge.time` makes `clock_synced` true.
- [ ] An `edge.time` older than the last applied one is **ignored**.
- [ ] `clock_synced` becomes false once the last sync exceeds `TIME_SYNC_MAX_AGE_SECONDS`.
- [ ] Withholding `edge.time` leaves `clock_synced` false, reported as such, with
      telemetry unaffected.

## Verification

```bash
cd firmware/esp32-node && cargo test net::mqtt
# with a board: kill power, observe the LWT
```

## Tests required

- LWT ordering.
- Subscription set.
- Host tests with a fake transport.
- `edge.time` monotonicity: stale, duplicate and out-of-order messages never move
  the clock.
- `clock_synced` age expiry on the monotonic clock.

## Documentation impact

- None.

## Files likely affected

```text
firmware/esp32-node/src/net/mqtt.rs
firmware/esp32-node/src/net/time_sync.rs
```
