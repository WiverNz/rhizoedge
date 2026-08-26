# Issue M9-008 — Implement the Wi-Fi connection

**Milestone:** M9 · **PRD:** [PRD 090](../../prd/090-esp32-rust-firmware.md) · **Depends on:** M9-004

## Context

Wi-Fi robustness is the most important non-safety behaviour in this firmware. A
device spends its life on a domestic router that reboots, changes channel, and
occasionally refuses DHCP for a minute.

The device's wall clock is **not** obtained here. There is no SNTP client: time
arrives from the Edge over MQTT, so synchronisation belongs to M9-009, after the
MQTT client exists ([ADR-013](../../adr/013-clock-and-time-semantics.md)).

## Goal

Connect to Wi-Fi reliably and stay useful when it is unavailable.

## Scope

- Wi-Fi with credentials from NVS
- Reconnect with full-jitter backoff, base 2 s cap 300 s, unlimited
- RSSI reported
- **Sampling continues while disconnected**

## Non-goals

- Provisioning the credentials (M9-006).
- MQTT (M9-009).
- Wall-clock synchronisation (M9-009).

## Dependencies

- M9-004

## Implementation notes

An isolated device is still a monitoring device: keep sampling and keep the pump
off. Whether it may water autonomously is decided by its persisted offline policy
(M9-015, M9-016), never by the network layer.

Unlimited retry with a capped backoff is deliberate. A device that gives up after
N attempts is a device that needs a human to power-cycle it, which is the failure
mode this project exists to avoid.

## Acceptance criteria

- [ ] The device connects with NVS credentials.
- [ ] Reconnect uses the documented backoff, with jitter, capped at 300 s.
- [ ] Retry is unlimited — the device never stops trying.
- [ ] Sampling continues while Wi-Fi is down.
- [ ] The pump stays off while disconnected.
- [ ] RSSI is reported in status.

## Verification

```bash
cd firmware/esp32-node && cargo test net::wifi
# with a board: power-cycle the router, observe reconnection
```

## Tests required

- Host tests with a fake network layer.
- Backoff bounds, including the cap and the jitter distribution.
- Sampling continues while disconnected.

## Documentation impact

- None.

## Files likely affected

```text
firmware/esp32-node/src/net/wifi.rs
```
