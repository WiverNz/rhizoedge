# Issue M2-002 — Implement MQTT connection, LWT, and reconnection

**Milestone:** M2 · **PRD:** [PRD 020](../../prd/020-device-simulator.md) · **Depends on:** M2-001

## Context

Protocol section 8 specifies the device connection sequence: LWT configured
before connect, `clean_session = true`, retained online status, and subscriptions
re-established every time.

## Goal

Connect to the broker correctly and stay connected across failures.

## Scope

- `rumqttc` client with `clean_session = true` and client id = `device_id`
- LWT configured **before** connect, retained, QoS 1
- Retained `status: online` published on connect
- Subscribe only to `rhizo/v1/devices/{own_id}/config`,
  `rhizo/v1/devices/{own_id}/policy`, `rhizo/v1/devices/{own_id}/time`, and
  `rhizo/v1/devices/{own_id}/commands/+`
- Reconnect with M0-007 backoff, base 2 s cap 300 s, unlimited
- Clean-disconnect status with `reason: "shutdown"`

## Non-goals

- Telemetry publication (M2-006).
- Command handling (M2-008).

## Dependencies

- M2-001

## Implementation notes

`clean_session = true` is normative (ADR-002): a persistent session would
have the broker queue water commands for an offline device, which is exactly
what SAFETY-002 exists to prevent.

The LWT must be set on the `MqttOptions` before `AsyncClient::new`. Setting it
after connecting silently does nothing, and the omission is invisible until a
device dies in a test.

Subscriptions are re-established on every reconnect, never assumed to survive.

## Acceptance criteria

- [ ] The simulator connects with valid credentials and is refused with invalid ones.
- [ ] Killing it produces the retained LWT within the keepalive window.
- [ ] A clean shutdown publishes `offline` with `reason: "shutdown"`.
- [ ] A fresh subscriber receives the retained status.
- [ ] Stopping and restarting the broker reconnects and **re-subscribes**.
- [ ] Every reconnect restores exactly the four normative edge→device
      subscriptions: `config`, `policy`, `time`, and `commands/+`.
- [ ] The simulator does not subscribe to `commands/result`.

## Verification

```bash
docker compose up -d mosquitto
cargo run -p device-simulator -- --device-id plant-node-01 &
mosquitto_sub -h localhost -u rhizo-edge -P "$P" -t 'rhizo/v1/#' -v --retained-only
```

## Tests required

- Integration: connect, retained status, LWT on kill, reconnect and re-subscribe.

## Documentation impact

- None.

## Files likely affected

```text
crates/device-simulator/src/mqtt.rs
```
