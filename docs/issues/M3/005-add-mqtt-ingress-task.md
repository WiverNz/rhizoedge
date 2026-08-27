# Issue M3-005 — Implement the MQTT ingress task with reconnection and re-subscription

**Milestone:** M3 · **PRD:** [PRD 030](../../prd/030-edge-ingestion-and-storage.md) · **Depends on:** M3-001

## Context

Protocol section 8 and failure-model 1.2 both require that subscriptions be
re-established on **every** reconnect. A reconnect without re-subscribe is
silent data loss and is easy to miss, because the connection metric looks
healthy.

## Goal

Consume MQTT reliably across broker restarts.

## Scope

- `rumqttc` event loop with the edge client id and credentials
- Subscribe through `Topic::EDGE_SUBSCRIPTIONS` to the five device→Edge forms:
  `telemetry`, `actuator`, `events`, `status`, and `commands/result`
- **Re-subscribe on every reconnect**, never assumed to survive
- Reconnect with M0-007 backoff, base 1 s cap 60 s, unlimited
- Connection state tracked as Disconnected/Connecting/Connected/Subscribed
- Messages handed to the pipeline via a bounded channel

## Non-goals

- Decoding (M3-006).
- Persistence (M3-008).

## Dependencies

- M3-001

## Implementation notes

`Subscribed` is a distinct state from `Connected` precisely because of the
re-subscribe requirement; readiness (M4-007) depends on `Subscribed`, not on
`Connected`.

The channel to the pipeline is bounded. If the pipeline falls behind, applying
backpressure to the event loop is correct — the broker will redeliver QoS 1
messages. An unbounded channel would grow until the process dies.

The edge must start successfully with the broker down (failure-model 1.1).

The narrow filters are equivalent to the normative Edge subtree for current
v1 device-originated traffic while excluding every Edge→device form. They
resolve wording that predated the final M1/M2 direction table.

## Acceptance criteria

- [x] The edge starts and stays up with the broker unavailable.
- [x] It connects when the broker appears.
- [x] Restarting the broker reconnects **and re-subscribes** — asserted by receiving messages afterwards.
- [x] Backoff delays increase and stay within bounds.
- [x] `mqtt_connection_state` and `mqtt_reconnects_total` reflect reality.
- [x] A slow pipeline applies backpressure rather than growing a queue.

## Verification

```bash
cargo test --test integration mqtt_ingress
docker compose restart mosquitto  # then confirm telemetry resumes
```

## Tests required

- Start with no broker.
- Reconnect and re-subscribe (SCEN-012).
- Backpressure under a stalled consumer.

## Documentation impact

- None.

## Files likely affected

```text
crates/edge-controller/src/mqtt/ingress.rs
```
