# Issue M9-009 — Implement the MQTT client with Last Will

**Milestone:** M9 · **PRD:** [PRD 090](../../prd/090-esp32-rust-firmware.md) · **Depends on:** M9-008

## Context

Protocol section 8's device sequence: LWT before connect, `clean_session =
true`, retained online status, subscribe to own topics only.

## Goal

Speak MQTT identically to the simulator.

## Scope

- `esp-idf-svc` MQTT client, client id = `device_id`
- **LWT configured before connect**, retained, QoS 1
- `clean_session = true`
- Retained `status: online` on connect
- Subscribe to own `config` and `commands/+` only
- Reconnect with backoff

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

## Acceptance criteria

- [ ] The device connects with per-device credentials.
- [ ] The LWT is set **before** connect, asserted by a host test.
- [ ] `clean_session` is true.
- [ ] Retained online status is published on connect.
- [ ] It subscribes to `config` and `commands/+` only.
- [ ] It does **not** subscribe to `commands/result`.
- [ ] Killing power produces the LWT within the keepalive window.

## Verification

```bash
cd firmware/esp32-node && cargo test net::mqtt
# with a board: kill power, observe the LWT
```

## Tests required

- LWT ordering.
- Subscription set.
- Host tests with a fake transport.

## Documentation impact

- None.

## Files likely affected

```text
firmware/esp32-node/src/net/mqtt.rs
```
